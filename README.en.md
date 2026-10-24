# BlinkOCR

[简体中文](README.md) · English

A Rust-based Windows screenshot OCR tool for quickly capturing and recognizing screen content with a global hotkey. Supports model downloads, streaming results, and continuous recognition. Rust provides memory safety and low runtime overhead while integrating closely with Windows APIs, local inference processes, and asynchronous tasks.

![BlinkOCR interface](assets/screenshots/main.png)

## Features

- **Screen and image OCR** — select a screen region with a hotkey, or open PNG, JPEG, WebP, and BMP images.
- **Model management** — download from Hugging Face with pause, resume, and SHA-256 verification, or load local files.
- **Device selection** — GGUF models prioritize available NVIDIA GPUs, adjust offloaded layers to VRAM capacity, and fall back to CPU when needed. PP-OCRv6 uses ONNX Runtime.
- **Editable results** — stream, edit, copy, and save recognized text. Continuous recognition appends to the current content, which can be edited manually.
- **Model-specific tasks** — supported models offer task selection for text, tables, formulas, charts, and more.
- **Updates** — check GitHub Releases automatically at startup or manually from About, then download the matching Windows installer.

## Quick start

1. Download the Windows x64 installer (CPU or CUDA) from [GitHub Releases](https://github.com/1878580314/BlinkOCR/releases) and run it.
2. Open the model library to select, download, and load a model.
3. Once ready, press **End** to select a screen region, or choose an image.
4. Edit the result, then click “Copy All” or press **Ctrl+Shift+C**.

Configure shortcuts by pressing the desired keys in Settings → Experience. **Ctrl+Alt+Q** exits by default; closing the window minimizes it to the system tray.

The installer is the primary distribution format. Use About → Check for Updates to fetch and run a newer installer; automatic checks can be toggled in Settings → Experience.

## Models

| Model                          |          Parameters |
| ------------------------------ | ------------------: |
| Xiaomi-OCR-0                   |                0.8B |
| GLM-OCR                        |                0.9B |
| PaddleOCR-VL-1.6               |                0.9B |
| DeepSeek-OCR-2                 |                  3B |
| Unlimited-OCR                  |                  3B |
| Qianfan-OCR                    |                  4B |
| dots.ocr                       |                  3B |
| TeleOCR                        |                1.2B |
| PP-OCRv6 Tiny / Small / Medium | 1.5M / 7.7M / 34.5M |
| Windows.Media.Ocr              |                     |

The catalogue offers 50 precision or quantization variants across 11 downloadable model families, plus the Windows system component.

- GGUF models require a matching vision projector; TeleOCR uses one dedicated GGUF file.
- PP-OCRv6 uses detection and recognition ONNX files with a matching bundled character dictionary.
- Windows.Media.Ocr uses installed Windows OCR languages. Select a language or open system language settings from the model settings.
- Weights are downloaded separately into `models/`; local files in other locations can also be selected.

## Recognition settings

CPU threads default to half the logical core count. Image token settings follow the model defaults. The image long-edge limit defaults to 2048 for most models and 4000 for Windows OCR. Adjust for small-text quality and processing time; hover over or focus a setting for an explanation.

Xiaomi table enhancement applies to OTSL tables in Xiaomi-OCR's general mode.

Installed versions store settings in `%APPDATA%/BlinkOCR/config.rust.toml` and models under the corresponding `models/` folder; portable directories can use `config.rust.toml` beside the executable. Screenshots and recognition results are not saved automatically. Logs are stored in `%APPDATA%/BlinkOCR/logs/`.

## Requirements

- Windows 10/11 x64, Microsoft Edge WebView2 Runtime, and the [Microsoft Visual C++ v14 x64 Runtime](https://aka.ms/vc14/vc_redist.x64.exe).
- The CUDA package's GGUF backend supports compatible NVIDIA GPUs and drivers; the CPU package's GGUF backend uses CPU. Both include PP-OCRv6 and Windows OCR.
- RAM and VRAM requirements depend on the model, quantization, image size, and context. The catalogue provides hardware recommendations only.

## Build from source

Install **Node.js 24+**, **Rust 1.95+ with the MSVC toolchain**, and Visual Studio Build Tools with Desktop development with C++ and the Windows SDK. Follow the [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows).

From the project directory:

```powershell
npm ci
npm run package:win
```

The first packaging run downloads pinned llama.cpp and TeleOCR releases, verifies SHA-256 hashes, builds the application, and collects third-party licenses. It selects CUDA when NVIDIA tools are detected, otherwise CPU. The main output is an NSIS installer named `dist/BlinkOCR-<version>-windows-x64-<cpu|cuda>-setup.exe`, accompanied by a SHA-256 checksum. Use `-OutputDir` to choose a different output directory.

To select a backend explicitly:

```powershell
npm run package:win -- -Backend CUDA
npm run package:win -- -Backend CPU
```

For development:

```powershell
npm run setup:runtime
npm run desktop:dev
```

From WSL, select a separate build directory on a Windows drive:

```bash
bash scripts/build_windows_blinkocr.sh /mnt/d/build/BlinkOCR CUDA
```

## About

Author: [Minayuki](https://github.com/1878580314) · [GitHub repository](https://github.com/1878580314/BlinkOCR)

The About page shows the current version, checks GitHub Releases, displays release notes, and downloads the matching installer.

## License

BlinkOCR's original application code is licensed under [MIT](LICENSE). Model weights, CUDA, DirectML, and other components retain their own licenses; see [third-party notices](THIRD_PARTY_NOTICES.md).
