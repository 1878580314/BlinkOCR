<script lang="ts">
  import { onMount } from "svelte";
  import { version } from "../package.json";
  import { listen } from "@tauri-apps/api/event";
  import {
    ScanLine,
    ScanText,
    Type,
    Table2,
    Sigma,
    Keyboard,
    Sparkles,
    ImagePlus,
    Settings2,
    History,
    ArrowUpRight,
    Copy,
    Check,
    Download,
    Pin,
    PinOff,
    Minus,
    X,
    Sun,
    Moon,
    ChevronRight,
    LoaderCircle,
    Square,
    FolderOpen,
    SlidersHorizontal,
    Cpu,
    Monitor,
    CheckCircle2,
    AlertCircle,
    RotateCw,
    Power,
    ArrowLeft,
    FileText,
    FilePlus2,
    Info,
  } from "@lucide/svelte";
  import About from "./About.svelte";
  import type {
    UpdateInfo,
    DownloadedUpdate,
    DownloadProgress,
  } from "./updates";
  import { call, copyText, subscribe, subscribeCapture } from "./bridge";
  import {
    initial,
    tasks,
    profileTasks,
    normalizeProfile,
    balancedProfile,
    type Task,
    isBusy,
    combined,
    type Config,
    type Snapshot,
    type OcrRecord,
  } from "./types";
  import { recordedShortcut } from "./hotkeys";
  import ModelLibrary from "./ModelLibrary.svelte";
  import {
    modelProfiles,
    parameterGroups,
    fileSize,
    isClassic,
    groupTitle,
  } from "./models";
  import { hint, hints } from "./hints";

  let snapshot = $state<Snapshot>(structuredClone(initial));
  let view = $state<"studio" | "settings" | "history" | "models">(
    "studio",
  );
  let settingTab = $state<"model" | "recognition" | "experience" | "about">("model");
  let draft = $state<Config>(structuredClone(initial.config));
  let error = $state("");
  let toast = $state("");
  let saving = $state(false);
  let detecting = $state(false);
  let edited = $state("");
  let selected = $state<OcrRecord | null>(null);
  let resultTab = $state<"text" | "supplements">("text");
  let recorder = $state<"trigger" | "quit" | null>(null);
  let recorderTimer: ReturnType<typeof setTimeout>;
  let toastTimer: ReturnType<typeof setTimeout>;
  let lastCopied = -1;
  let pinned = $state(false);
  let updateInfo = $state<UpdateInfo | null>(null);
  let checkingUpdates = $state(false);
  let downloadingUpdate = $state(false);
  let downloadedUpdate = $state<DownloadedUpdate | null>(null);
  let updateProgress = $state<DownloadProgress | null>(null);
  let updateError = $state("");
  let startupUpdateCheck = false;
  let theme = $state(document.documentElement.dataset.theme ?? "light");
  const busy = $derived(isBusy(snapshot.phase));
  const loading = $derived(snapshot.phase === "loading");
  const record = $derived(selected ?? snapshot.result);
  const generation = $derived(record?.generation);
  const tokenSpeed = $derived(
    generation && generation.tokens > 0 && generation.milliseconds > 0
      ? (generation.tokens * 1000) / generation.milliseconds
      : null,
  );
  const profileGroups = parameterGroups(modelProfiles);
  const availableTasks = $derived(profileTasks(snapshot.config.model.profile));
  const draftTasks = $derived(profileTasks(draft.model.profile));
  const draftClassic = $derived(isClassic(draft.model.profile));
  const draftWindows = $derived(draft.model.profile === "windows-ocr");
  const draftPaddle = $derived(draft.model.profile.startsWith("ppocrv6-"));
  const inferenceLabels = { gpu: "GPU", hybrid: "GPU + CPU", cpu: "CPU" };
  const editedCount = $derived(Array.from(edited).length);
  const modelName = $derived(
    (snapshot.loaded_model ?? snapshot.config.model.model_path)
      .split(/[\\/]/)
      .pop()
      ?.replace(/\.gguf$/i, "") || "尚未选择模型",
  );

  function notify(message: string) {
    toast = message;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = ""), 2600);
  }
  async function detectHardware() {
    detecting = true;
    try {
      accept(await call<Snapshot>("refresh_hardware"));
    } catch (failure) {
      error = String(failure);
    } finally {
      detecting = false;
    }
  }
  async function action(command: string, args: Record<string, unknown> = {}) {
    try {
      await call(command, args);
    } catch (failure) {
      notify(String(failure));
    }
  }
  function accept(next: Snapshot) {
    if (next.revision < snapshot.revision) return;
    const changed = next.result?.id !== snapshot.result?.id;
    const completing =
      next.phase === "completed" && snapshot.phase !== "completed";
    if (changed) {
      selected = null;
      resultTab = "text";
    }
    if (!selected && (changed || isBusy(next.phase) || completing))
      edited = next.result?.text ?? "";
    if (
      next.config.ui.always_on_top !== snapshot.config.ui.always_on_top ||
      snapshot.revision === 0
    )
      pinned = next.config.ui.always_on_top;
    snapshot = next;
    if (view !== "settings") draft = structuredClone(next.config);
    if (!startupUpdateCheck && next.revision > 0) {
      startupUpdateCheck = true;
      if (next.config.ui.auto_check_updates) void checkUpdates();
    }
    if (
      completing &&
      next.config.ui.auto_copy &&
      next.result &&
      next.result.id !== lastCopied
    ) {
      lastCopied = next.result.id;
      void copyText(combined(next.result.text, next.result.supplements))
        .then(() => notify("识别结果已复制"))
        .catch((failure) => notify(String(failure)));
    }
  }
  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let unlistenCapture: (() => void) | undefined;
    let unlistenUpdate: (() => void) | undefined;
    void listen<DownloadProgress>("update-download-progress", ({ payload }) => {
      updateProgress = payload;
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlistenUpdate = cleanup;
    });
    void subscribeCapture(() => {
      void capture();
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlistenCapture = cleanup;
    });
    void subscribe(accept)
      .then((cleanup) => {
        if (disposed) {
          cleanup();
          return;
        }
        unlisten = cleanup;
      })
      .catch((failure) => {
        error = String(failure);
      });
    return () => {
      disposed = true;
      unlisten?.();
      unlistenCapture?.();
      unlistenUpdate?.();
      clearTimeout(toastTimer);
      clearTimeout(recorderTimer);
      if (recorder) void call("set_hotkey_recording", { recording: false });
    };
  });

  async function checkUpdates() {
    if (checkingUpdates || downloadingUpdate) return;
    checkingUpdates = true;
    updateError = "";
    downloadedUpdate = null;
    try {
      updateInfo = await call<UpdateInfo>("check_updates");
    } catch (failure) {
      updateError = String(failure);
    } finally {
      checkingUpdates = false;
    }
  }

  async function downloadLatest() {
    if (downloadingUpdate || !updateInfo?.asset_name) return;
    downloadingUpdate = true;
    updateProgress = null;
    updateError = "";
    try {
      downloadedUpdate = await call<DownloadedUpdate>("download_update");
    } catch (failure) {
      updateError = String(failure);
    } finally {
      downloadingUpdate = false;
    }
  }

  async function installLatest() {
    try {
      await call("install_update");
    } catch (failure) {
      updateError = String(failure);
    }
  }

  async function persistEdits() {
    if (record && !busy && (selected || edited !== record.text)) {
      accept(
        await call<Snapshot>("update_result_text", {
          id: record.id,
          text: edited,
        }),
      );
    }
  }
  async function capture() {
    try {
      await persistEdits();
      selected = null;
      view = "studio";
      await call("capture");
    } catch (failure) {
      notify(String(failure));
    }
  }
  async function setContinuous(enabled: boolean) {
    try {
      await persistEdits();
      const config = structuredClone($state.snapshot(snapshot.config));
      config.ocr.continuous_recognition = enabled;
      accept(await call<Snapshot>("save_settings", { config }));
    } catch (failure) {
      notify(String(failure));
    }
  }
  async function newResult() {
    try {
      selected = null;
      accept(await call<Snapshot>("clear_result"));
    } catch (failure) {
      notify(String(failure));
    }
  }

  function navigate(next: typeof view) {
    void stopRecording();
    error = "";
    if (next === "settings")
      draft = structuredClone($state.snapshot(snapshot.config));
    view = next;
  }
  function localModel(profile: string) {
    navigate("settings");
    settingTab = "model";
    if (draft.model.profile !== profile) {
      draft.model.model_path = "";
      draft.model.mmproj_path = "";
    }
    draft.model.profile = profile;
    normalizeProfile(draft);
    balancedProfile(draft);
  }
  async function chooseTask(task: Task) {
    if (busy || loading) return;
    const config = structuredClone($state.snapshot(snapshot.config));
    if (config.ocr.prompt === task.prompt) return;
    config.ocr.prompt = task.prompt;
    try {
      accept(await call<Snapshot>("save_settings", { config }));
    } catch (failure) {
      notify(String(failure));
    }
  }
  async function openImage() {
    try {
      await persistEdits();
    } catch (failure) {
      notify(String(failure));
      return;
    }
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({
      multiple: false,
      filters: [
        { name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "bmp"] },
      ],
    });
    if (typeof path === "string") {
      selected = null;
      view = "studio";
      await action("recognize_file", { path });
    }
  }
  async function pickFile(target: "model" | "mmproj" | "backend") {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({
      multiple: false,
      filters: [
        {
          name:
            target === "backend"
              ? "本地推理程序"
              : draftPaddle
                ? "ONNX 模型"
                : "GGUF 模型",
          extensions:
            target === "backend" ? ["exe"] : draftPaddle ? ["onnx"] : ["gguf"],
        },
      ],
    });
    if (typeof path !== "string") return;
    if (target === "model") draft.model.model_path = path;
    else if (target === "mmproj") draft.model.mmproj_path = path;
    else if (draft.model.profile === "teleocr")
      draft.server.teleocr_server_bin = path;
    else draft.server.llama_server_bin = path;
  }
  async function save() {
    if (saving || recorder) return;
    saving = true;
    error = "";
    try {
      accept(
        await call<Snapshot>("save_settings", {
          config: $state.snapshot(draft),
        }),
      );
      view = "studio";
      notify("设置已保存");
    } catch (failure) {
      error = String(failure);
    } finally {
      saving = false;
    }
  }
  async function startRecording(field: "trigger" | "quit") {
    await stopRecording();
    try {
      await call("set_hotkey_recording", { recording: true });
      recorder = field;
      recorderTimer = setTimeout(() => void stopRecording(), 15000);
    } catch (failure) {
      error = String(failure);
    }
  }
  async function stopRecording() {
    if (!recorder) return;
    recorder = null;
    clearTimeout(recorderTimer);
    try {
      await call("set_hotkey_recording", { recording: false });
    } catch (failure) {
      error = String(failure);
    }
  }
  function keydown(event: KeyboardEvent) {
    if (recorder) {
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape") {
        void stopRecording();
        return;
      }
      const shortcut = recordedShortcut(event);
      if (shortcut) {
        draft.hotkey[recorder] = shortcut;
        void stopRecording();
      }
      return;
    }
    if (
      (event.ctrlKey || event.metaKey) &&
      event.shiftKey &&
      event.code === "KeyC"
    ) {
      event.preventDefault();
      void copy();
    } else if (event.key === "Escape") {
      if (busy) void action("cancel");
      else if (view !== "studio") navigate("studio");
    }
  }
  async function copy() {
    if (!edited && !record?.supplements.length) return;
    try {
      await copyText(combined(edited, record?.supplements ?? []));
      notify("已复制到剪贴板");
    } catch (failure) {
      notify(String(failure));
    }
  }
  async function exportResult() {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({
      defaultPath: `OCR-${new Date().toISOString().slice(0, 10)}.txt`,
      filters: [{ name: "文本", extensions: ["txt", "md"] }],
    });
    if (path) {
      await action("export_text", {
        path,
        text: combined(edited, record?.supplements ?? []),
      });
      notify("文本已保存");
    }
  }
  async function windowAction(name: "minimize" | "hide" | "pin") {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const window = getCurrentWindow();
    if (name === "pin") {
      pinned = !pinned;
      await window.setAlwaysOnTop(pinned);
    } else await window[name]();
  }
  function toggleTheme() {
    theme = theme === "light" ? "dark" : "light";
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("ocr-theme", theme);
  }
  function openRecord(item: OcrRecord) {
    selected = item;
    edited = item.text;
    resultTab = "text";
    view = "studio";
  }
  function time(milliseconds: number) {
    return `${(milliseconds / 1000).toFixed(1)} 秒`;
  }
</script>

<svelte:window onkeydown={keydown} onblur={() => void stopRecording()} />

<div class="app-shell">
  <header class="titlebar">
    <div class="drag-area" data-tauri-drag-region>
      <ScanLine size={17} /><span data-tauri-drag-region>BlinkOCR</span><span
        class="version"
        data-tauri-drag-region>{version}</span
      >
    </div>
    <button
      class="window-button"
      aria-label={pinned ? "取消置顶" : "窗口置顶"}
      title={pinned ? "取消置顶" : "窗口置顶"}
      onclick={() => windowAction("pin")}
      >{#if pinned}<PinOff size={14} />{:else}<Pin size={14} />{/if}</button
    >
    <button
      class="window-button"
      aria-label="最小化"
      onclick={() => windowAction("minimize")}><Minus size={16} /></button
    >
    <button
      class="window-button close-button"
      aria-label="隐藏到托盘"
      title="隐藏到托盘"
      onclick={() => windowAction("hide")}><X size={16} /></button
    >
  </header>

  <nav class="navigation" aria-label="主要导航">
    <div class="nav-group">
      <button
        class:active={view === "studio"}
        onclick={() => navigate("studio")}><ScanText size={16} />识别</button
      >
      <button
        class:active={view === "history"}
        onclick={() => navigate("history")}
        ><History size={16} />本次记录{#if snapshot.history.length}<span
            class="count">{snapshot.history.length}</span
          >{/if}</button
      >
      <button
        class:active={view === "models"}
        onclick={() => navigate("models")}
        ><Cpu size={16} /><span>模型库</span></button
      >
    </div>
    <div class="nav-tools">
      <button
        class="icon-button"
        title="切换主题"
        aria-label="切换主题"
        onclick={toggleTheme}
        >{#if theme === "dark"}<Sun size={17} />{:else}<Moon
            size={17}
          />{/if}</button
      ><button
        class:active={view === "settings"}
        class="settings-nav"
        aria-label="设置"
        onclick={() => navigate("settings")}
        ><Settings2 size={16} /><span>设置</span></button
      >
    </div>
  </nav>

  {#if view === "studio"}
    <main class="workspace" class:has-result={record !== null}>
      {#if !snapshot.ready && !loading}
        <button class="setup-banner" onclick={() => navigate("settings")}
          ><Cpu size={21} /><span
            ><strong>配置模型</strong><small>{snapshot.message}</small></span
          ><ArrowUpRight size={19} /></button
        >
      {/if}

      <section class="capture-panel">
        <div class="capture-toolbar">
          {#if availableTasks.length > 4}
            <label class="mode-picker" use:hint={hints.mode}
              >识别模式<select
                aria-label="识别模式"
                value={snapshot.config.ocr.prompt}
                disabled={busy || loading}
                onchange={(event) => {
                  const task = availableTasks.find(
                    (task) => task.prompt === event.currentTarget.value,
                  );
                  if (task) void chooseTask(task);
                }}
                >{#each availableTasks as task}<option value={task.prompt}
                    >{task.name}</option
                  >{/each}</select
              ></label
            >
          {:else}<div class="segmented" aria-label="识别模式">
              {#each availableTasks as task}<button
                  use:hint={hints[task.id]}
                  class:chosen={snapshot.config.ocr.prompt === task.prompt}
                  disabled={busy || loading}
                  onclick={() => chooseTask(task)}
                >
                  {#if task.id === "general"}<Sparkles
                      size={14}
                    />{:else if task.id === "text"}<Type
                      size={14}
                    />{:else if task.id === "table"}<Table2
                      size={14}
                    />{:else if task.id === "formula"}<Sigma
                      size={14}
                    />{:else}<FileText size={14} />{/if}{task.name}
                </button>{/each}
            </div>{/if}
        </div>
        <div class="capture-actions">
          <button
            class="capture-button"
            disabled={!snapshot.ready}
            onclick={capture}
            ><ScanLine size={22} /><span
              >{busy ? "取消并重新框选" : "框选识别"}</span
            >{#if snapshot.config.hotkey.enabled}<kbd
                >{snapshot.config.hotkey.trigger}</kbd
              >{/if}</button
          ><button
            class="image-button"
            disabled={!snapshot.ready || busy}
            onclick={openImage}
            ><ImagePlus size={19} /><span>选择图片</span></button
          >{#if busy}<button
              class="stop-button"
              title="取消识别"
              aria-label="取消识别"
              onclick={() => action("cancel")}><Square size={16} /></button
            >{/if}
        </div>
        <div class="recognition-options">
          <label class="continuous-option" use:hint={hints.continuous}
            ><input
              type="checkbox"
              class="toggle"
              role="switch"
              aria-label="连续识别"
              checked={snapshot.config.ocr.continuous_recognition}
              disabled={busy || loading}
              onchange={(event) => setContinuous(event.currentTarget.checked)}
            /><span>连续识别</span></label
          >
          {#if snapshot.config.ocr.continuous_recognition}<span
              class="append-hint">后续结果追加到当前内容</span
            >{/if}
        </div>
      </section>

      <section class="result-panel" aria-label="识别结果">
        <div class="result-toolbar">
          <div class="result-tabs">
            <button
              class:chosen={resultTab === "text"}
              onclick={() => (resultTab = "text")}
              ><FileText size={15} />识别结果</button
            >{#if record?.supplements.length}<button
                class:chosen={resultTab === "supplements"}
                onclick={() => (resultTab = "supplements")}
                >补充内容<span class="count">{record.supplements.length}</span
                ></button
              >{/if}
          </div>
          <div class="result-tools">
            {#if record}<button
                class="new-result-button"
                disabled={busy}
                aria-label="新建结果"
                title="新建结果"
                onclick={newResult}
                ><FilePlus2 size={16} /><span>新建</span></button
              >{/if}
            {#if busy}<span class="live-indicator"
                ><LoaderCircle size={13} class="spin" />{snapshot.phase ===
                "postprocessing"
                  ? "补充识别中"
                  : "识别中"}</span
              >{:else if edited && edited !== record?.text}<span
                class="edited-badge">已编辑</span
              >{/if}<button
              class="icon-button"
              aria-label="保存文本"
              title="保存文本"
              disabled={!edited}
              onclick={exportResult}><Download size={16} /></button
            >
          </div>
        </div>
        {#if resultTab === "supplements" && record}
          <div class="supplement-list">
            {#each record.supplements as item}<article
                class="supplement"
                class:conflict={item.conflict}
              >
                <div>
                  {#if item.conflict}<AlertCircle size={14} />{:else}<Sparkles
                      size={14}
                    />{/if}<span
                    >{item.conflict ? "差异待核对" : "补充识别"}</span
                  ><small>{item.source}</small>
                </div>
                <p>{item.text}</p>
              </article>{/each}
          </div>
        {:else if record && (edited || busy)}
          <textarea
            class="result-editor"
            aria-label="识别文本"
            bind:value={edited}
            readonly={busy && !selected}
            spellcheck="false"
            placeholder="正在识别…"></textarea>
        {:else}
          <div class="empty-result">
            <FileText size={42} class="empty-icon" />
            <h2>{loading ? "正在加载模型" : "暂无识别结果"}</h2>
            {#if loading}<p>加载完成后可开始识别。</p>{/if}
          </div>
        {/if}
        <div class="result-bottom">
          <span
            >{#if edited}<strong>{editedCount.toLocaleString()}</strong>
              字符{#if record?.pages}<span class="meta-dot" aria-hidden="true"></span
                >{record.pages} 张{/if}{#if record?.elapsed_ms}<span
                  class="meta-dot" aria-hidden="true"></span
                >{time(record.elapsed_ms)}{/if}{:else}0 字符{/if}</span
          ><button
            class="copy-button"
            disabled={!edited && !record?.supplements.length}
            onclick={copy}
            ><Copy size={15} />复制全文<span class="shortcut-hint"
              >Ctrl ⇧ C</span
            ></button
          >
        </div>
      </section>
      {#if snapshot.phase === "error"}<div class="result-notice error">
          <AlertCircle size={14} /><span>{snapshot.message}</span>
        </div>
      {:else if record?.warnings.length}<div class="result-notice">
          <AlertCircle size={14} /><span>{record.warnings.join("；")}</span>
        </div>{/if}
    </main>
  {:else if view === "models"}
    <ModelLibrary
      {snapshot}
      onLoaded={(state) => {
        accept(state);
        view = "studio";
      }}
      onLocal={localModel}
    />
  {:else if view === "settings"}
    <main class="settings-view">
      <div class="view-heading">
        <div>
          <h1>设置</h1>
        </div>
        <button
          class="icon-button"
          aria-label="返回识别"
          onclick={() => navigate("studio")}><ArrowLeft size={19} /></button
        >
      </div>
      <div class="settings-tabs" aria-label="设置分类">
        <button
          class:chosen={settingTab === "model"}
          onclick={() => {
            void stopRecording();
            settingTab = "model";
          }}><Cpu size={16} />模型</button
        ><button
          class:chosen={settingTab === "recognition"}
          onclick={() => {
            void stopRecording();
            settingTab = "recognition";
          }}><SlidersHorizontal size={16} />识别</button
        ><button
          class:chosen={settingTab === "experience"}
          onclick={() => (settingTab = "experience")}
          ><Monitor size={16} />体验</button
        ><button
          class:chosen={settingTab === "about"}
          onclick={() => {
            void stopRecording();
            settingTab = "about";
          }}
          ><Info size={16} />关于{#if updateInfo?.available}<span
              class="update-dot inline"
            ></span>{/if}</button
        >
      </div>
      {#if settingTab === "about"}
        <About
          update={updateInfo}
          checking={checkingUpdates}
          downloading={downloadingUpdate}
          progress={updateProgress}
          downloaded={downloadedUpdate}
          {updateError}
          onCheck={() => void checkUpdates()}
          onDownload={() => void downloadLatest()}
          onInstall={() => void installLatest()}
        />
      {:else}
      <div class="settings-scroll">
        {#if settingTab === "model"}
          <section class="settings-card">
            <div class="card-heading">
              <div>
                <h2>本地模型</h2>
                <p>
                  {draftWindows
                    ? "使用 Windows 已安装的 OCR 语言。"
                    : draftPaddle
                      ? "选择配套的检测与识别 ONNX 文件。"
                      : "选择主模型与对应的视觉投影文件。"}
                </p>
              </div>
            </div>
            <button class="secondary-button" onclick={() => navigate("models")}
              ><Download size={16} />选择模型</button
            >
            <label class="field" use:hint={hints.model}
              >模型类型<select
                aria-label="模型类型"
                bind:value={draft.model.profile}
                onchange={(event) => {
                  draft.model.profile = event.currentTarget.value;
                  normalizeProfile(draft);
                  balancedProfile(draft);
                }}
              >
                {#each profileGroups as group}<optgroup
                    label={groupTitle(group.parameters)}
                    >{#each group.models as model}<option value={model.id}
                        >{model.name}</option
                      >{/each}</optgroup
                  >{/each}<option value="other">其他兼容模型</option>
              </select></label
            >
            {#if draftWindows}
              <label class="field" use:hint={hints.language}
                >识别语言<select
                  aria-label="识别语言"
                  bind:value={draft.ocr.native_language}
                  ><option value="auto">自动（Windows 用户语言）</option
                  >{#each snapshot.native_languages ?? [] as language}<option
                      value={language.tag}>{language.name}</option
                    >{/each}</select
                ></label
              >
              <button
                class="secondary-button"
                onclick={() => action("open_ocr_languages")}
                >打开系统语言设置</button
              >
            {:else}
              <label
                class="field"
                use:hint={draftPaddle ? hints.detection : hints.weights}
                >{draftPaddle ? "检测模型" : "主模型"}
                <div class="path-field">
                  <input
                    aria-label={draftPaddle ? "检测模型" : "主模型"}
                    bind:value={draft.model.model_path}
                    placeholder={draftPaddle
                      ? "选择检测 ONNX 文件"
                      : "选择 GGUF 主模型"}
                  /><button
                    title={draftPaddle ? "选择检测模型" : "选择主模型"}
                    aria-label={draftPaddle ? "选择检测模型" : "选择主模型"}
                    onclick={() => pickFile("model")}
                    ><FolderOpen size={17} /></button
                  >
                </div></label
              >
              {#if draft.model.profile !== "teleocr"}<label
                  class="field"
                  use:hint={draftPaddle ? hints.recognition : hints.projector}
                  >{draftPaddle ? "识别模型" : "视觉投影"}
                  <div class="path-field">
                    <input
                      aria-label={draftPaddle ? "识别模型" : "视觉投影"}
                      bind:value={draft.model.mmproj_path}
                      placeholder={draftPaddle
                        ? "选择识别 ONNX 文件"
                        : "选择匹配的 mmproj 文件"}
                    /><button
                      title={draftPaddle ? "选择识别模型" : "选择视觉投影"}
                      aria-label={draftPaddle ? "选择识别模型" : "选择视觉投影"}
                      onclick={() => pickFile("mmproj")}
                      ><FolderOpen size={17} /></button
                    >
                  </div></label
                >{/if}
              {#if !draftClassic}<label class="field" use:hint={hints.backend}
                  >推理程序
                  <div class="path-field">
                    <input
                      aria-label="推理程序"
                      value={draft.model.profile === "teleocr"
                        ? draft.server.teleocr_server_bin
                        : draft.server.llama_server_bin}
                      oninput={(event) => {
                        if (draft.model.profile === "teleocr")
                          draft.server.teleocr_server_bin =
                            event.currentTarget.value;
                        else
                          draft.server.llama_server_bin =
                            event.currentTarget.value;
                      }}
                      placeholder={draft.model.profile === "teleocr"
                        ? "teleocr.exe"
                        : "llama-server.exe"}
                    /><button
                      title="选择推理程序"
                      aria-label="选择推理程序"
                      onclick={() => pickFile("backend")}
                      ><FolderOpen size={17} /></button
                    >
                  </div></label
                >{/if}
            {/if}
          </section>
          {#if !draftWindows}<section class="settings-card">
              <div class="card-heading">
                <div>
                  <h2>运行参数</h2>
                  <p>模型参数变更后会自动重新加载。</p>
                </div>
              </div>
              <div class="field-grid">
                <label class="field" use:hint={hints.device}
                  >推理设备<select
                    aria-label="推理设备"
                    bind:value={draft.model.device_mode}
                    ><option value="auto">自动（GPU 优先）</option><option
                      value="cpu">CPU</option
                    ></select
                  ></label
                >
                <label class="field" use:hint={hints.threads}
                  >CPU 线程数<input
                    aria-label="CPU 线程数"
                    type="number"
                    min="1"
                    max="256"
                    bind:value={draft.model.threads}
                  /></label
                >
                {#if !draftClassic}
                  <label class="field" use:hint={hints.context}
                    >上下文长度<input
                      aria-label="上下文长度"
                      type="number"
                      min="1024"
                      max="262144"
                      step="1024"
                      bind:value={draft.model.context_size}
                    /></label
                  >
                  {#if draft.model.profile !== "teleocr"}<label
                      class="field"
                      use:hint={hints.imageTokens}
                      >图像最小 token 数<select
                        aria-label="图像最小 token 数"
                        bind:value={draft.model.image_min_tokens}
                        ><option value={0}>模型默认</option
                        >{#each [128, 256, 512, 1024, 2048] as count}<option
                            value={count}>{count}</option
                          >{/each}</select
                      ></label
                    >{/if}
                  <label class="field" use:hint={hints.port}
                    >本地服务端口<input
                      aria-label="本地服务端口"
                      type="number"
                      min="1"
                      max="65535"
                      bind:value={draft.server.port}
                    /></label
                  >
                  <label class="field" use:hint={hints.timeout}
                    >模型加载超时（秒）<input
                      aria-label="模型加载超时（秒）"
                      type="number"
                      min="5"
                      max="900"
                      bind:value={draft.server.startup_timeout}
                    /></label
                  >
                {/if}
              </div>
            </section>{/if}
          <section class="settings-card hardware-card">
            <div class="card-heading">
              <div>
                <h2>硬件</h2>
                <p>自动模式按模型、上下文和可用显存分配推理设备。</p>
              </div>
              <button
                class="icon-button"
                title="重新检测硬件"
                aria-label="重新检测硬件"
                disabled={detecting || loading}
                onclick={detectHardware}
                ><RotateCw size={17} class={detecting ? "spin" : ""} /></button
              >
            </div>
            {#if snapshot.hardware}
              <div class="hardware-row">
                <span>CPU</span><strong
                  >{snapshot.hardware.cpu_name
                    ? `${snapshot.hardware.cpu_name} / `
                    : ""}{snapshot.hardware.logical_cores} 个逻辑核心</strong
                >
              </div>
              {#if snapshot.hardware.available_ram_bytes != null && snapshot.hardware.total_ram_bytes != null}<div
                  class="hardware-row"
                >
                  <span>可用内存 / 总内存</span><strong
                    >{fileSize(snapshot.hardware.available_ram_bytes)} / {fileSize(
                      snapshot.hardware.total_ram_bytes,
                    )}</strong
                  >
                </div>{/if}
              {#each snapshot.hardware.gpus as gpu}<div class="hardware-gpu">
                  <strong>{gpu.name}</strong><span
                    >可用显存 {fileSize(gpu.free_bytes)} / {fileSize(
                      gpu.total_bytes,
                    )}</span
                  >{#if !gpu.usable}<small>当前推理程序使用 CPU</small>{/if}
                </div>{:else}<div class="hardware-row">
                  <span>GPU</span><strong>未检测到可用设备</strong>
                </div>{/each}
              {#if snapshot.inference}<p class="hardware-plan">
                  {snapshot.inference.reason}
                </p>{/if}
            {:else}<p class="hardware-plan">加载模型时检测硬件。</p>{/if}
          </section>
        {:else if settingTab === "recognition"}
          <section class="settings-card">
            <div class="card-heading">
              <div>
                <h2>识别偏好</h2>
                <p>按内容选择合适的输出与图片尺寸。</p>
              </div>
            </div>
            {#if !draftClassic}<label class="field" use:hint={hints.mode}
                >默认识别模式<select bind:value={draft.ocr.prompt}
                  >{#each draftTasks as task}<option value={task.prompt}
                      >{task.name}</option
                    >{/each}</select
                ></label
              >{/if}
            <div class="field-grid">
              {#if !draftClassic}<div class="field" use:hint={hints.output}>
                  <span>最大输出长度</span><label class="continuous-option"
                    ><input
                      type="checkbox"
                      class="toggle"
                      role="switch"
                      aria-label="不限输出长度"
                      checked={draft.ocr.max_tokens < 0}
                      onchange={(event) =>
                        (draft.ocr.max_tokens = event.currentTarget.checked
                          ? -1
                          : 4096)}
                    /><span>不限</span></label
                  >{#if draft.ocr.max_tokens > 0}<input
                      type="number"
                      min="1"
                      aria-label="最大输出长度"
                      bind:value={draft.ocr.max_tokens}
                    />{/if}
                </div>{/if}
              <label class="field" use:hint={hints.dimension}
                >图片最长边<select bind:value={draft.ocr.max_image_dimension}
                  ><option value={1024}>1024 像素</option><option value={1536}
                    >1536 像素</option
                  ><option value={2048}>2048 像素</option><option value={3072}
                    >3072 像素</option
                  >{#if draftWindows}<option value={4000}>4000 像素</option
                    >{/if}<option value={4096}>4096 像素</option></select
                ></label
              >
            </div>
          </section>
          <section class="settings-card xiaomi-card">
            <div class="feature-icon"><Table2 size={22} /></div>
            <div class="toggle-row">
              <div>
                <h2>Xiaomi 表格增强</h2>
                <p>额外智能检查表格的识别遗漏</p>
              </div>
              <input
                class="toggle"
                type="checkbox"
                role="switch"
                aria-label="Xiaomi 表格增强"
                disabled={draft.model.profile !== "xiaomi-ocr"}
                bind:checked={draft.ocr.xiaomi_table_postprocess}
              />
            </div>
            <p class="feature-description">
              默认关闭。适用于Xiaomi-OCR的通用识别模式，会增加识别时间
            </p>
            {#if draft.model.profile !== "xiaomi-ocr"}<p class="subtle-hint">
                选择 Xiaomi-OCR 模型后可启用。
              </p>{/if}
          </section>
        {:else}
          <section class="settings-card">
            <div class="card-heading">
              <div>
                <h2>快捷键</h2>
                <p>点击按键框后按键录入，Esc 取消。</p>
              </div>
            </div>
            <div class="toggle-row" use:hint={hints.hotkeys}>
              <div>
                <strong>启用全局快捷键</strong>
                <p>在其他应用中也能随时框选。</p>
              </div>
              <input
                type="checkbox"
                class="toggle"
                role="switch"
                aria-label="启用全局快捷键"
                bind:checked={draft.hotkey.enabled}
              />
            </div>
            <div class="shortcut-row" use:hint={hints.captureKey}>
              <span>框选识别</span><button
                class="key-recorder"
                class:recording={recorder === "trigger"}
                aria-label="录入框选快捷键"
                onclick={() => startRecording("trigger")}
                disabled={!draft.hotkey.enabled}
                >{#if recorder === "trigger"}<span class="record-dot"
                  ></span>请按下快捷键…{:else}<Keyboard size={16} /><kbd
                    >{draft.hotkey.trigger}</kbd
                  >{/if}</button
              >
            </div>
            <div class="shortcut-row" use:hint={hints.quitKey}>
              <span>退出应用</span><button
                class="key-recorder"
                class:recording={recorder === "quit"}
                aria-label="录入退出快捷键"
                onclick={() => startRecording("quit")}
                disabled={!draft.hotkey.enabled}
                >{#if recorder === "quit"}<span class="record-dot"
                  ></span>请按下快捷键…{:else}<Keyboard size={16} /><kbd
                    >{draft.hotkey.quit || "点击录入"}</kbd
                  >{/if}</button
              >
            </div>
            {#if recorder}<p class="subtle-hint">
                支持单键或组合键 / Esc 取消
              </p>{/if}
          </section>
          <section class="settings-card">
            <div class="card-heading">
              <div>
                <h2>行为设置</h2>
              </div>
            </div>
            <div class="toggle-row" use:hint={hints.copy}>
              <div>
                <strong>完成后自动复制</strong>
                <p>识别完成后复制全部结果。</p>
              </div>
              <input
                type="checkbox"
                class="toggle"
                role="switch"
                aria-label="完成后自动复制"
                bind:checked={draft.ui.auto_copy}
              />
            </div>
            <div class="toggle-row" use:hint={hints.pin}>
              <div>
                <strong>窗口始终置顶</strong>
                <p>使窗口显示在其他窗口上方。</p>
              </div>
              <input
                type="checkbox"
                class="toggle"
                role="switch"
                aria-label="窗口始终置顶"
                bind:checked={draft.ui.always_on_top}
              />
            </div>
            <div class="toggle-row">
              <div>
                <strong>启动时检查更新</strong>
                <p>从 GitHub Releases 检查最新版本。</p>
              </div>
              <input
                class="toggle"
                type="checkbox"
                role="switch"
                aria-label="启动时检查更新"
                bind:checked={draft.ui.auto_check_updates}
              />
            </div>
          </section>
          <button class="quit-button" onclick={() => action("quit")}
            ><Power size={15} />退出 BlinkOCR</button
          >
        {/if}
      </div>
      {#if error}<div class="inline-error" role="alert">
          <AlertCircle size={15} />{error}
        </div>{/if}
      <div class="settings-actions">
        <button class="secondary-button" onclick={() => navigate("studio")}
          >取消</button
        >
        <button
          class="primary-button"
          disabled={saving || busy || loading || recorder !== null}
          onclick={save}
          >{#if saving}<LoaderCircle size={15} class="spin" />{:else}<Check
              size={16}
            />{/if}保存设置</button
        >
      </div>
      {/if}
    </main>
  {:else}
    <main class="history-view">
      <div class="view-heading">
        <div>
          <h1>识别记录</h1>
          <p>本次会话的最近 20 条记录。</p>
        </div>
        <History size={28} class="heading-icon" />
      </div>
      <div class="history-list">
        {#each snapshot.history as item}<button
            class="history-item"
            onclick={() => openRecord(item)}
            ><span class="history-icon"><FileText size={20} /></span><span
              class="history-content"
              ><strong>{item.text.slice(0, 45) || "识别结果"}</strong><small
                >{tasks.find((task) => task.id === item.task)?.name ??
                  "通用"}<span class="meta-dot" aria-hidden="true"></span>{new Date(
                  item.created_at,
                ).toLocaleTimeString("zh-CN", {
                  hour: "2-digit",
                  minute: "2-digit",
                })}<span class="meta-dot" aria-hidden="true"></span>{item.text.length} 字符</small
              ></span
            ><ChevronRight size={17} /></button
          >{:else}<div class="history-empty">
            <History size={38} />
            <h2>暂无记录</h2>

            <button class="secondary-button" onclick={() => navigate("studio")}
              >返回识别<ArrowUpRight size={15} /></button
            >
          </div>{/each}
      </div>
    </main>
  {/if}

  <footer class="statusbar">
    <div
      class="engine-summary"
      title={snapshot.loaded_model ?? snapshot.config.model.model_path}
    >
      <span
        class="status-dot"
        class:offline={!snapshot.ready}
        class:loading-dot={loading}
      ></span>
      <span class="model-label"
        >{snapshot.ready
          ? "当前加载模型："
          : loading
            ? "正在加载模型："
            : "模型未加载"}</span
      >
      {#if snapshot.ready || loading}<span class="model-name">{modelName}</span
        >{/if}
    </div>
    <div class="engine-status">
      {#if view === "studio" && !busy && !loading && tokenSpeed !== null && Number.isFinite(tokenSpeed)}<span
          class="token-speed"
          title={generation?.includes_prefill
            ? "按模型返回的 token 数和处理耗时计算，包含图像处理与预填充。连续识别按累计数据计算。"
            : "按模型返回的生成 token 数和生成耗时计算。连续识别按累计数据计算。"}
          >平均 {tokenSpeed.toFixed(1)} token/s</span
        >{/if}
      {#if snapshot.ready && snapshot.inference && !busy}<span
          class="device-tag"
          title={snapshot.inference.reason}
          >{inferenceLabels[snapshot.inference.mode]}</span
        >{/if}
      {#if loading || busy}<LoaderCircle size={14} class="spin" /><span
          >{snapshot.message}</span
        >
      {:else if snapshot.hotkey_error}<button
          title={snapshot.hotkey_error}
          onclick={() => {
            navigate("settings");
            settingTab = "experience";
          }}><Keyboard size={14} />快捷键被占用</button
        >
      {:else if snapshot.ready}<span>就绪</span>
      {:else}<button onclick={() => action("retry_engine")}
          ><RotateCw size={14} />重新加载</button
        >{/if}
    </div>
  </footer>
  {#if toast}<div class="toast" role="status">
      <CheckCircle2 size={17} />{toast}
    </div>{/if}
</div>
