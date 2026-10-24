# Third-party notices

BlinkOCR's original application code is licensed under MIT. The components and
model resources listed here retain their own licenses.

| Component | Version / origin | License materials |
|---|---|---|
| llama.cpp | b11747, ggml-org/llama.cpp | [MIT](licenses/llama.cpp-MIT.txt) |
| TeleOCR Rust implementation | teleocr-rs v0.2.0, rzafiamy/teleocr-rs | [Apache-2.0](licenses/TeleOCR-Apache-2.0.txt), [credits](licenses/TeleOCR-CREDITS.md) |
| OAR-OCR | 0.10.1, GreatV/oar-ocr | [Apache-2.0](licenses/OAR-OCR-Apache-2.0.txt) |
| ONNX Runtime | 1.28.0, Microsoft | [MIT](licenses/ONNX-Runtime-MIT.txt), [third-party notices](licenses/ONNX-Runtime-ThirdPartyNotices.txt) |
| DirectML redistributable | Microsoft.AI.DirectML 1.15.4 | [binary redistribution terms](licenses/DirectML-Redistributable.txt), [source-code MIT license](licenses/DirectML-Code-MIT.txt), [third-party notices](licenses/DirectML-ThirdPartyNotices.txt) |
| LLVM OpenMP | runtime supplied with llama.cpp b11747 | [license](licenses/LLVM-OpenMP.txt) |
| CUDA Runtime and cuBLAS | CUDA runtime supplied with llama.cpp b11747 CUDA 13.4 package | [CUDA license](licenses/NVIDIA-CUDA-LICENSE.txt), [cuBLAS license](licenses/NVIDIA-cuBLAS-LICENSE.txt) |
| Manrope | @fontsource/manrope | SIL OFL-1.1; included in the npm dependency notices and distribution |
| Svelte, Lucide, Tauri JavaScript APIs | versions in package-lock.json | [npm dependency notices](licenses/NPM-DEPENDENCIES.txt) |
| Rust dependencies | versions in src-tauri/Cargo.lock | [Rust dependency notices](licenses/RUST-DEPENDENCIES.txt) |

The CUDA components and the DirectML binary are redistributable software under
their respective vendor terms; the application MIT license does not relicense them.
CUDA runtime files are included only in the CUDA distribution.

The packaged TeleOCR component is used for image recognition. Its optional
PDFium library is omitted because BlinkOCR accepts image files and screen captures.

## Included model resources

PP-OCRv6 character dictionaries are derived from PaddlePaddle's official ONNX
recognition configurations under Apache-2.0. Copyright belongs to the PaddleOCR
contributors. Their entry ordering is preserved.

- Tiny: `PaddlePaddle/PP-OCRv6_tiny_rec_onnx`, revision
  `2612ab37152ae0a677521bae4e1e3d4fb4cf7c30`, `inference.yml`, 6,904 entries.
- Small / Medium shared dictionary:
  `PaddlePaddle/PP-OCRv6_small_rec_onnx`, revision
  `b8f84f0b80c529de40b4fbb3544b84fa7233a513`, `inference.yml`, 18,708 entries.
- [PaddleOCR license](licenses/PaddleOCR-Apache-2.0.txt).

The dots.ocr task prompts are taken from
[rednote-hilab/dots.ocr](https://github.com/rednote-hilab/dots.ocr/blob/master/dots_ocr/utils/prompts.py),
Copyright (c) 2025 rednote-hilab, under [MIT](licenses/dots.ocr-MIT.txt).
Other model task instructions follow their publishers' model cards. Model files
are downloaded separately from the repositories recorded in
`src-tauri/resources/models.json`; model weights are not included in the source
repository or portable distribution. Each model retains its publisher's license.

## Dependency inventory

`licenses/dependencies.json` records package names, versions, declared licenses,
and public source URLs. `scripts/collect_licenses.mjs` regenerates the full Rust
and production npm notices from the lockfiles and installed dependency packages.
