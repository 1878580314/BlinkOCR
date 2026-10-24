use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub model: ModelConfig,
    pub server: ServerConfig,
    pub ocr: OcrConfig,
    pub hotkey: HotkeyConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    pub model_path: String,
    pub mmproj_path: String,
    pub profile: String,
    pub n_gpu_layers: i32,
    pub device_mode: String,
    pub context_size: u32,
    pub threads: u16,
    pub image_min_tokens: u32,
}
impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            model_path: String::new(),
            mmproj_path: String::new(),
            profile: "xiaomi-ocr".into(),
            n_gpu_layers: 99,
            device_mode: "auto".into(),
            context_size: 16384,
            threads: default_threads(
                std::thread::available_parallelism()
                    .map(usize::from)
                    .unwrap_or(1),
            ),
            image_min_tokens: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub llama_server_bin: String,
    pub teleocr_server_bin: String,
    pub startup_timeout: u64,
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8818,
            llama_server_bin: "bin/llama-server.exe".into(),
            teleocr_server_bin: "bin/teleocr/teleocr.exe".into(),
            startup_timeout: 180,
        }
    }
}
impl ServerConfig {
    pub fn base_url(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("http://{host}:{}", self.port)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OcrConfig {
    pub native_language: String,
    pub prompt: String,
    pub max_tokens: i32,
    pub max_image_dimension: u32,
    /// Independent opt-in: legacy rescue_edge_bands must never enable this.
    pub xiaomi_table_postprocess: bool,
    pub continuous_recognition: bool,
}
impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            native_language: "auto".into(),
            prompt: crate::profiles::selector("general").into(),
            max_tokens: -1,
            max_image_dimension: 2048,
            xiaomi_table_postprocess: false,
            continuous_recognition: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    pub enabled: bool,
    pub trigger: String,
    pub quit: String,
}
impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger: "End".into(),
            quit: "Ctrl+Alt+Q".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub result_opacity: f64,
    pub auto_copy: bool,
    pub always_on_top: bool,
    pub auto_check_updates: bool,
}
impl Default for UiConfig {
    fn default() -> Self {
        Self {
            result_opacity: 1.0,
            auto_copy: false,
            always_on_top: false,
            auto_check_updates: true,
        }
    }
}

impl Config {
    pub fn normalize(&mut self) {
        if self.model.profile != "other"
            || crate::profiles::task_for_prompt(&self.ocr.prompt).is_some()
        {
            self.ocr.prompt = crate::profiles::selector(crate::profiles::selected_task(
                &self.model.profile,
                &self.ocr.prompt,
            ))
            .into();
        }
        if self.model.profile != "xiaomi-ocr" {
            self.ocr.xiaomi_table_postprocess = false;
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            ["auto", "cpu"].contains(&self.model.device_mode.as_str()),
            "请选择自动或 CPU 推理"
        );
        ensure!(
            crate::models::profile_valid(&self.model.profile),
            "请选择支持的模型类型"
        );
        ensure!(self.server.port > 0, "端口应在 1–65535 之间");
        ensure!(
            ["127.0.0.1", "localhost", "::1"].contains(&self.server.host.as_str()),
            "本地服务地址应为 localhost、127.0.0.1 或 ::1"
        );
        ensure!(
            (1024..=262144).contains(&self.model.context_size),
            "上下文长度应在 1024–262144 之间"
        );
        ensure!(
            self.model.threads > 0 && self.model.threads <= 256,
            "CPU 线程数应在 1–256 之间"
        );
        ensure!(
            self.model.n_gpu_layers >= -1 && self.model.n_gpu_layers <= 999,
            "GPU 层数应在 -1–999 之间"
        );
        ensure!(
            self.model.image_min_tokens <= self.model.context_size,
            "图像最小 token 数应为 0（模型默认）或不超过上下文长度"
        );
        ensure!(
            self.ocr.max_tokens == -1 || self.ocr.max_tokens > 0,
            "输出长度应为正数或 -1（不限）"
        );
        ensure!(
            (256..=8192).contains(&self.ocr.max_image_dimension),
            "图片长边应在 256–8192 之间"
        );
        ensure!(
            (5..=900).contains(&self.server.startup_timeout),
            "加载超时应在 5–900 秒之间"
        );
        ensure!(!self.ocr.prompt.trim().is_empty(), "识别指令不能为空");
        ensure!(
            !self.hotkey.enabled || !self.hotkey.trigger.trim().is_empty(),
            "请设置框选快捷键"
        );
        ensure!(
            !self.hotkey.enabled
                || self.hotkey.trigger.to_lowercase() != self.hotkey.quit.to_lowercase(),
            "框选和退出快捷键应不同"
        );
        Ok(())
    }

    pub fn needs_restart(&self, next: &Self) -> bool {
        self.model != next.model
            || self.server != next.server
            || self.ocr.max_image_dimension != next.ocr.max_image_dimension
            || (next.model.profile == "windows-ocr"
                && self.ocr.native_language != next.ocr.native_language)
    }

    pub fn postprocess_enabled(&self, output: &str) -> bool {
        self.model.profile == "xiaomi-ocr"
            && self.ocr.xiaomi_table_postprocess
            && self.ocr.prompt == crate::profiles::selector("general")
            && (output.contains("<fcel>") || output.contains("<nl>"))
    }

    pub fn task(&self) -> &'static str {
        crate::profiles::selected_task(&self.model.profile, &self.ocr.prompt)
    }
    pub fn effective_prompt(&self) -> &str {
        crate::profiles::effective_prompt(&self.model.profile, &self.ocr.prompt)
    }
    pub fn api_task(&self) -> &'static str {
        crate::profiles::api_task(&self.model.profile, &self.ocr.prompt)
    }
}

pub fn default_threads(logical_cores: usize) -> u16 {
    (logical_cores / 2).clamp(1, 256) as u16
}

pub struct ConfigStore {
    pub config: Config,
    pub path: PathBuf,
    pub base_dir: PathBuf,
    pub exe_dir: PathBuf,
}
impl ConfigStore {
    pub fn load() -> Result<Self> {
        let exe_dir = env::current_exe()?
            .parent()
            .context("无法定位程序目录")?
            .to_path_buf();
        let app_dir = env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
                    .join(".config")
            })
            .join("BlinkOCR");
        let legacy_app_dir = app_dir.parent().unwrap().join("ocr-tool");
        let explicit = env::var_os("BLINKOCR_CONFIG")
            .or_else(|| env::var_os("OCR_TOOL_CONFIG"))
            .map(PathBuf::from);
        let mut candidates = vec![
            exe_dir.join("config.rust.toml"),
            exe_dir.join("config.portable.toml"),
            app_dir.join("config.rust.toml"),
            app_dir.join("config.toml"),
            legacy_app_dir.join("config.rust.toml"),
            legacy_app_dir.join("config.toml"),
        ];
        if cfg!(debug_assertions) {
            let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
            candidates.push(project.join("config.rust.toml"));
            candidates.push(project.join("config.toml"));
        }
        let source = if let Some(path) = explicit.as_ref() {
            ensure!(path.is_file(), "指定的配置文件不存在：{}", path.display());
            Some(path.clone())
        } else {
            candidates.into_iter().find(|path| path.is_file())
        };
        match source {
            Some(source) => Self::from_file(&source, exe_dir, explicit.is_some()),
            None => Ok(Self {
                config: Config::default(),
                base_dir: app_dir.clone(),
                path: app_dir.join("config.rust.toml"),
                exe_dir,
            }),
        }
    }

    pub fn from_file(source: &Path, exe_dir: PathBuf, explicit: bool) -> Result<Self> {
        let mut config: Config =
            toml::from_str(&fs::read_to_string(source)?).context("配置格式有误")?;
        config.ocr.prompt = match config.ocr.prompt.as_str() {
            "Text Recognition:" => crate::profiles::selector("text").into(),
            "Table Recognition:" => crate::profiles::selector("table").into(),
            "Formula Recognition:" => crate::profiles::selector("formula").into(),
            "Recognize all content in the image: output plain text as-is, formulas in LaTeX, and tables in Markdown." => {
                crate::profiles::selector("general").into()
            }
            text if text.starts_with(
                "Extract all information from the image and represent it in markdown",
            ) =>
            {
                crate::profiles::selector("general").into()
            }
            _ => config.ocr.prompt,
        };
        config.normalize();
        config.validate()?;
        let source = fs::canonicalize(source)?;
        let base_dir = source.parent().context("配置文件没有父目录")?.to_path_buf();
        // A Python configuration is imported, then saved separately so its original remains usable.
        let path = if explicit
            || source
                .file_name()
                .is_some_and(|name| name == "config.rust.toml")
        {
            source
        } else {
            base_dir.join("config.rust.toml")
        };
        Ok(Self {
            config,
            path,
            base_dir,
            exe_dir,
        })
    }

    pub fn resolve(&self, value: &str) -> PathBuf {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            self.base_dir.join(path)
        }
    }

    pub fn backend_path(&self) -> PathBuf {
        let teleocr = self.config.model.profile == "teleocr";
        let configured = self.resolve(if teleocr {
            &self.config.server.teleocr_server_bin
        } else {
            &self.config.server.llama_server_bin
        });
        if configured.is_file() {
            return configured;
        }
        for root in [&self.exe_dir, &self.base_dir] {
            for relative in if teleocr {
                [
                    "bin/teleocr/teleocr.exe",
                    "resources/bin/teleocr/teleocr.exe",
                ]
            } else {
                ["bin/llama-server.exe", "resources/bin/llama-server.exe"]
            } {
                let path = root.join(relative);
                if path.is_file() {
                    return path;
                }
            }
        }
        #[cfg(debug_assertions)]
        {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join(".cache/runtime/bin");
            let binary = root.join(if teleocr {
                "teleocr/teleocr.exe"
            } else {
                "llama-server.exe"
            });
            if binary.is_file() {
                return binary;
            }
        }
        configured
    }

    pub fn save(&mut self, mut config: Config) -> Result<()> {
        config.normalize();
        config.validate()?;
        fs::create_dir_all(&self.base_dir)?;
        let text = toml::to_string_pretty(&config)?;
        let temp = self.path.with_extension("toml.tmp");
        fs::write(&temp, text).context("无法保存设置")?;
        if self.path.exists() {
            let backup = self.path.with_extension("toml.bak");
            fs::copy(&self.path, &backup)?;
        }
        fs::rename(&temp, &self.path).context("保存设置失败")?;
        self.config = config;
        Ok(())
    }
}
