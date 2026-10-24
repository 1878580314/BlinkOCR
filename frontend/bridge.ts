import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { initial, type Snapshot } from "./types";

type EngineEvent =
  | (Snapshot & { kind?: "state" })
  | {
      kind: "progress";
      revision: number;
      id: number;
      text: string;
      elapsed_ms: number;
    };

export const call = invoke;
export const copyText = writeText;

export async function subscribe(callback: (state: Snapshot) => void) {
  let latest = structuredClone(initial);
  const unlisten = await listen<EngineEvent>("ocr-state", ({ payload }) => {
    if (payload.revision < latest.revision) return;
    if (payload.kind === "progress") {
      if (latest.result?.id !== payload.id) return;
      latest = {
        ...latest,
        revision: payload.revision,
        result: {
          ...latest.result,
          text: payload.text,
          elapsed_ms: payload.elapsed_ms,
        },
      };
    } else latest = payload;
    callback(latest);
  });
  try {
    const snapshot = await invoke<Snapshot>("get_snapshot");
    if (snapshot.revision >= latest.revision) {
      latest = snapshot;
      callback(latest);
    }
  } catch (error) {
    unlisten();
    throw error;
  }
  return unlisten;
}
export function subscribeCapture(callback: () => void) {
  return listen("ocr-request-capture", callback);
}
