//! GitHub Release discovery and verified download of Windows installers.
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Mutex, time::Duration};
use tokio::{fs, io::AsyncWriteExt, sync::Mutex as AsyncMutex};

#[derive(Debug, Deserialize)]
struct Project {
    repository: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
    digest: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub available: bool,
    pub release_url: String,
    pub notes: String,
    pub asset_name: Option<String>,
    pub asset_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadedUpdate {
    pub path: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Clone)]
struct Pending {
    version: String,
    asset: Asset,
}

pub struct Updates {
    client: reqwest::Client,
    repository: String,
    backend: String,
    pending: Mutex<Option<Pending>>,
    downloaded: Mutex<Option<(DownloadedUpdate, Asset)>>,
    downloading: AsyncMutex<()>,
}

fn parse_version(tag: &str) -> Result<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag)).context("Release 版本号格式不正确")
}

fn installer<'a>(assets: &'a [Asset], backend: &str) -> Option<&'a Asset> {
    let suffix = format!("-windows-x64-{}-setup.exe", backend.to_ascii_lowercase());
    assets.iter().find(|asset| {
        asset.name.to_ascii_lowercase().ends_with(&suffix)
            && asset.name.starts_with("BlinkOCR-")
            && asset
                .name
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || b"-_.".contains(&ch))
    })
}

async fn verify(path: &std::path::Path, asset: &Asset) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let mut file = fs::File::open(path).await?;
    ensure!(
        file.metadata().await?.len() == asset.size,
        "安装包文件大小不正确"
    );
    if let Some(digest) = &asset.digest {
        let expected = digest
            .strip_prefix("sha256:")
            .context("未知的安装包校验算法")?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let count = file.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        ensure!(
            format!("{:x}", hasher.finalize()) == expected,
            "安装包 SHA-256 校验失败"
        );
    }
    Ok(())
}

impl Updates {
    pub fn new(backend: String) -> Result<Self> {
        let project: Project = serde_json::from_str(include_str!("../resources/project.json"))?;
        let parts: Vec<_> = project.repository.split('/').collect();
        ensure!(
            parts.len() == 2
                && parts.iter().all(|part| !part.is_empty()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))),
            "GitHub 仓库配置无效"
        );
        Ok(Self {
            client: reqwest::Client::builder()
                .user_agent("BlinkOCR-update-check")
                .timeout(Duration::from_secs(25))
                .build()?,
            repository: project.repository,
            backend,
            pending: Mutex::new(None),
            downloaded: Mutex::new(None),
            downloading: AsyncMutex::new(()),
        })
    }

    pub async fn check(&self) -> Result<UpdateInfo> {
        let url = format!(
            "https://api.github.com/repos/{}/releases/latest",
            self.repository
        );
        let response: Release = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .context("无法连接 GitHub")?
            .error_for_status()
            .context("无法读取 GitHub Release")?
            .json()
            .await
            .context("Release 数据格式有误")?;
        let version = parse_version(&response.tag_name)?;
        let available = version > parse_version(env!("CARGO_PKG_VERSION"))?;
        let asset = if available {
            installer(&response.assets, &self.backend)
        } else {
            None
        };
        *self.pending.lock().unwrap() = asset.cloned().map(|asset| Pending {
            version: version.to_string(),
            asset,
        });
        *self.downloaded.lock().unwrap() = None;
        Ok(UpdateInfo {
            current_version: env!("CARGO_PKG_VERSION").into(),
            latest_version: version.to_string(),
            available,
            release_url: response.html_url,
            notes: response
                .body
                .unwrap_or_default()
                .chars()
                .take(3000)
                .collect(),
            asset_name: asset.map(|asset| asset.name.clone()),
            asset_size: asset.map(|asset| asset.size),
        })
    }

    pub async fn download(
        &self,
        directory: PathBuf,
        progress: impl Fn(DownloadProgress),
    ) -> Result<DownloadedUpdate> {
        let _guard = self.downloading.try_lock().context("安装包正在下载")?;
        let pending = self
            .pending
            .lock()
            .unwrap()
            .clone()
            .context("请先检查更新")?;
        let asset = &pending.asset;
        let expected_prefix = format!("https://github.com/{}/releases/download/", self.repository);
        ensure!(
            asset.browser_download_url.starts_with(&expected_prefix),
            "安装包下载地址无效"
        );
        fs::create_dir_all(&directory).await?;
        let target = directory.join(&asset.name);
        if target.is_file() && verify(&target, asset).await.is_ok() {
            let receipt = DownloadedUpdate {
                path: target.to_string_lossy().into_owned(),
                version: pending.version,
            };
            *self.downloaded.lock().unwrap() = Some((receipt.clone(), asset.clone()));
            return Ok(receipt);
        }
        let part = target.with_extension("exe.part");
        let response = self
            .client
            .get(&asset.browser_download_url)
            .timeout(Duration::from_secs(3600))
            .send()
            .await
            .context("无法下载 GitHub 安装包")?
            .error_for_status()
            .context("GitHub 安装包下载失败")?;
        let mut output = fs::File::create(&part).await?;
        let mut downloaded = 0u64;
        let mut stream = response.bytes_stream();
        let mut last = std::time::Instant::now();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("下载中断")?;
            downloaded += chunk.len() as u64;
            ensure!(downloaded <= asset.size, "安装包大小不符合 Release 记录");
            output.write_all(&chunk).await?;
            if last.elapsed() >= Duration::from_millis(150) {
                progress(DownloadProgress {
                    downloaded,
                    total: asset.size,
                });
                last = std::time::Instant::now();
            }
        }
        output.flush().await?;
        drop(output);
        if let Err(error) = verify(&part, asset).await {
            let _ = fs::remove_file(&part).await;
            return Err(error);
        }
        if target.exists() {
            fs::remove_file(&target).await?;
        }
        fs::rename(&part, &target).await?;
        progress(DownloadProgress {
            downloaded: asset.size,
            total: asset.size,
        });
        let receipt = DownloadedUpdate {
            path: target.to_string_lossy().into_owned(),
            version: pending.version,
        };
        *self.downloaded.lock().unwrap() = Some((receipt.clone(), asset.clone()));
        Ok(receipt)
    }

    pub async fn launch(&self) -> Result<()> {
        let (receipt, asset) = self
            .downloaded
            .lock()
            .unwrap()
            .clone()
            .context("请先下载安装包")?;
        let path = PathBuf::from(&receipt.path);
        verify(&path, &asset).await?;
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            std::process::Command::new(&path)
                .creation_flags(0x08000000)
                .spawn()
                .context("无法启动安装程序")?;
        }
        #[cfg(not(windows))]
        anyhow::bail!("安装程序仅支持 Windows");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_versions() {
        assert!(parse_version("v0.2.0").unwrap() > parse_version("0.1.0").unwrap());
        assert!(parse_version("v0.1.0-beta.1").unwrap() < parse_version("0.1.0").unwrap());
    }
    #[test]
    fn selects_matching_installer() {
        let assets = [Asset {
            name: "BlinkOCR-0.2.0-windows-x64-cuda-setup.exe".into(),
            size: 12,
            browser_download_url: String::new(),
            digest: None,
        }];
        assert!(installer(&assets, "CUDA").is_some());
        assert!(installer(&assets, "CPU").is_none());
    }
}
