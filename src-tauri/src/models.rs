//! Verified Hugging Face catalogue and resumable, checksummed paired downloads.
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Semaphore,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeLanguage {
    pub tag: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFile {
    pub repo: String,
    pub revision: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    pub bytes: u64,
    pub sha256: String,
}
impl ModelFile {
    pub fn local_name(&self) -> &str {
        self.local_path.as_deref().unwrap_or(&self.path)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variant {
    pub id: String,
    pub quantization: String,
    pub publisher: String,
    pub published_at: String,
    pub files: Vec<ModelFile>,
    pub bytes: u64,
    pub projector_quantization: Option<String>,
    pub recommended: bool,
    pub hardware: String,
    #[serde(default)]
    pub installed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub source_repo: String,
    pub author: String,
    pub parameters: String,
    pub released_at: String,
    pub release_note: String,
    pub repository_created_at: String,
    pub backend: String,
    pub context_size: u32,
    pub variant_count: usize,
    pub variants: Vec<Variant>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalogue {
    pub updated_at: String,
    pub models: Vec<Model>,
}
pub static CATALOGUE: LazyLock<Catalogue> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../resources/models.json"))
        .expect("valid bundled model catalogue")
});

pub fn profile_valid(profile: &str) -> bool {
    profile == "other" || CATALOGUE.models.iter().any(|model| model.id == profile)
}
pub fn is_classic(profile: &str) -> bool {
    profile.starts_with("ppocrv6-") || profile == "windows-ocr"
}
pub fn find_variant(model: &str, variant: &str) -> Result<(&'static Model, &'static Variant)> {
    let model = CATALOGUE
        .models
        .iter()
        .find(|entry| entry.id == model)
        .context("模型不存在")?;
    Ok((
        model,
        model
            .variants
            .iter()
            .find(|entry| entry.id == variant)
            .context("量化版本不存在")?,
    ))
}

pub fn local_file(root: &Path, model: &str, file: &ModelFile) -> Option<PathBuf> {
    [
        root.join(model).join(file.local_name()),
        root.join(file.local_name()),
    ]
    .into_iter()
    .find(|path| {
        path.metadata()
            .is_ok_and(|meta| meta.is_file() && meta.len() == file.bytes)
    })
}

pub fn catalogue(root: &Path) -> Catalogue {
    let mut result = CATALOGUE.clone();
    for model in &mut result.models {
        for variant in &mut model.variants {
            variant.installed = if model.backend == "windows" {
                true
            } else {
                variant
                    .files
                    .iter()
                    .all(|file| local_file(root, &model.id, file).is_some())
            };
        }
    }
    result
}

pub fn variant_paths(
    root: &Path,
    model: &str,
    variant: &Variant,
) -> Result<(PathBuf, Option<PathBuf>)> {
    let paths = variant
        .files
        .iter()
        .map(|file| local_file(root, model, file).context("请先下载完整模型文件"))
        .collect::<Result<Vec<_>>>()?;
    let main = paths.first().context("系统 OCR 不需要模型文件")?.clone();
    Ok((main, paths.get(1).cloned()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
    pub id: String,
    pub model_id: String,
    pub variant_id: String,
    pub phase: String,
    pub downloaded: u64,
    pub total: u64,
    pub speed: u64,
    pub file: String,
    pub error: Option<String>,
}
struct Job {
    state: Download,
    cancel: CancellationToken,
}
pub struct Downloads {
    jobs: Mutex<HashMap<String, Job>>,
    slots: Semaphore,
    model_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    root: PathBuf,
    client: reqwest::Client,
    emit: Arc<dyn Fn(Vec<Download>) + Send + Sync>,
}
impl Downloads {
    pub fn new(
        root: PathBuf,
        emit: impl Fn(Vec<Download>) + Send + Sync + 'static,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            jobs: Mutex::new(HashMap::new()),
            slots: Semaphore::new(2),
            model_locks: Mutex::new(HashMap::new()),
            root,
            client: reqwest::Client::builder()
                .user_agent(concat!("BlinkOCR/", env!("CARGO_PKG_VERSION")))
                .tls_built_in_native_certs(true)
                .read_timeout(Duration::from_secs(90))
                .connect_timeout(Duration::from_secs(20))
                .build()?,
            emit: Arc::new(emit),
        }))
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn snapshot(&self) -> Vec<Download> {
        let mut list = self
            .jobs
            .lock()
            .unwrap()
            .values()
            .map(|job| job.state.clone())
            .collect::<Vec<_>>();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }
    fn update(&self, id: &str, edit: impl FnOnce(&mut Download)) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(id) {
            edit(&mut job.state);
        }
        (self.emit)(self.snapshot());
    }
    pub fn cancel(&self, id: &str) {
        if let Some(job) = self.jobs.lock().unwrap().get(id) {
            job.cancel.cancel();
        }
    }
    pub fn start(self: &Arc<Self>, model_id: &str, variant_id: &str) -> Result<String> {
        let (model, variant) = find_variant(model_id, variant_id)?;
        ensure!(model.backend != "windows", "系统 OCR 无需下载，请直接加载");
        let id = variant.id.clone();
        let token = CancellationToken::new();
        {
            let mut jobs = self.jobs.lock().unwrap();
            if let Some(job) = jobs.get(&id)
                && ["queued", "downloading", "verifying"].contains(&job.state.phase.as_str())
            {
                return Ok(id);
            }
            jobs.insert(
                id.clone(),
                Job {
                    state: Download {
                        id: id.clone(),
                        model_id: model.id.clone(),
                        variant_id: variant.id.clone(),
                        phase: "queued".into(),
                        downloaded: 0,
                        total: variant.bytes,
                        speed: 0,
                        file: String::new(),
                        error: None,
                    },
                    cancel: token.clone(),
                },
            );
        }
        (self.emit)(self.snapshot());
        let manager = self.clone();
        let job_id = id.clone();
        let model_lock = self
            .model_locks
            .lock()
            .unwrap()
            .entry(model.id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        tokio::spawn(async move {
            let result = async {
                let _model = tokio::select! { _ = token.cancelled() => bail!("已暂停下载"), guard = model_lock.lock() => guard };
                let _permit = tokio::select! { _ = token.cancelled() => bail!("已暂停下载"), permit = manager.slots.acquire() => permit? };
                let mut completed = 0;
                for file in &variant.files {
                    ensure!(!token.is_cancelled(), "已暂停下载");
                    let target = manager.root.join(&model.id).join(file.local_name());
                    ensure!(Path::new(file.local_name()).components().all(|part| matches!(part,Component::Normal(_))), "模型文件路径有误");
                    if let Some(existing) = local_file(&manager.root, &model.id, file) {
                        manager.update(&job_id, |state| { state.phase = "verifying".into(); state.file = file.path.clone(); });
                        if verify(&existing, file, &token).await.is_ok() { completed += file.bytes; continue; }
                    }
                    manager.download_file(&job_id, file, &target, completed, &token).await?;
                    completed += file.bytes;
                }
                Ok::<(), anyhow::Error>(())
            }.await;
            manager.update(&job_id, |state| {
                state.speed = 0;
                match result {
                    Ok(()) => {
                        state.phase = "completed".into();
                        state.downloaded = state.total;
                    }
                    Err(error) => {
                        state.phase = if token.is_cancelled() {
                            "paused"
                        } else {
                            "error"
                        }
                        .into();
                        state.error = Some(format!("{error:#}"));
                    }
                }
            });
        });
        Ok(id)
    }

    async fn download_file(
        &self,
        id: &str,
        file: &ModelFile,
        target: &Path,
        completed: u64,
        token: &CancellationToken,
    ) -> Result<()> {
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}?download=true",
            file.repo, file.revision, file.path
        );
        self.download_from(id, file, target, completed, token, &url)
            .await
    }

    async fn download_from(
        &self,
        id: &str,
        file: &ModelFile,
        target: &Path,
        completed: u64,
        token: &CancellationToken,
        url: &str,
    ) -> Result<()> {
        tokio::fs::create_dir_all(target.parent().context("模型目录有误")?).await?;
        let part = target.with_extension("gguf.part");
        let mut offset = tokio::fs::metadata(&part)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        if offset > file.bytes {
            tokio::fs::remove_file(&part).await?;
            offset = 0;
        }
        self.update(id, |state| {
            state.phase = "downloading".into();
            state.file = file.path.clone();
            state.downloaded = completed + offset;
            state.error = None;
        });
        if offset < file.bytes {
            let mut request = self.client.get(url);
            if offset > 0 {
                request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
            }
            let response = tokio::select! { _ = token.cancelled() => bail!("已暂停下载"), response = request.send() => response.context("连接 Hugging Face 失败")? };
            if response.status() == reqwest::StatusCode::UNAUTHORIZED
                || response.status() == reqwest::StatusCode::FORBIDDEN
            {
                bail!("此模型需要在 Hugging Face 登录或授权");
            }
            let response = response.error_for_status().context("模型下载请求失败")?;
            if offset > 0 && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                offset = 0;
            }
            if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                let expected = format!("bytes {offset}-");
                ensure!(
                    response
                        .headers()
                        .get(reqwest::header::CONTENT_RANGE)
                        .and_then(|h| h.to_str().ok())
                        .is_some_and(|h| h.starts_with(&expected)),
                    "下载续传位置不匹配，请重试"
                );
            }
            let mut output = tokio::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .append(offset > 0)
                .truncate(offset == 0)
                .open(&part)
                .await?;
            let start = Instant::now();
            let initial = offset;
            let mut last = Instant::now();
            let mut stream = response.bytes_stream();
            loop {
                let chunk = tokio::select! { _ = token.cancelled() => bail!("已暂停下载"), next = tokio::time::timeout(Duration::from_secs(90),stream.next()) => next.context("下载长时间无响应，请重试")? };
                let Some(chunk) = chunk else {
                    break;
                };
                let chunk = chunk.context("下载中断，可点击继续下载")?;
                ensure!(
                    offset + chunk.len() as u64 <= file.bytes,
                    "文件大小与模型信息不符"
                );
                output.write_all(&chunk).await?;
                offset += chunk.len() as u64;
                if last.elapsed() >= Duration::from_millis(150) {
                    self.update(id, |state| {
                        state.downloaded = completed + offset;
                        state.speed = ((offset - initial) as f64
                            / start.elapsed().as_secs_f64().max(0.001))
                            as u64;
                    });
                    last = Instant::now();
                }
            }
            output.flush().await?;
        }
        self.update(id, |state| {
            state.phase = "verifying".into();
            state.downloaded = completed + offset;
            state.speed = 0;
        });
        if let Err(error) = verify(&part, file, token).await {
            if !token.is_cancelled()
                && tokio::fs::metadata(&part)
                    .await
                    .is_ok_and(|m| m.len() == file.bytes)
            {
                let _ = tokio::fs::remove_file(&part).await;
            }
            return Err(error);
        }
        tokio::fs::rename(&part, target).await?;
        Ok(())
    }
}

pub async fn verify(path: &Path, file: &ModelFile, token: &CancellationToken) -> Result<()> {
    let mut input = tokio::fs::File::open(path).await?;
    ensure!(
        input.metadata().await?.len() == file.bytes,
        "文件尚未完整下载，可继续下载"
    );
    let mut buffer = vec![0; 1 << 20];
    let mut hash = Sha256::new();
    loop {
        ensure!(!token.is_cancelled(), "已暂停下载");
        let count = input.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    ensure!(
        format!("{:x}", hash.finalize()) == file.sha256,
        "文件 SHA-256 校验失败，请重新下载"
    );
    Ok(())
}
