#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    blinkocr::desktop::run();
    #[cfg(not(windows))]
    eprintln!("BlinkOCR 的桌面程序运行于 Windows；核心测试可在此平台运行。");
}
