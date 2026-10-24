//! Native OCR paths: PP-OCRv6 via OAR/ONNX Runtime, and Windows' installed OCR languages.
use crate::{
    config::Config,
    hardware::{GIB, Hardware, InferencePlan, MIB},
    models::NativeLanguage,
};
use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use oar_ocr::{
    core::config::{OrtExecutionProvider, OrtSessionConfig},
    domain::TextDetectionConfig,
    oarocr::{OAROCR, OAROCRBuilder},
};
use std::path::Path;
use windows::{
    Globalization::Language,
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    core::HSTRING,
};

struct Runtime;
impl Runtime {
    fn initialize() -> Result<Self> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        Ok(Self)
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

pub fn languages() -> Vec<NativeLanguage> {
    let Ok(_runtime) = Runtime::initialize() else {
        return Vec::new();
    };
    let Ok(languages) = OcrEngine::AvailableRecognizerLanguages() else {
        return Vec::new();
    };
    languages
        .into_iter()
        .filter_map(|language| {
            Some(NativeLanguage {
                tag: language.LanguageTag().ok()?.to_string(),
                name: language.DisplayName().ok()?.to_string(),
            })
        })
        .collect()
}
fn windows_engine(language: &str) -> Result<OcrEngine> {
    (if language == "auto" {
        OcrEngine::TryCreateFromUserProfileLanguages()
    } else {
        OcrEngine::TryCreateFromLanguage(&Language::CreateLanguage(&HSTRING::from(language))?)
    })
    .context("未安装对应的 Windows OCR 语言，请在系统语言设置中安装文字识别组件")
}

pub enum Classic {
    Paddle(Box<OAROCR>),
    Windows,
}
impl Classic {
    pub fn load(
        config: &Config,
        model: &Path,
        recognizer: &Path,
        host: &Hardware,
    ) -> Result<(Self, InferencePlan, String)> {
        let windows = config.model.profile == "windows-ocr";
        let weights = if windows {
            0
        } else {
            ensure!(model.is_file(), "请选择 ONNX 检测模型");
            ensure!(recognizer.is_file(), "请选择 ONNX 识别模型");
            model.metadata()?.len() + recognizer.metadata()?.len()
        };
        let workspace = if config.model.profile == "ppocrv6-medium" {
            GIB
        } else {
            512 * MIB
        };
        let required = weights + workspace;
        if let Some(available) = host.available_ram_bytes {
            ensure!(
                available > required + GIB,
                "可用内存不足，请关闭其他程序或使用 Tiny 规格"
            );
        }
        let mut plan = InferencePlan {
            mode: "cpu".into(),
            reason: "使用 CPU 推理".into(),
            gpu_id: None,
            gpu_name: None,
            estimated_vram_bytes: if windows { 0 } else { required },
            estimated_ram_bytes: required,
            available_vram_bytes: 0,
            fit_margin_mib: 0,
            fallback: false,
        };
        if windows {
            let _runtime = Runtime::initialize()?;
            let engine = windows_engine(&config.ocr.native_language)?;
            let name = engine.RecognizerLanguage()?.DisplayName()?.to_string();
            plan.reason = "使用 Windows 系统 OCR".into();
            return Ok((Self::Windows, plan, format!("Windows.Media.Ocr · {name}")));
        }
        let gpu = host.gpus.iter().max_by_key(|gpu| gpu.free_bytes);
        let want_gpu = config.model.device_mode == "auto"
            && gpu.is_some_and(|gpu| gpu.free_bytes > required + (gpu.total_bytes / 10).max(GIB));
        let session = if want_gpu {
            OrtSessionConfig::auto().resolve_auto()
        } else {
            OrtSessionConfig::new().with_execution_providers(vec![OrtExecutionProvider::CPU])
        };
        let accelerated = session
            .execution_providers
            .iter()
            .flatten()
            .any(|provider| !matches!(provider, OrtExecutionProvider::CPU));
        let build = |session: OrtSessionConfig| {
            let dictionary = match config.model.profile.as_str() {
                "ppocrv6-tiny" => include_str!("../resources/ppocrv6-tiny-dict.txt"),
                _ => include_str!("../resources/ppocrv6-dict.txt"),
            };
            OAROCRBuilder::new(model, recognizer, "")
                .character_dict_content(dictionary)
                .ort_session(session.with_intra_threads(config.model.threads as usize))
                .image_batch_size(1)
                .region_batch_size(if config.model.profile == "ppocrv6-tiny" {
                    16
                } else {
                    4
                })
                .text_detection_config(TextDetectionConfig {
                    score_threshold: 0.2,
                    box_threshold: if config.model.profile == "ppocrv6-tiny" {
                        0.4
                    } else {
                        0.45
                    },
                    unclip_ratio: 1.4,
                    max_candidates: 3000,
                    ..Default::default()
                })
                .build()
        };
        let ocr = match build(session) {
            Ok(ocr) => {
                if accelerated {
                    plan.mode = "gpu".into();
                    plan.gpu_id = Some("DML0".into());
                    plan.available_vram_bytes = gpu.map(|gpu| gpu.free_bytes).unwrap_or(0);
                    plan.reason = "使用 DirectML GPU 推理".into();
                    plan.gpu_name = gpu.map(|gpu| gpu.name.clone());
                }
                ocr
            }
            Err(error) if accelerated => {
                plan.fallback = true;
                plan.reason = "GPU 初始化失败，已切换 CPU".into();
                build(
                    OrtSessionConfig::new()
                        .with_execution_providers(vec![OrtExecutionProvider::CPU]),
                )
                .with_context(|| format!("CPU 加载失败；GPU 初始化信息：{error}"))?
            }
            Err(error) => return Err(error.into()),
        };
        let name = crate::models::CATALOGUE
            .models
            .iter()
            .find(|model| model.id == config.model.profile)
            .unwrap()
            .name
            .clone();
        Ok((Self::Paddle(Box::new(ocr)), plan, format!("{name} · FP32")))
    }
    pub fn recognize(&self, image: DynamicImage, language: &str) -> Result<String> {
        match self {
            Self::Paddle(ocr) => {
                let result = ocr.predict(vec![image.to_rgb8()])?;
                Ok(result
                    .first()
                    .map(|page| page.concatenated_text("\n"))
                    .unwrap_or_default())
            }
            Self::Windows => {
                let _runtime = Runtime::initialize()?;
                let engine = windows_engine(language)?;
                let maximum = OcrEngine::MaxImageDimension()?;
                let image = if image.width() > maximum || image.height() > maximum {
                    image.thumbnail(maximum, maximum)
                } else {
                    image
                };
                let mut rgba = image.to_rgba8();
                for pixel in rgba.pixels_mut() {
                    pixel.0.swap(0, 2);
                }
                let writer = DataWriter::new()?;
                writer.WriteBytes(rgba.as_raw())?;
                let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
                    &writer.DetachBuffer()?,
                    BitmapPixelFormat::Bgra8,
                    image.width() as i32,
                    image.height() as i32,
                )?;
                let result = engine.RecognizeAsync(&bitmap)?.join()?;
                let text = result
                    .Lines()?
                    .into_iter()
                    .map(|line| line.Text().map(|text| text.to_string()))
                    .collect::<windows::core::Result<Vec<_>>>()?
                    .join("\n");
                Ok(text)
            }
        }
    }
}
