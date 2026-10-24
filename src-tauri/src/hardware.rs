//! Capacity selection uses physical free memory and the selected backend's devices.
use anyhow::{Result, ensure};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::LazyLock, time::Duration};
use tokio::process::Command;

pub const MIB: u64 = 1 << 20;
pub const GIB: u64 = 1 << 30;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Gpu {
    pub id: String,
    pub name: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub reported_free_bytes: u64,
    pub usable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Hardware {
    pub cpu_name: String,
    pub logical_cores: usize,
    pub total_ram_bytes: Option<u64>,
    pub available_ram_bytes: Option<u64>,
    pub gpus: Vec<Gpu>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferencePlan {
    pub mode: String,
    pub reason: String,
    pub gpu_id: Option<String>,
    pub gpu_name: Option<String>,
    pub estimated_vram_bytes: u64,
    pub estimated_ram_bytes: u64,
    pub available_vram_bytes: u64,
    pub fit_margin_mib: u64,
    pub fallback: bool,
}

impl InferencePlan {
    pub fn arguments(&self) -> Vec<String> {
        if self.mode == "cpu" {
            [
                "-ngl",
                "0",
                "--device",
                "none",
                "--fit",
                "off",
                "--no-mmproj-offload",
                "--no-kv-offload",
                "--no-op-offload",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        } else {
            vec![
                "--device".into(),
                self.gpu_id.clone().unwrap(),
                "--fit".into(),
                "on".into(),
                "--fit-target".into(),
                self.fit_margin_mib.to_string(),
            ]
        }
    }
}

async fn output(program: &Path, arguments: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command.args(arguments).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let response = tokio::time::timeout(Duration::from_secs(6), command.output())
        .await
        .ok()?
        .ok()?;
    if !response.status.success() {
        return None;
    }
    Some(format!(
        "{}\n{}",
        String::from_utf8_lossy(&response.stdout),
        String::from_utf8_lossy(&response.stderr)
    ))
}

pub async fn detect(backend: Option<&Path>) -> Hardware {
    let (system, (devices, physical)) = tokio::join!(
        tokio::task::spawn_blocking(|| (system_memory(), cpu_name())),
        async {
            tokio::join!(
                async {
                    match backend {
                        Some(path) => output(path, &["--list-devices"]).await,
                        None => None,
                    }
                },
                output(
                    Path::new("nvidia-smi.exe"),
                    &[
                        "--query-gpu=index,name,memory.total,memory.free",
                        "--format=csv,noheader,nounits"
                    ]
                )
            )
        }
    );
    let ((total_ram_bytes, available_ram_bytes), cpu_name) =
        system.unwrap_or(((None, None), String::new()));
    let mut gpus = parse_devices(devices.as_deref().unwrap_or_default());
    reconcile_physical(&mut gpus, physical.as_deref().unwrap_or_default());
    Hardware {
        cpu_name,
        logical_cores: std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        total_ram_bytes,
        available_ram_bytes,
        gpus,
    }
}

fn cpu_name() -> String {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Registry::{
            HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
        };
        let key: Vec<u16> = "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let value: Vec<u16> = "ProcessorNameString"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut buffer = [0u16; 256];
        let mut size = std::mem::size_of_val(&buffer) as u32;
        if RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        ) == 0
        {
            let end = buffer
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(buffer.len());
            return String::from_utf16_lossy(&buffer[..end]).trim().into();
        }
    }
    #[cfg(not(windows))]
    if let Ok(text) = std::fs::read_to_string("/proc/cpuinfo") {
        return text
            .lines()
            .find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|line| line.split_once(':'))
                    .map(|(_, name)| name.trim().to_string())
            })
            .unwrap_or_default();
    }
    String::new()
}

fn system_memory() -> (Option<u64>, Option<u64>) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut status) != 0 {
            return (Some(status.ullTotalPhys), Some(status.ullAvailPhys));
        }
    }
    #[cfg(not(windows))]
    if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
        let read = |name: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(name))
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse::<u64>().ok())
                .map(|value| value * 1024)
        };
        return (read("MemTotal:"), read("MemAvailable:"));
    }
    (None, None)
}

fn parse_devices(text: &str) -> Vec<Gpu> {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)^\s*([A-Za-z]+\d+):\s*(.+?)\s*\((\d+) MiB,\s*(\d+) MiB free\)").unwrap()
    });
    PATTERN
        .captures_iter(text)
        .filter_map(|capture| {
            let total = capture[3].parse::<u64>().ok()? * MIB;
            let free = capture[4].parse::<u64>().ok()? * MIB;
            Some(Gpu {
                id: capture[1].into(),
                name: capture[2].into(),
                total_bytes: total,
                free_bytes: free.min(total),
                reported_free_bytes: free.min(total),
                usable: true,
            })
        })
        .collect()
}

fn reconcile_physical(gpus: &mut Vec<Gpu>, text: &str) {
    for line in text.lines() {
        let fields: Vec<_> = line.split(',').map(str::trim).collect();
        if fields.len() != 4 {
            continue;
        }
        let (Ok(total), Ok(free)) = (fields[2].parse::<u64>(), fields[3].parse::<u64>()) else {
            continue;
        };
        let id = format!("CUDA{}", fields[0]);
        if let Some(device) = gpus
            .iter_mut()
            .find(|device| device.id == id && device.name == fields[1])
        {
            device.free_bytes = device.free_bytes.min(free * MIB);
            device.total_bytes = device.total_bytes.min(total * MIB);
        } else if !gpus.iter().any(|device| device.name == fields[1]) {
            gpus.push(Gpu {
                id,
                name: fields[1].into(),
                total_bytes: total * MIB,
                free_bytes: free * MIB,
                reported_free_bytes: free * MIB,
                usable: false,
            });
        }
    }
}

fn memory_costs(config: &crate::config::Config, weights: u64, projector: u64) -> (u64, u64, u64) {
    let context = config.model.context_size as u64;
    let kv_per_8k = match config.model.profile.as_str() {
        "xiaomi-ocr" | "paddleocr" => 256 * MIB,
        "glm-ocr" | "dots-ocr" => 512 * MIB,
        "deepseek-ocr2" | "unlimited-ocr" => 1536 * MIB,
        "teleocr" => 1900 * MIB,
        _ => GIB,
    };
    let kv = kv_per_8k.saturating_mul(context) / 8192;
    let image_scale = (config.ocr.max_image_dimension as u64)
        .saturating_mul(config.ocr.max_image_dimension as u64)
        .max(2048 * 2048)
        / (2048 * 2048);
    let vision = if ["deepseek-ocr2", "unlimited-ocr"].contains(&config.model.profile.as_str()) {
        1536 * MIB
    } else {
        768 * MIB
    } * image_scale;
    let language = weights.saturating_mul(105) / 100 + kv + 256 * MIB;
    let projection = projector.saturating_mul(115) / 100 + vision;
    (
        language,
        projection,
        weights + projector + kv + vision + 512 * MIB,
    )
}

pub fn choose(
    config: &crate::config::Config,
    hardware: &Hardware,
    weights: u64,
    projector: u64,
) -> Result<InferencePlan> {
    let (language, projection, cpu_ram) = memory_costs(config, weights, projector);
    let mut plan = InferencePlan {
        mode: "cpu".into(),
        reason: "未检测到后端可用的 GPU，使用 CPU".into(),
        gpu_id: None,
        gpu_name: None,
        estimated_vram_bytes: language + projection,
        estimated_ram_bytes: cpu_ram,
        available_vram_bytes: 0,
        fit_margin_mib: 0,
        fallback: false,
    };
    if config.model.profile == "teleocr" {
        plan.reason = "当前 TeleOCR 组件使用 CPU".into();
    } else if config.model.device_mode == "cpu" {
        plan.reason = "已选择 CPU 推理".into();
    } else if let Some(gpu) = hardware
        .gpus
        .iter()
        .filter(|gpu| gpu.usable)
        .max_by_key(|gpu| gpu.free_bytes)
    {
        let desktop = (gpu.total_bytes / 10).max(GIB);
        let reserve = desktop + projection;
        let language_budget = gpu.free_bytes.saturating_sub(reserve);
        plan.available_vram_bytes = gpu.free_bytes;
        if language_budget >= 512 * MIB {
            plan.mode = if language <= language_budget {
                "gpu"
            } else {
                "hybrid"
            }
            .into();
            plan.reason = if plan.mode == "gpu" {
                "显存充足，使用 GPU"
            } else {
                "显存不足以容纳完整模型，自动分配 GPU 与 CPU"
            }
            .into();
            plan.gpu_id = Some(gpu.id.clone());
            plan.gpu_name = Some(gpu.name.clone());
            // WDDM can report a CUDA virtual budget larger than physical free VRAM.
            plan.fit_margin_mib =
                (reserve + gpu.reported_free_bytes.saturating_sub(gpu.free_bytes)).div_ceil(MIB);
            let cpu_fraction =
                language.saturating_sub(language_budget) as f64 / language.max(1) as f64;
            plan.estimated_ram_bytes = (weights as f64 * cpu_fraction) as u64 + 2 * GIB;
        } else {
            plan.reason = "空闲显存不足，使用 CPU".into();
        }
    }
    check_ram(hardware, plan.estimated_ram_bytes)?;
    Ok(plan)
}

pub fn cpu_fallback(
    config: &crate::config::Config,
    hardware: &Hardware,
    weights: u64,
    projector: u64,
) -> Result<InferencePlan> {
    let mut cpu = config.clone();
    cpu.model.device_mode = "cpu".into();
    let mut plan = choose(&cpu, hardware, weights, projector)?;
    plan.fallback = true;
    plan.reason = "GPU 加载失败，已切换 CPU".into();
    Ok(plan)
}

fn check_ram(hardware: &Hardware, required: u64) -> Result<()> {
    if let Some(available) = hardware.available_ram_bytes {
        let reserve = hardware
            .total_ram_bytes
            .unwrap_or(0)
            .saturating_div(20)
            .max(GIB);
        ensure!(
            required <= available.saturating_sub(reserve),
            "可用内存不足：预计需要 {:.1} GiB，当前可用 {:.1} GiB，请选择更小的量化版本",
            required as f64 / GIB as f64,
            available as f64 / GIB as f64
        );
    }
    Ok(())
}

pub fn gpu_failure(log: &str) -> bool {
    let log = log.to_ascii_lowercase();
    (log.contains("cuda") || log.contains("vulkan") || log.contains("sycl") || log.contains("gpu"))
        && [
            "out of memory",
            "failed to allocate",
            "no devices",
            "no cuda-capable",
            "driver version is insufficient",
            "failed to initialize",
            "device not found",
        ]
        .iter()
        .any(|needle| log.contains(needle))
}

pub fn actual_mode(plan: &mut InferencePlan, log: &str) {
    if plan.mode == "cpu" {
        return;
    }
    static OFFLOAD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"offloaded (\d+)/(\d+) layers").unwrap());
    if let Some(capture) = OFFLOAD.captures_iter(log).last() {
        let offload = capture[1].parse::<u32>().unwrap_or(0);
        let total = capture[2].parse::<u32>().unwrap_or(u32::MAX);
        plan.mode = if offload >= total { "gpu" } else { "hybrid" }.into();
        plan.reason = if plan.mode == "gpu" {
            "模型已加载到 GPU"
        } else {
            "模型由 GPU 与 CPU 共同推理"
        }
        .into();
    }
}
