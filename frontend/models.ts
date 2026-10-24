export interface ModelFile {
  repo: string;
  revision: string;
  path: string;
  bytes: number;
  sha256: string;
  local_path?: string;
}
export interface ModelVariant {
  id: string;
  quantization: string;
  publisher: string;
  published_at: string;
  files: ModelFile[];
  bytes: number;
  projector_quantization: string | null;
  recommended: boolean;
  hardware: string;
  installed: boolean;
}
export interface OcrModel {
  id: string;
  name: string;
  source_repo: string;
  author: string;
  parameters: string;
  released_at: string;
  release_note: string;
  repository_created_at: string;
  backend: string;
  context_size: number;
  variants: ModelVariant[];
}
export interface Catalogue {
  updated_at: string;
  models: OcrModel[];
}
export interface ModelDownload {
  id: string;
  model_id: string;
  variant_id: string;
  phase: string;
  downloaded: number;
  total: number;
  speed: number;
  file: string;
  error: string | null;
}
export const modelProfiles = [
  { id: "teleocr", name: "TeleOCR", parameters: "1.2B" },
  { id: "paddleocr", name: "PaddleOCR-VL-1.6", parameters: "0.9B" },
  { id: "glm-ocr", name: "GLM-OCR", parameters: "0.9B" },
  { id: "unlimited-ocr", name: "Unlimited-OCR", parameters: "3B" },
  { id: "deepseek-ocr2", name: "DeepSeek-OCR-2", parameters: "3B" },
  { id: "dots-ocr", name: "dots.ocr", parameters: "3B" },
  { id: "xiaomi-ocr", name: "Xiaomi-OCR-0", parameters: "0.8B" },
  { id: "qianfan-ocr", name: "Qianfan-OCR", parameters: "4B" },
  { id: "ppocrv6-tiny", name: "PP-OCRv6 Tiny", parameters: "1.5M" },
  { id: "ppocrv6-small", name: "PP-OCRv6 Small", parameters: "7.7M" },
  { id: "ppocrv6-medium", name: "PP-OCRv6 Medium", parameters: "34.5M" },
  {
    id: "windows-ocr",
    name: "Windows.Media.Ocr",
    parameters: "Windows系统内置",
  },
];
export function parameterGroups<T extends { parameters: string; name: string }>(
  models: T[],
) {
  const scale: Record<string, number> = { B: 1e9, M: 1e6, K: 1e3 };
  const size = (value: string) => {
    const match = value.match(/^([\d.]+)\s*([BMK])$/i);
    return match ? Number(match[1]) * scale[match[2].toUpperCase()] : Infinity;
  };
  const groups = new Map<string, T[]>();
  for (const model of models) {
    const label = model.parameters.trim();
    const group = groups.get(label) ?? [];
    group.push(model);
    groups.set(label, group);
  }
  return [...groups]
    .sort(([a], [b]) => size(a) - size(b))
    .map(([parameters, entries]) => ({
      parameters,
      models: entries.sort((a, b) => a.name.localeCompare(b.name)),
    }));
}
export function fileSize(bytes: number) {
  return bytes >= 1024 ** 3
    ? `${(bytes / 1024 ** 3).toFixed(2)} GiB`
    : `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
}
export const downloadPhase: Record<string, string> = {
  queued: "等待下载",
  downloading: "下载中",
  verifying: "校验中",
  completed: "下载完成",
  paused: "已暂停",
  error: "下载失败",
};

export const isClassic = (profile: string) =>
  profile.startsWith("ppocrv6-") || profile === "windows-ocr";
export const groupTitle = (parameters: string) =>
  parameters === "Windows系统内置" ? parameters : `${parameters} 参数`;
