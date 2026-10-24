<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    Cpu,
    Download,
    Pause,
    Play,
    FolderOpen,
    ExternalLink,
    LoaderCircle,
    Check,
    HardDrive,
  } from "@lucide/svelte";
  import { call } from "./bridge";
  import { hint, hints } from "./hints";
  import type { Snapshot } from "./types";
  import {
    type Catalogue,
    type ModelDownload,
    fileSize,
    downloadPhase,
    parameterGroups,
    groupTitle,
  } from "./models";
  let {
    snapshot,
    onLoaded,
    onLocal,
  }: {
    snapshot: Snapshot;
    onLoaded: (state: Snapshot) => void;
    onLocal: (profile: string) => void;
  } = $props();
  let catalogue = $state<Catalogue>({ updated_at: "", models: [] });
  let downloads = $state<ModelDownload[]>([]);
  let selectedId = $state("xiaomi-ocr");
  let variantId = $state("");
  let pending = $state(false);
  let error = $state("");
  let fetching = $state(true);
  const groups = $derived(parameterGroups(catalogue.models));
  const model = $derived(
    catalogue.models.find((model) => model.id === selectedId) ??
      catalogue.models[0],
  );
  const systemModel = $derived(model?.backend === "windows");
  const variant = $derived(
    model?.variants.find((variant) => variant.id === variantId) ??
      model?.variants[0],
  );
  const job = $derived(downloads.find((job) => job.id === variant?.id));
  const active = $derived(
    job && ["queued", "downloading", "verifying"].includes(job.phase),
  );
  const current = $derived(
    snapshot.ready &&
      snapshot.config.model.profile === model?.id &&
      (systemModel ||
        snapshot.config.model.model_path
          .replaceAll("\\", "/")
          .endsWith(
            variant?.files[0]?.local_path ?? variant?.files[0]?.path ?? "---",
          )),
  );
  const unavailable = $derived(
    ["selecting", "recognizing", "postprocessing", "loading"].includes(
      snapshot.phase,
    ),
  );
  async function refresh() {
    catalogue = await call<Catalogue>("get_model_catalogue");
  }
  onMount(() => {
    selectedId =
      snapshot.config.model.profile === "other"
        ? "xiaomi-ocr"
        : snapshot.config.model.profile;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void Promise.all([
      refresh(),
      call<ModelDownload[]>("get_model_downloads").then(
        (value) => (downloads = value),
      ),
    ])
      .catch((failure) => (error = String(failure)))
      .finally(() => (fetching = false));
    void listen<ModelDownload[]>("model-downloads", (event) => {
      const completed = event.payload.some(
        (job) =>
          job.phase === "completed" &&
          downloads.find((previous) => previous.id === job.id)?.phase !==
            "completed",
      );
      downloads = event.payload;
      if (completed) void refresh();
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });
  async function download(modelId = model?.id, selectedVariant = variant?.id) {
    if (!modelId || !selectedVariant) return;
    error = "";
    try {
      await call("download_model", { modelId, variantId: selectedVariant });
      downloads = await call<ModelDownload[]>("get_model_downloads");
    } catch (failure) {
      error = String(failure);
    }
  }
  async function load() {
    if (!model || !variant) return;
    pending = true;
    error = "";
    try {
      onLoaded(
        await call<Snapshot>("load_model", {
          modelId: model.id,
          variantId: variant.id,
        }),
      );
    } catch (failure) {
      error = String(failure);
    } finally {
      pending = false;
    }
  }
  async function openPage(quantized = false) {
    if (model)
      await call("open_model_page", {
        modelId: model.id,
        variantId: quantized ? variant?.id : null,
      });
  }
</script>

<main class="model-library">
  <div class="library-heading">
    <h1>模型库</h1>
    <button
      class="secondary-button"
      onclick={() => onLocal(model?.id ?? "xiaomi-ocr")}
      ><FolderOpen size={17} />{systemModel ? "设置语言" : "本地加载"}</button
    >
  </div>
  {#if fetching}<div class="library-loading">
      <LoaderCircle size={22} class="spin" />读取模型列表
    </div>
  {:else if model && variant}
    <div class="library-scroll">
      <label class="field library-picker"
        >模型<select
          aria-label="选择在线模型"
          value={model.id}
          onchange={(event) => {
            selectedId = event.currentTarget.value;
            variantId = "";
            error = "";
          }}
          >{#each groups as group}<optgroup label={groupTitle(group.parameters)}
              >{#each group.models as entry}<option value={entry.id}
                  >{entry.name}</option
                >{/each}</optgroup
            >{/each}</select
        ></label
      >
      <section class="model-card">
        <div class="model-heading">
          <span class="model-icon"><Cpu size={23} /></span>
          <div>
            <h2>{model.name}</h2>
            <span>{model.source_repo}</span>
          </div>
          <button
            class="icon-button"
            title={systemModel
              ? "打开 Microsoft OCR 文档"
              : "打开 Hugging Face 模型页面"}
            aria-label="打开模型页面"
            onclick={() => openPage()}><ExternalLink size={17} /></button
          >
        </div>
        <dl class="model-metadata">
          <div>
            <dt>参数量（标称）</dt>
            <dd>{model.parameters}</dd>
          </div>
          <div>
            <dt>
              {model.release_note.includes("创建") ? "HF 创建日期" : "发布日期"}
            </dt>
            <dd>{model.released_at}<small>{model.release_note}</small></dd>
          </div>
          <div>
            <dt>模型作者</dt>
            <dd>{model.author}</dd>
          </div>
          <div>
            <dt>推理方式</dt>
            <dd>
              {systemModel
                ? "Windows 系统 OCR · CPU"
                : model.backend === "onnx"
                  ? "ONNX · CPU / DirectML"
                  : model.backend === "teleocr"
                    ? "TeleOCR · CPU"
                    : "GPU / CPU"}
            </dd>
          </div>
        </dl>
        <label
          class="field"
          use:hint={model.backend === "onnx"
            ? hints.onnxPrecision
            : systemModel
              ? hints.language
              : hints.quantization}
          >{systemModel ? "版本" : "量化版本"}<select
            aria-label="选择量化版本"
            value={variant.id}
            onchange={(event) => {
              variantId = event.currentTarget.value;
              error = "";
            }}
            >{#each model.variants as item}<option value={item.id}
                >{item.quantization}{systemModel
                  ? ""
                  : ` · ${fileSize(item.bytes)}`} · {item.publisher}{item.recommended
                  ? " · 推荐"
                  : ""}</option
              >{/each}</select
          ></label
        >
        {#if !systemModel}<dl class="model-metadata variant-metadata">
            <div>
              <dt>文件总大小</dt>
              <dd>{fileSize(variant.bytes)}</dd>
            </div>
            <div>
              <dt>量化发布者</dt>
              <dd>
                <button class="source-link" onclick={() => openPage(true)}
                  >{variant.publisher}<ExternalLink size={12} /></button
                >
              </dd>
            </div>
            {#if variant.projector_quantization}<div>
                <dt>视觉投影精度</dt>
                <dd>{variant.projector_quantization}</dd>
              </div>{/if}
            <div>
              <dt>量化仓库创建</dt>
              <dd>{variant.published_at}</dd>
            </div>
          </dl>
        {/if}
        <p class="hardware-advice"><HardDrive size={16} />{variant.hardware}</p>
        {#if !systemModel}<details class="model-files">
            <summary>文件明细 · {variant.files.length} 个文件</summary
            >{#each variant.files as file}<div>
                <span title={file.path}>{file.local_path ?? file.path}</span
                ><small>{fileSize(file.bytes)}</small>
              </div>{/each}
          </details>
        {/if}{#if systemModel}<button
            class="secondary-button native-languages"
            onclick={() => call("open_ocr_languages")}
            >安装或管理 OCR 语言</button
          >{/if}
        {#if job}<div class="download-progress">
            <div>
              <span>{downloadPhase[job.phase]}</span><span
                >{fileSize(job.downloaded)} / {fileSize(job.total)}{job.speed >
                0
                  ? ` · ${fileSize(job.speed)}/s`
                  : ""}</span
              >
            </div>
            <progress value={job.downloaded} max={job.total}
            ></progress>{#if job.error && job.phase === "error"}<p role="alert">
                {job.error}
              </p>{/if}
          </div>{/if}
        <div class="model-actions">
          {#if active}<button
              class="secondary-button"
              onclick={() => call("pause_model_download", { id: job!.id })}
              ><Pause size={16} />暂停</button
            >
          {:else if variant.installed}<span class="downloaded-label"
              ><Check size={15} />{systemModel
                ? "Windows系统内置"
                : "已下载"}</span
            ><button
              class="primary-button"
              disabled={pending || unavailable || current}
              onclick={load}
              >{#if pending}<LoaderCircle
                  size={16}
                  class="spin"
                />校验中{:else if current}<Check
                  size={16}
                />当前模型{:else}<Play size={16} />加载模型{/if}</button
            >
          {:else}<button class="primary-button" onclick={() => download()}
              ><Download size={16} />{job?.phase === "paused" ||
              job?.phase === "error"
                ? "继续下载"
                : "下载模型"}</button
            >{/if}
        </div>
      </section>
      {#if downloads.length}<details class="all-downloads">
          <summary>下载任务 · {downloads.length}</summary
          >{#each downloads as item}<div>
              <span
                >{catalogue.models.find((model) => model.id === item.model_id)
                  ?.name}<small
                  >{downloadPhase[item.phase]} · {fileSize(
                    item.downloaded,
                  )}</small
                ></span
              >{#if ["queued", "downloading", "verifying"].includes(item.phase)}<button
                  class="icon-button"
                  aria-label="暂停下载任务"
                  onclick={() => call("pause_model_download", { id: item.id })}
                  ><Pause size={15} /></button
                >{:else if item.phase !== "completed"}<button
                  class="icon-button"
                  aria-label="继续下载任务"
                  onclick={() => download(item.model_id, item.variant_id)}
                  ><Play size={15} /></button
                >{/if}
            </div>{/each}
        </details>{/if}
    </div>
  {/if}
  {#if error}<div class="inline-error" role="alert">{error}</div>{/if}
</main>

<style>
  .model-library {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    padding: 1rem 1.9rem 1.1rem;
  }
  .library-heading {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1rem;
    gap: 1rem;
  }
  .library-heading h1 {
    font-size: 1.65rem;
  }
  .library-scroll {
    overflow: auto;
    min-height: 0;
    padding-right: 0.35rem;
  }
  .library-picker {
    margin: 0 0 1rem;
  }
  .model-card {
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 1rem;
    padding: 1.2rem;
    box-shadow: var(--shadow);
  }
  .model-heading {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  .model-heading > div {
    flex: 1;
    min-width: 0;
  }
  .model-heading h2 {
    font-size: 1.05rem;
  }
  .model-heading > div > span {
    display: block;
    color: var(--soft);
    font-size: 0.76rem;
    margin-top: 0.3rem;
    overflow-wrap: anywhere;
  }
  .model-icon {
    display: grid;
    place-items: center;
    width: 2.8rem;
    height: 2.8rem;
    border-radius: 0.7rem;
    color: var(--accent);
    background: var(--wash);
    flex-shrink: 0;
  }
  .model-metadata {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
    margin: 1.2rem 0 0;
    padding-top: 1rem;
    border-top: 1px solid var(--line);
  }
  .model-metadata div {
    min-width: 0;
  }
  dt {
    font-size: 0.75rem;
    color: var(--soft);
    margin-bottom: 0.3rem;
  }
  dd {
    margin: 0;
    font-size: 0.88rem;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  dd small {
    display: block;
    font-size: 0.7rem;
    color: var(--faint);
    font-weight: 400;
    margin-top: 0.2rem;
  }
  .source-link {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--accent);
    padding: 0;
    text-align: left;
    overflow-wrap: anywhere;
  }
  .hardware-advice {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.8rem;
    color: var(--soft);
    background: var(--wash);
    padding: 0.8rem;
    border-radius: 0.65rem;
    margin-top: 1.1rem;
  }
  .model-files {
    font-size: 0.76rem;
    color: var(--soft);
    margin-top: 1rem;
  }
  .model-files > div {
    display: flex;
    justify-content: space-between;
    gap: 0.7rem;
    margin-top: 0.65rem;
    align-items: flex-start;
  }
  .model-files > div span {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .model-files small {
    white-space: nowrap;
  }
  .model-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.8rem;
    margin-top: 1.1rem;
    padding-top: 1rem;
    border-top: 1px solid var(--line);
  }
  .downloaded-label {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.76rem;
    color: var(--accent);
  }
  summary {
    cursor: pointer;
  }
  .download-progress {
    margin-top: 1.2rem;
    font-size: 0.74rem;
    color: var(--soft);
  }
  .download-progress > div {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
  }
  .download-progress p {
    color: var(--danger);
    margin-top: 0.5rem;
  }
  progress {
    appearance: none;
    width: 100%;
    height: 0.4rem;
    margin-top: 0.6rem;
    border: 0;
    border-radius: 9px;
    overflow: hidden;
  }
  progress::-webkit-progress-bar {
    background: var(--line);
  }
  progress::-webkit-progress-value {
    background: var(--accent);
    border-radius: 9px;
  }
  .all-downloads {
    font-size: 0.8rem;
    color: var(--soft);
    padding: 1rem 0.4rem;
  }
  .all-downloads > div {
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-bottom: 1px solid var(--line);
    padding: 0.7rem 0;
  }
  .all-downloads small {
    display: block;
    font-size: 0.74rem;
    color: var(--faint);
    margin-top: 0.3rem;
  }
  .library-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 1;
    gap: 0.6rem;
    color: var(--soft);
    font-size: 0.85rem;
  }
  @media (max-width: 480px) {
    .model-library {
      padding-left: 1.1rem;
      padding-right: 1.1rem;
    }
    .model-card {
      padding: 1rem;
    }
    .model-metadata {
      gap: 0.8rem;
    }
    .model-heading h2 {
      font-size: 0.95rem;
    }
    .hardware-advice {
      align-items: flex-start;
    }
    .download-progress > div {
      flex-wrap: wrap;
    }
  }
</style>
