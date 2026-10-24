use crate::{
    config::{Config, ConfigStore},
    hardware::{self, Hardware, InferencePlan},
    imaging,
    table::{self, Supplement},
};
use anyhow::{Context, Result, bail, ensure};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    process::{Child, Command},
    sync::Mutex as AsyncMutex,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: u64,
    pub text: String,
    pub supplements: Vec<Supplement>,
    pub warnings: Vec<String>,
    pub task: String,
    pub source: String,
    pub width: u32,
    pub height: u32,
    pub elapsed_ms: u64,
    pub created_at: u64,
    #[serde(default)]
    pub pages: u32,
    #[serde(default)]
    pub generation: Option<GenerationStats>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct GenerationStats {
    pub tokens: u64,
    pub milliseconds: f64,
    #[serde(default)]
    pub includes_prefill: bool,
}
impl GenerationStats {
    fn from_response(response: &Value) -> Option<Self> {
        let timing = &response["timings"];
        let mut stats = if response["stats"].is_object() {
            let page = &response["stats"];
            Self {
                tokens: page["generated_tokens"].as_u64()?,
                milliseconds: page["layout_ms"].as_f64()? + page["extract_ms"].as_f64()?,
                includes_prefill: true,
            }
        } else {
            Self {
                tokens: timing["predicted_n"]
                    .as_u64()
                    .or_else(|| timing["generated_tokens"].as_u64())?,
                milliseconds: timing["predicted_ms"]
                    .as_f64()
                    .or_else(|| timing["decode_ms"].as_f64())?,
                includes_prefill: false,
            }
        };
        // llama.cpp's decode timer excludes the first token (sampled during prefill).
        // Recover its timed token count so single and accumulated speeds match the backend.
        if let Some(rate) = timing["predicted_per_second"]
            .as_f64()
            .filter(|rate| rate.is_finite() && *rate >= 0.0)
        {
            stats.tokens = (rate * stats.milliseconds / 1000.0).round() as u64;
        }
        (stats.tokens > 0 && stats.milliseconds.is_finite() && stats.milliseconds > 0.0)
            .then_some(stats)
    }
    fn merge(self, other: Self) -> Self {
        Self {
            tokens: self.tokens + other.tokens,
            milliseconds: self.milliseconds + other.milliseconds,
            includes_prefill: self.includes_prefill || other.includes_prefill,
        }
    }
}
impl Record {
    pub fn combined(&self) -> String {
        table::combined(&self.text, &self.supplements)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub revision: u64,
    pub phase: String,
    pub message: String,
    pub ready: bool,
    pub hotkey_error: Option<String>,
    pub loaded_model: Option<String>,
    pub hardware: Option<Hardware>,
    pub inference: Option<InferencePlan>,
    pub native_languages: Vec<crate::models::NativeLanguage>,
    pub config: Config,
    pub result: Option<Record>,
    pub history: VecDeque<Record>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineEvent {
    State {
        #[serde(flatten)]
        snapshot: Box<Snapshot>,
    },
    Progress {
        revision: u64,
        id: u64,
        text: String,
        elapsed_ms: u64,
    },
}

struct ActiveJob {
    id: u64,
    cancel: CancellationToken,
    previous: Option<Record>,
    prefix: String,
    base_pages: u32,
    base_elapsed: u64,
    append: bool,
    selecting: bool,
}
struct ServerProcess {
    child: Child,
    #[cfg(windows)]
    _job: crate::native::ProcessJob,
}

pub struct Engine {
    pub store: Mutex<ConfigStore>,
    state: Mutex<Snapshot>,
    active: Mutex<Option<ActiveJob>>,
    server: AsyncMutex<Option<ServerProcess>>,
    #[cfg(windows)]
    classic: Mutex<Option<Arc<crate::classic::Classic>>>,
    lifecycle: AsyncMutex<()>,
    pub workflow: AsyncMutex<()>,
    request_id: AtomicU64,
    emit: Arc<dyn Fn(EngineEvent) + Send + Sync>,
    client: reqwest::Client,
    log_path: PathBuf,
}

impl Engine {
    pub fn new(
        store: ConfigStore,
        emit: impl Fn(EngineEvent) + Send + Sync + 'static,
    ) -> Result<Arc<Self>> {
        let log_root = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("BlinkOCR/logs");
        fs::create_dir_all(&log_root)?;
        let state = Snapshot {
            revision: 0,
            phase: "loading".into(),
            message: "正在准备识别引擎".into(),
            ready: false,
            hotkey_error: None,
            loaded_model: None,
            hardware: None,
            inference: None,
            native_languages: Vec::new(),
            config: store.config.clone(),
            result: None,
            history: VecDeque::new(),
        };
        Ok(Arc::new(Self {
            store: Mutex::new(store),
            state: Mutex::new(state),
            active: Mutex::new(None),
            server: AsyncMutex::new(None),
            #[cfg(windows)]
            classic: Mutex::new(None),
            lifecycle: AsyncMutex::new(()),
            workflow: AsyncMutex::new(()),
            request_id: AtomicU64::new(0),
            emit: Arc::new(emit),
            client: reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(3))
                .build()?,
            log_path: log_root.join("blinkocr.log"),
        }))
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().clone()
    }
    fn update(&self, edit: impl FnOnce(&mut Snapshot)) {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            edit(&mut state);
            state.revision += 1;
            state.clone()
        };
        (self.emit)(EngineEvent::State {
            snapshot: Box::new(snapshot),
        });
    }
    fn log(&self, text: &str) {
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = writeln!(file, "{} {text}", now());
        }
    }
    pub fn notice(&self, message: impl Into<String>) {
        self.update(|state| state.message = message.into());
    }
    pub fn hotkey_error(&self, error: Option<String>) {
        self.update(|state| state.hotkey_error = error);
    }
    pub fn busy(&self) -> bool {
        self.active.lock().unwrap().is_some()
    }
    fn cancel_active(&self) {
        if let Some(job) = self.active.lock().unwrap().as_ref() {
            job.cancel.cancel();
        }
    }
    pub fn cancel(&self) {
        self.request_id.fetch_add(1, Ordering::SeqCst);
        self.cancel_active();
    }
    pub fn request(&self) -> u64 {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.cancel_active();
        id
    }
    pub fn current_request(&self, id: u64) -> bool {
        self.request_id.load(Ordering::SeqCst) == id
    }

    pub fn begin(&self, id: u64, source: String, phase: &str) -> Result<CancellationToken> {
        ensure!(
            self.snapshot().ready,
            "识别引擎尚未就绪，请先配置并加载模型"
        );
        let snapshot = self.snapshot();
        let previous = snapshot.result;
        let append = snapshot.config.ocr.continuous_recognition
            && previous
                .as_ref()
                .is_some_and(|record| !record.combined().is_empty());
        let prefix = if append {
            previous.as_ref().unwrap().combined()
        } else {
            String::new()
        };
        let base_pages = if append {
            previous.as_ref().unwrap().pages
        } else {
            0
        };
        let base_elapsed = if append {
            previous.as_ref().unwrap().elapsed_ms
        } else {
            0
        };
        let created_at = if append {
            previous.as_ref().unwrap().created_at
        } else {
            now()
        };
        let token = CancellationToken::new();
        *self.active.lock().unwrap() = Some(ActiveJob {
            id,
            cancel: token.clone(),
            previous,
            prefix: prefix.clone(),
            base_pages,
            base_elapsed,
            append,
            selecting: phase == "selecting",
        });
        let task = self.snapshot().config.task().to_string();
        self.update(|state| {
            state.phase = phase.into();
            state.message = if phase == "selecting" {
                "拖动框选，Esc 取消"
            } else {
                "正在读取图片"
            }
            .into();
            state.result = Some(Record {
                id,
                text: prefix,
                supplements: Vec::new(),
                warnings: Vec::new(),
                task,
                source,
                width: 0,
                height: 0,
                elapsed_ms: base_elapsed,
                created_at,
                pages: base_pages,
                generation: None,
            });
        });
        Ok(token)
    }

    pub fn fail(&self, id: u64, error: &str, cancelled: bool) {
        let job = {
            let mut active = self.active.lock().unwrap();
            if active.as_ref().is_some_and(|job| job.id == id) {
                active.take()
            } else {
                None
            }
        };
        let Some(job) = job else {
            return;
        };
        self.log(error);
        self.update(|state| {
            state.phase = if cancelled { "cancelled" } else { "error" }.into();
            state.message = if cancelled {
                "已取消识别".into()
            } else {
                error.into()
            };
            if (job.append || job.selecting) && job.previous.is_some() {
                state.result = job.previous;
                return;
            }
            if let Some(result) = state.result.as_mut()
                && !result.text.is_empty()
            {
                result.warnings.push(if cancelled {
                    "识别已取消，保留已生成内容".into()
                } else {
                    error.into()
                });
            }
        });
    }

    pub fn update_result_text(&self, id: u64, text: String) -> Result<Snapshot> {
        ensure!(!self.busy(), "请先结束当前识别");
        let state = self.snapshot();
        let mut record = state
            .result
            .filter(|record| record.id == id)
            .or_else(|| state.history.into_iter().find(|record| record.id == id))
            .context("识别结果已不存在")?;
        record.text = text;
        self.update(|state| {
            if let Some(item) = state.history.iter_mut().find(|item| item.id == id) {
                *item = record.clone();
            }
            state.result = Some(record);
        });
        Ok(self.snapshot())
    }

    pub fn clear_result(&self) -> Result<Snapshot> {
        ensure!(!self.busy(), "请先结束当前识别");
        self.update(|state| {
            state.result = None;
            if state.ready {
                state.phase = "idle".into();
                state.message = "就绪".into();
            }
        });
        Ok(self.snapshot())
    }

    pub async fn start_server(&self) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        self.stop_process().await;
        #[cfg(windows)]
        self.classic.lock().unwrap().take();
        self.update(|state| {
            state.ready = false;
            state.loaded_model = None;
            state.inference = None;
            state.phase = "loading".into();
            state.message = "正在加载本地模型".into();
        });
        let (config, bin, model, mmproj) = {
            let store = self.store.lock().unwrap();
            (
                store.config.clone(),
                store.backend_path(),
                store.resolve(&store.config.model.model_path),
                store.resolve(&store.config.model.mmproj_path),
            )
        };
        let outcome = async {
            #[cfg(windows)]
            {
                let languages = tokio::task::spawn_blocking(crate::classic::languages).await?;
                self.update(|state| state.native_languages = languages);
                if crate::models::is_classic(&config.model.profile) {
                    let mut host = hardware::detect(None).await;
                    self.update(|state| state.hardware = Some(host.clone()));
                    let native_config = config.clone();
                    let native_host = host.clone();
                    let (classic, plan, name) = tokio::task::spawn_blocking(move || {
                        crate::classic::Classic::load(&native_config, &model, &mmproj, &native_host)
                    })
                    .await??;
                    *self.classic.lock().unwrap() = Some(Arc::new(classic));
                    if plan.mode == "gpu" {
                        for gpu in &mut host.gpus {
                            gpu.usable = true;
                        }
                    }
                    self.update(|state| {
                        state.hardware = Some(host);
                        state.inference = Some(plan);
                        state.loaded_model = Some(name);
                    });
                    return Ok(());
                }
            }
            ensure!(
                !config.model.model_path.is_empty() && model.is_file(),
                "请选择 GGUF 主模型"
            );
            ensure!(
                config.model.profile == "teleocr"
                    || (!config.model.mmproj_path.is_empty() && mmproj.is_file()),
                "请选择匹配的视觉投影文件"
            );
            ensure!(
                bin.is_file(),
                "未找到推理程序，请在设置中选择对应的本地程序"
            );
            let weights = fs::metadata(&model)?.len();
            let projection = if config.model.profile == "teleocr" {
                0
            } else {
                fs::metadata(&mmproj)?.len()
            };
            let host =
                hardware::detect((config.model.profile != "teleocr").then_some(bin.as_path()))
                    .await;
            self.update(|state| state.hardware = Some(host.clone()));
            let mut plan = hardware::choose(&config, &host, weights, projection)?;
            self.update(|state| {
                state.inference = Some(plan.clone());
                state.message = format!("正在加载模型 · {}", plan.reason);
            });
            let log = self.log_path.with_file_name("llama-server.log");
            let mut offset = fs::metadata(&log).map(|info| info.len()).unwrap_or(0);
            if let Err(error) = self
                .launch(&config, bin.clone(), model.clone(), mmproj.clone(), &plan)
                .await
            {
                let attempt = read_log(&log, offset);
                self.stop_process().await;
                if plan.mode == "cpu" || !hardware::gpu_failure(&attempt) {
                    return Err(error);
                }
                self.log(&format!("GPU 加载失败，尝试 CPU：{error:#}"));
                let host = hardware::detect(None).await;
                plan = hardware::cpu_fallback(&config, &host, weights, projection)?;
                self.update(|state| {
                    state.inference = Some(plan.clone());
                    state.message = plan.reason.clone();
                });
                offset = fs::metadata(&log).map(|info| info.len()).unwrap_or(0);
                self.launch(&config, bin, model, mmproj, &plan).await?;
            }
            hardware::actual_mode(&mut plan, &read_log(&log, offset));
            self.update(|state| state.inference = Some(plan));
            Ok(())
        }
        .await;
        match &outcome {
            Ok(()) => self.update(|state| {
                state.ready = true;
                state.phase = "idle".into();
                state.message = "就绪".into();
                if !crate::models::is_classic(&config.model.profile) {
                    state.loaded_model = Some(config.model.model_path.clone());
                }
            }),
            Err(error) => {
                self.stop_process().await;
                self.log(&format!("启动失败：{error:#}"));
                self.update(|state| {
                    state.ready = false;
                    state.phase = "setup".into();
                    state.message = error.to_string();
                });
            }
        }
        outcome
    }

    async fn launch(
        &self,
        config: &Config,
        bin: PathBuf,
        model: PathBuf,
        mmproj: PathBuf,
        plan: &InferencePlan,
    ) -> Result<()> {
        ensure!(
            !config.model.model_path.is_empty() && model.is_file(),
            "请选择 GGUF 主模型"
        );
        ensure!(
            config.model.profile == "teleocr"
                || (!config.model.mmproj_path.is_empty() && mmproj.is_file()),
            "请选择匹配的视觉投影文件"
        );
        ensure!(
            bin.is_file(),
            "未找到推理程序，请在设置中选择对应的本地程序"
        );
        if tokio::net::TcpStream::connect((config.server.host.as_str(), config.server.port))
            .await
            .is_ok()
        {
            bail!("端口 {} 已被占用，请在设置中更换端口", config.server.port);
        }
        let started = Instant::now();
        let log_file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path.with_file_name("llama-server.log"))?;
        let mut command = Command::new(&bin);
        if config.model.profile == "teleocr" {
            command.arg("serve").arg("-m").arg(model).args([
                "--host",
                &config.server.host,
                "--port",
                &config.server.port.to_string(),
                "--batch",
                "1",
                "--threads",
                &config.model.threads.to_string(),
                "--cpu",
            ]);
        } else {
            command
                .args(["-m"])
                .arg(model)
                .arg("--mmproj")
                .arg(mmproj)
                .args(plan.arguments())
                .args([
                    "-c",
                    &config.model.context_size.to_string(),
                    "-t",
                    &config.model.threads.to_string(),
                    "--batch-size",
                    &4096.min(config.model.context_size).to_string(),
                    "--ubatch-size",
                    &2048.min(config.model.context_size).to_string(),
                    "--host",
                    &config.server.host,
                    "--port",
                    &config.server.port.to_string(),
                    "--jinja",
                    "--no-webui",
                    "--parallel",
                    "1",
                    "--cache-ram",
                    "0",
                    "--log-verbosity",
                    "4",
                ]);
            if config.model.image_min_tokens > 0 {
                command.args([
                    "--image-min-tokens",
                    &config.model.image_min_tokens.to_string(),
                ]);
            }
            if ["deepseek-ocr2", "unlimited-ocr"].contains(&config.model.profile.as_str()) {
                command.args([
                    "--chat-template",
                    "deepseek-ocr",
                    "--no-jinja",
                    "--flash-attn",
                    "off",
                    "--no-warmup",
                ]);
            }
            command.args([
                "-n",
                "-1",
                "--alias",
                if config.model.profile == "other" {
                    "local"
                } else {
                    &config.model.profile
                },
            ]);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file.try_clone()?))
            .stderr(Stdio::from(log_file))
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let child = command.spawn().context("启动本地推理程序失败")?;
        self.log(&format!(
            "创建后端 pid={:?} elapsed={:?}",
            child.id(),
            started.elapsed()
        ));
        #[cfg(windows)]
        let job = crate::native::ProcessJob::attach(child.id().context("后端已退出")?)?;
        *self.server.lock().await = Some(ServerProcess {
            child,
            #[cfg(windows)]
            _job: job,
        });
        loop {
            {
                let mut server = self.server.lock().await;
                let process = server.as_mut().context("模型加载已取消")?;
                if let Some(status) = process.child.try_wait()? {
                    bail!("本地推理程序已退出（{status}），请检查模型和运行时");
                }
            }
            if let Ok(response) = self
                .client
                .get(format!("{}/health", config.server.base_url()))
                .timeout(Duration::from_secs(2))
                .send()
                .await
                && response.status().is_success()
            {
                self.log(&format!("模型就绪 elapsed={:?}", started.elapsed()));
                return Ok(());
            }
            ensure!(
                started.elapsed() < Duration::from_secs(config.server.startup_timeout),
                "模型加载超时，请检查模型文件和显存容量"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn stop_process(&self) {
        if let Some(mut server) = self.server.lock().await.take() {
            let _ = server.child.kill().await;
            let _ = server.child.wait().await;
        }
    }
    pub async fn refresh_backend_status(&self) {
        if !self.snapshot().ready {
            return;
        }
        let exited = {
            let mut server = self.server.lock().await;
            server
                .as_mut()
                .and_then(|process| process.child.try_wait().ok().flatten())
        };
        if let Some(status) = exited {
            let busy = self.busy();
            self.log(&format!("后端退出：{status}"));
            self.update(|state| {
                state.ready = false;
                state.loaded_model = None;
                state.message = "本地推理程序已退出，请重新加载模型".into();
                if !busy {
                    state.phase = "error".into();
                }
            });
        }
    }
    pub async fn refresh_hardware(&self) -> Snapshot {
        let (profile, bin) = {
            let store = self.store.lock().unwrap();
            (store.config.model.profile.clone(), store.backend_path())
        };
        let host = hardware::detect((profile != "teleocr").then_some(bin.as_path())).await;
        self.update(|state| state.hardware = Some(host));
        self.snapshot()
    }
    pub async fn shutdown(&self) {
        self.cancel();
        self.stop_process().await;
    }

    pub fn save_config(&self, mut config: Config) -> Result<bool> {
        ensure!(!self.busy(), "请先结束当前识别再保存设置");
        config.normalize();
        let mut store = self.store.lock().unwrap();
        let restart = store.config.needs_restart(&config) || !self.snapshot().ready;
        store.save(config.clone())?;
        self.update(|state| {
            state.config = config;
            state.message = "设置已保存".into();
        });
        if restart {
            self.prepare_reload();
        }
        Ok(restart)
    }

    pub fn prepare_reload(&self) {
        self.update(|state| {
            state.ready = false;
            state.loaded_model = None;
            state.phase = "loading".into();
            state.message = "正在加载本地模型".into();
        });
    }

    pub async fn recognize(
        &self,
        id: u64,
        image: DynamicImage,
        token: CancellationToken,
    ) -> Result<()> {
        let started = Instant::now();
        let config = self.snapshot().config;
        let (prefix, base_pages, base_elapsed, previous_warnings, base_generation) = {
            let mut active = self.active.lock().unwrap();
            let job = active
                .as_mut()
                .filter(|job| job.id == id)
                .context("识别任务已结束")?;
            job.selecting = false;
            (
                job.prefix.clone(),
                job.base_pages,
                job.base_elapsed,
                if job.append {
                    job.previous
                        .as_ref()
                        .map(|record| record.warnings.clone())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                },
                if job.append {
                    job.previous.as_ref().and_then(|record| record.generation)
                } else {
                    Some(GenerationStats::default())
                },
            )
        };
        ensure!(!token.is_cancelled(), "识别已取消");
        let max_size = config.ocr.max_image_dimension;
        let image =
            tokio::task::spawn_blocking(move || imaging::prepare(image, max_size)).await??;
        let (width, height) = (image.width(), image.height());
        self.update(|state| {
            state.phase = "recognizing".into();
            state.message = "正在识别选区内容".into();
            if let Some(result) = state.result.as_mut() {
                result.width = width;
                result.height = height;
            }
        });
        #[cfg(windows)]
        let native_text = {
            let native = self.classic.lock().unwrap().clone();
            if let Some(classic) = native {
                let native_image = image.clone();
                let language = config.ocr.native_language.clone();
                let text =
                    tokio::task::spawn_blocking(move || classic.recognize(native_image, &language))
                        .await??;
                ensure!(!token.is_cancelled(), "识别已取消");
                Some(text)
            } else {
                None
            }
        };
        #[cfg(not(windows))]
        let native_text: Option<String> = None;
        let encoded = if native_text.is_some() {
            String::new()
        } else {
            imaging::data_url(&image)?
        };
        let mut text = String::new();
        let mut finish = String::new();
        let mut warnings = previous_warnings;
        let mut generation = None;
        if let Some(output) = native_text {
            text = output;
            finish = "stop".into();
        } else if config.model.profile == "teleocr" {
            let request = async {
                let response: Value = self.client.post(format!("{}/v1/ocr", config.server.base_url()))
                    .json(&json!({"image":encoded,"task":config.api_task(),
                        "max_tokens":if config.ocr.max_tokens<0 {config.model.context_size as i64} else {config.ocr.max_tokens as i64}}))
                    .timeout(Duration::from_secs(900)).send().await?.error_for_status()?.json().await?;
                Ok::<Value, anyhow::Error>(response)
            };
            let response = tokio::select! {
                _ = token.cancelled() => {
                    self.stop_process().await;
                    self.prepare_reload();
                    bail!("识别已取消");
                },
                response = request => response?,
            };
            generation = GenerationStats::from_response(&response);
            text = response["markdown"]
                .as_str()
                .or(response["content"].as_str())
                .or(response["text"].as_str())
                .unwrap_or_default()
                .into();
            finish = if response["truncated"].as_bool() == Some(true) {
                "length"
            } else {
                "stop"
            }
            .into();
        } else {
            let body = request_body(
                &config,
                &encoded,
                config.effective_prompt(),
                config.ocr.max_tokens,
                true,
            );
            let response = tokio::select! {
            _ = token.cancelled() => bail!("识别已取消"),
            response = self.client.post(format!("{}/v1/chat/completions", config.server.base_url())).json(&body).timeout(Duration::from_secs(900)).send() => response?,
        }.error_for_status()?;
            let mut stream = response.bytes_stream().eventsource();

            let mut last_emit = Instant::now();

            loop {
                let item = tokio::select! {
                    _ = token.cancelled() => { self.partial(id, &text, started.elapsed()); bail!("识别已取消"); },
                    item = stream.next() => item,
                };
                let Some(item) = item else {
                    break;
                };
                let event = match item {
                    Ok(event) => event,
                    Err(error) => {
                        if text.is_empty() {
                            bail!("识别连接中断：{error}");
                        }
                        warnings.push("识别连接中断，已保留生成内容".into());
                        break;
                    }
                };
                if event.data == "[DONE]" {
                    break;
                }
                let chunk: Value = serde_json::from_str(&event.data).context("识别响应格式有误")?;
                if chunk["timings"].is_object() {
                    generation = GenerationStats::from_response(&chunk);
                }
                if let Some(content) = chunk["choices"][0]["delta"]["content"].as_str() {
                    text.push_str(content);
                }
                if let Some(reason) = chunk["choices"][0]["finish_reason"].as_str() {
                    finish = reason.into();
                }
                if last_emit.elapsed() >= Duration::from_millis(80) {
                    self.partial(id, &text, started.elapsed());
                    last_emit = Instant::now();
                }
            }
        }
        let text = clean(&text);
        ensure!(!text.is_empty(), "未识别到文字，请调整选区后重试");
        if finish == "length" {
            warnings.push(
                if config.ocr.max_tokens > 0 {
                    "达到输出长度上限，可增加输出长度后重试"
                } else {
                    "达到上下文长度，可增加上下文或缩小选区"
                }
                .into(),
            );
        } else if finish != "stop" {
            warnings.push("识别未完整结束，请核对原图".into());
        }
        let mut supplements = Vec::new();
        if config.postprocess_enabled(&text) && image.height() >= 220 {
            self.partial(id, &text, started.elapsed());
            self.update(|state| {
                state.phase = "postprocessing".into();
                state.message = "正在补充识别表格内容".into();
            });
            let bounds = imaging::band_bounds(&image);
            let overlap = bounds.windows(2).any(|pair| pair[0].1 > pair[1].0);
            let mut outputs = Vec::new();
            for (label, &(top, bottom)) in ["上部", "中部", "下部"].into_iter().zip(&bounds) {
                let band = image.crop_imm(0, top, width, bottom - top);
                let output = self.recognize_band(&config, &band, &token).await;
                ensure!(!token.is_cancelled(), "识别已取消");
                let (band_text, warning, stats) = match output {
                    Ok(result) => result,
                    Err(error) => {
                        self.log(&format!("{label}补充失败：{error:#}"));
                        (String::new(), "请求失败".into(), None)
                    }
                };
                generation = generation.zip(stats).map(|(total, band)| total.merge(band));
                if !warning.is_empty() {
                    warnings.push(format!("{label}补充识别{warning}"));
                }
                outputs.push(band_text);
            }
            let union = if overlap {
                String::new()
            } else {
                table::normalize(&outputs.join("\n"))
            };
            let mut base = table::normalize(&text);
            for (source, output) in ["上部", "中部", "下部"].into_iter().zip(outputs) {
                let mut found = table::uncovered(&output, &base, &union, finish == "length");
                for item in &mut found {
                    item.source = source.into();
                    base.push('\n');
                    base.push_str(&table::normalize(&item.text));
                }
                supplements.extend(found);
            }
            if overlap {
                warnings.push("请核对表格中的重复行数量".into());
            }
        }
        ensure!(!token.is_cancelled(), "识别已取消");
        let mut seen = std::collections::HashSet::new();
        warnings.retain(|warning| seen.insert(warning.clone()));
        self.update(|state| {
            state.phase = "completed".into();
            state.message = "识别完成".into();
            if let Some(record) = state.result.as_mut().filter(|record| record.id == id) {
                record.text = append_text(&prefix, &text);
                record.warnings = warnings;
                record.supplements = supplements;
                record.elapsed_ms = base_elapsed + started.elapsed().as_millis() as u64;
                record.pages = base_pages + 1;
                record.generation = base_generation
                    .zip(generation)
                    .map(|(base, current)| base.merge(current));
                state.history.push_front(record.clone());
                state.history.truncate(20);
            }
        });
        self.active.lock().unwrap().take();
        Ok(())
    }

    fn partial(&self, id: u64, text: &str, elapsed: Duration) {
        let context = {
            let active = self.active.lock().unwrap();
            active
                .as_ref()
                .filter(|job| job.id == id)
                .map(|job| (job.prefix.clone(), job.base_elapsed))
        };
        let Some((prefix, base_elapsed)) = context else {
            return;
        };
        let event = {
            let mut state = self.state.lock().unwrap();
            let Some(record) = state.result.as_mut().filter(|record| record.id == id) else {
                return;
            };
            record.text = append_text(&prefix, text);
            record.elapsed_ms = base_elapsed + elapsed.as_millis() as u64;
            let (text, elapsed_ms) = (record.text.clone(), record.elapsed_ms);
            state.revision += 1;
            EngineEvent::Progress {
                revision: state.revision,
                id,
                text,
                elapsed_ms,
            }
        };
        (self.emit)(event);
    }

    async fn recognize_band(
        &self,
        config: &Config,
        image: &DynamicImage,
        token: &CancellationToken,
    ) -> Result<(String, String, Option<GenerationStats>)> {
        let max_tokens = if config.ocr.max_tokens > 0 {
            config.ocr.max_tokens.min(2048)
        } else {
            -1
        };
        let body = request_body(
            config,
            &imaging::data_url(image)?,
            crate::profiles::selector("text"),
            max_tokens,
            false,
        );
        let request = async {
            let response: Value = self
                .client
                .post(format!("{}/v1/chat/completions", config.server.base_url()))
                .json(&body)
                .timeout(Duration::from_secs(180))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let text = clean(
                response["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or_default(),
            );
            let warning = match response["choices"][0]["finish_reason"].as_str() {
                Some("length") => "达到输出上限",
                Some("stop") if text.is_empty() => "未返回文字",
                Some("stop") => "",
                _ => "未完整结束",
            };
            Ok((
                text,
                warning.into(),
                GenerationStats::from_response(&response),
            ))
        };
        tokio::select! { _ = token.cancelled() => bail!("识别已取消"), result = request => result }
    }
}

fn append_text(prefix: &str, text: &str) -> String {
    if prefix.is_empty() {
        text.into()
    } else if text.is_empty() {
        prefix.into()
    } else {
        format!("{prefix}\n\n{text}")
    }
}

pub fn request_body(
    config: &Config,
    image: &str,
    prompt: &str,
    max_tokens: i32,
    stream: bool,
) -> Value {
    let mut body = json!({"model": if config.model.profile == "other" { "local" } else { &config.model.profile },
        "messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":image}}, {"type":"text","text":prompt}]}],
        "temperature":0.0,"top_p":1.0,"top_k":1,"min_p":0.0,"seed":0,"stream":stream});
    if max_tokens > 0 {
        body["max_tokens"] = json!(max_tokens);
    }
    if stream {
        body["stream_options"] = json!({"include_usage":true});
    }
    body
}

fn read_log(path: &std::path::Path, offset: u64) -> String {
    let Ok(mut file) = fs::File::open(path) else {
        return String::new();
    };
    let end = file.metadata().map(|info| info.len()).unwrap_or(offset);
    let _ = file.seek(SeekFrom::Start(offset.max(end.saturating_sub(512 * 1024))));
    let mut bytes = Vec::new();
    let _ = file.take(512 * 1024).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn clean(text: &str) -> String {
    let text = text.trim();
    if text.starts_with("<think>")
        && let Some((_, content)) = text.split_once("</think>")
    {
        return content.trim().into();
    }
    text.into()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
