<script lang="ts">
  import {
    ArrowUpRight,
    CheckCircle2,
    Download,
    ExternalLink,
    GitBranch,
    Info,
    LoaderCircle,
    RefreshCw,
  } from "@lucide/svelte";
  import { version as packageVersion } from "../package.json";
  import { getVersion } from "@tauri-apps/api/app";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { author, authorUrl, githubUrl } from "./project";
  import { fileSize } from "./models";
  import type {
    DownloadedUpdate,
    DownloadProgress,
    UpdateInfo,
  } from "./updates";

  let {
    update,
    checking,
    downloading,
    progress,
    downloaded,
    updateError,
    onCheck,
    onDownload,
    onInstall,
  }: {
    update: UpdateInfo | null;
    checking: boolean;
    downloading: boolean;
    progress: DownloadProgress | null;
    downloaded: DownloadedUpdate | null;
    updateError: string;
    onCheck: () => void;
    onDownload: () => void;
    onInstall: () => void;
  } = $props();
  let version = $state(packageVersion);
  void getVersion().then((value) => (version = value));
</script>

<div class="about-view">
  <div class="view-heading">
    <div>
      <h1>关于 BlinkOCR</h1>
      <p>基于 Rust 与 Tauri 构建的 Windows OCR 工具</p>
    </div>
    <Info size={27} class="heading-icon" />
  </div>

  <div class="about-scroll">
    <section class="settings-card about-identity">
      <div class="about-brand">
        <span class="about-mark">B</span>
        <div>
          <h2>BlinkOCR</h2>
          <p>版本 {version}</p>
        </div>
      </div>
      <div class="about-links">
        <div>
          <span>作者</span><button onclick={() => openUrl(authorUrl)}
            >{author} <ExternalLink size={14} /></button
          >
        </div>
        <div>
          <span>开源仓库</span><button onclick={() => openUrl(githubUrl)}
            ><GitBranch size={16} /> GitHub <ArrowUpRight size={14} /></button
          >
        </div>
      </div>
    </section>

    <section class="settings-card">
      <div class="card-heading">
        <div>
          <h2>软件更新</h2>
        </div>
        <button
          class="icon-button"
          title="检查更新"
          aria-label="检查更新"
          disabled={checking || downloading}
          onclick={onCheck}
        >
          <RefreshCw size={18} class={checking ? "spin" : ""} />
        </button>
      </div>
      {#if update}
        <div class="update-version">
          <span>当前版本 <strong>{update.current_version}</strong></span>
          <span>最新版本 <strong>{update.latest_version}</strong></span>
        </div>
        {#if update.available}
          <p class="update-available">发现新版本 {update.latest_version}</p>
          {#if update.notes}<div class="release-notes">{update.notes}</div>{/if}
          {#if update.asset_name}
            <p class="update-package">
              {update.asset_name} / {fileSize(update.asset_size ?? 0)}
            </p>
            {#if downloading && progress}
              <progress value={progress.downloaded} max={progress.total}
              ></progress>
              <p class="update-package">
                {fileSize(progress.downloaded)} / {fileSize(progress.total)}
              </p>
            {/if}
            {#if downloaded}
              <p class="update-file">已下载到：{downloaded.path}</p>
              <button class="primary-button" onclick={onInstall}
                ><CheckCircle2 size={16} />启动安装程序</button
              >
            {:else}<button
                class="primary-button"
                onclick={onDownload}
                disabled={downloading || checking}
              >
                {#if downloading}<LoaderCircle
                    size={16}
                    class="spin"
                  />下载中{:else}<Download size={16} />下载安装包{/if}
              </button>{/if}
          {:else}<p class="update-package">
              该版本尚未提供适合当前运行时的 Windows 安装包。
            </p>{/if}
          <button
            class="about-release-link"
            onclick={() => openUrl(update.release_url)}
            >查看 Release <ArrowUpRight size={14} /></button
          >
        {:else}<p class="update-current">
            <CheckCircle2 size={17} />已是最新版本
          </p>{/if}
      {:else}<p class="update-unchecked">尚未检查更新</p>{/if}
      {#if updateError}<p class="inline-error" role="alert">
          {updateError}
        </p>{/if}
      <button
        class="secondary-button"
        onclick={onCheck}
        disabled={checking || downloading}
      >
        {#if checking}<LoaderCircle
            size={15}
            class="spin"
          />检查中{:else}<RefreshCw size={15} />手动检查更新{/if}
      </button>
    </section>
  </div>
</div>
