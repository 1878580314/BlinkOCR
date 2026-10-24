import registry from "../src-tauri/resources/profiles.json" with { type: "json" };
export interface Config {
  model: {
    model_path: string;
    mmproj_path: string;
    profile: string;
    n_gpu_layers: number;
    device_mode: "auto" | "cpu";
    context_size: number;
    threads: number;
    image_min_tokens: number;
  };
  server: {
    host: string;
    port: number;
    llama_server_bin: string;
    teleocr_server_bin: string;
    startup_timeout: number;
  };
  ocr: {
    native_language: string;
    prompt: string;
    max_tokens: number;
    max_image_dimension: number;
    xiaomi_table_postprocess: boolean;
    continuous_recognition: boolean;
  };
  hotkey: { enabled: boolean; trigger: string; quit: string };
  ui: {
    result_opacity: number;
    auto_copy: boolean;
    always_on_top: boolean;
    auto_check_updates: boolean;
  };
}
export interface Supplement {
  text: string;
  source: string;
  conflict: boolean;
}
export interface OcrRecord {
  id: number;
  text: string;
  supplements: Supplement[];
  warnings: string[];
  task: string;
  source: string;
  width: number;
  height: number;
  elapsed_ms: number;
  created_at: number;
  pages?: number;
  generation?: {
    tokens: number;
    milliseconds: number;
    includes_prefill?: boolean;
  } | null;
}
export interface Hardware {
  cpu_name?: string;
  logical_cores: number;
  total_ram_bytes: number | null;
  available_ram_bytes: number | null;
  gpus: {
    id: string;
    name: string;
    total_bytes: number;
    free_bytes: number;
    usable: boolean;
  }[];
}
export interface InferencePlan {
  mode: "gpu" | "hybrid" | "cpu";
  reason: string;
  gpu_name: string | null;
  estimated_vram_bytes: number;
  estimated_ram_bytes: number;
  fallback: boolean;
}
export interface Snapshot {
  revision: number;
  phase: string;
  message: string;
  ready: boolean;
  hotkey_error?: string | null;
  loaded_model?: string | null;
  native_languages?: { tag: string; name: string }[];
  hardware?: Hardware | null;
  inference?: InferencePlan | null;
  config: Config;
  result: OcrRecord | null;
  history: OcrRecord[];
}

export type TaskId =
  | "general"
  | "text"
  | "table"
  | "formula"
  | "chart"
  | "spotting"
  | "seal"
  | "code"
  | "layout";
export interface Task {
  id: TaskId;
  name: string;
  prompt: string;
}
export const tasks = registry.tasks as Task[];
export function profileTasks(profile: string) {
  const rules =
    registry.models[profile as keyof typeof registry.models] ??
    registry.models.other;
  return rules.modes.map((id) => tasks.find((task) => task.id === id)!);
}
export function normalizeProfile(config: Config) {
  const choices = profileTasks(config.model.profile);
  if (!choices.some((task) => task.prompt === config.ocr.prompt))
    config.ocr.prompt = choices[0].prompt;
  if (config.model.profile !== "xiaomi-ocr")
    config.ocr.xiaomi_table_postprocess = false;
}
export function balancedProfile(config: Config) {
  const rules =
    registry.models[config.model.profile as keyof typeof registry.models] ??
    registry.models.other;
  config.model.image_min_tokens = rules.image_min_tokens;
  config.ocr.max_image_dimension = rules.max_image_dimension;
}

export const defaults: Config = {
  model: {
    model_path: "",
    mmproj_path: "",
    profile: "xiaomi-ocr",
    n_gpu_layers: 99,
    device_mode: "auto",
    context_size: 16384,
    threads: Math.max(
      1,
      Math.min(
        256,
        Math.floor((globalThis.navigator?.hardwareConcurrency ?? 2) / 2),
      ),
    ),
    image_min_tokens: 0,
  },
  server: {
    host: "127.0.0.1",
    port: 8818,
    llama_server_bin: "bin/llama-server.exe",
    teleocr_server_bin: "bin/teleocr/teleocr.exe",
    startup_timeout: 180,
  },
  ocr: {
    native_language: "auto",
    prompt: tasks[0].prompt,
    max_tokens: -1,
    max_image_dimension: 2048,
    xiaomi_table_postprocess: false,
    continuous_recognition: false,
  },
  hotkey: { enabled: true, trigger: "End", quit: "Ctrl+Alt+Q" },
  ui: {
    result_opacity: 1,
    auto_copy: false,
    always_on_top: false,
    auto_check_updates: true,
  },
};
export const initial: Snapshot = {
  revision: 0,
  phase: "loading",
  message: "正在准备识别引擎",
  ready: false,
  config: defaults,
  result: null,
  history: [],
};
export const isBusy = (phase: string) =>
  ["selecting", "recognizing", "postprocessing"].includes(phase);

export function combined(text: string, supplements: Supplement[]) {
  let output = text;
  for (const source of ["上部", "中部", "下部"])
    for (const conflict of [false, true]) {
      const lines = supplements.filter(
        (item) => item.source === source && item.conflict === conflict,
      );
      if (lines.length)
        output += `\n\n【${conflict ? "差异待核对" : "补充识别"}·${source}】\n${lines.map((item) => item.text).join("\n")}`;
    }
  return output;
}
