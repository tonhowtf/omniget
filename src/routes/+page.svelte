<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { goto } from "$app/navigation";
  import Mascot from "$components/mascot/Mascot.svelte";
  import SupportedServices from "$components/services/SupportedServices.svelte";
  import { getDownloads, removeDownload, type GenericDownloadItem } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";

  type PlatformInfo = {
    platform: string;
    supported: boolean;
    content_id: string | null;
    content_type: string | null;
  };

  type DownloadStarted = {
    id: number;
    title: string;
  };

  type OmniState =
    | { kind: "idle" }
    | { kind: "detecting" }
    | { kind: "detected"; info: PlatformInfo }
    | { kind: "unsupported" }
    | { kind: "preparing"; platform: string }
    | { kind: "downloading"; trackingId: number; platform: string; title: string }
    | { kind: "complete"; title: string; filePath?: string; platform: string; fileCount?: number }
    | { kind: "error"; message: string; originalUrl: string; platform: string; trackingId?: number };

  let url = $state("");
  let savedUrl = $state("");
  let omniState = $state<OmniState>({ kind: "idle" });
  let debounceTimer = $state<ReturnType<typeof setTimeout> | null>(null);

  const STALL_THRESHOLD = 30_000;

  let downloads = $derived(getDownloads());

  let trackedDownload = $derived.by((): GenericDownloadItem | undefined => {
    if (omniState.kind !== "downloading") return undefined;
    const item = downloads.get(omniState.trackingId);
    if (item && item.kind === "generic") return item;
    return undefined;
  });

  $effect(() => {
    if (omniState.kind !== "downloading") return;
    const item = downloads.get(omniState.trackingId);
    if (!item || item.kind !== "generic") return;

    if (item.status === "complete") {
      omniState = {
        kind: "complete",
        title: item.name,
        filePath: item.filePath,
        platform: omniState.platform,
        fileCount: item.fileCount,
      };
    } else if (item.status === "error") {
      omniState = {
        kind: "error",
        message: item.error ?? $t("omnibox.error"),
        originalUrl: savedUrl,
        platform: omniState.platform,
        trackingId: omniState.trackingId,
      };
    }
  });

  let mascotEmotion = $derived.by((): "idle" | "downloading" | "error" | "stalled" => {
    if (omniState.kind === "error") return "error";
    if (omniState.kind === "downloading" || omniState.kind === "preparing") return "downloading";

    let hasCourseError = false;
    let hasStalled = false;
    let hasDownloading = false;

    for (const item of downloads.values()) {
      if (item.kind === "course" && item.status === "error") {
        hasCourseError = true;
      } else if (item.status === "downloading") {
        hasDownloading = true;
        if (item.kind === "course" && item.speed === 0 && (Date.now() - item.startedAt) > STALL_THRESHOLD) {
          const stalledDuration = Date.now() - item.lastUpdateAt;
          if (stalledDuration > STALL_THRESHOLD) {
            hasStalled = true;
          }
        }
      }
    }

    if (hasCourseError) return "error";
    if (hasStalled) return "stalled";
    if (hasDownloading) return "downloading";
    return "idle";
  });

  let showLoopIcon = $derived(
    omniState.kind === "detected" ||
    omniState.kind === "preparing" ||
    omniState.kind === "downloading" ||
    omniState.kind === "complete"
  );

  function isUrl(value: string): boolean {
    return value.startsWith("http://") || value.startsWith("https://");
  }

  function handleInput() {
    if (debounceTimer) clearTimeout(debounceTimer);

    if (!url.trim() || !isUrl(url.trim())) {
      omniState = { kind: "idle" };
      return;
    }

    omniState = { kind: "detecting" };
    debounceTimer = setTimeout(() => {
      detectPlatform(url.trim());
    }, 500);
  }

  async function detectPlatform(value: string) {
    try {
      const result = await invoke<PlatformInfo>("detect_platform", { url: value });
      if (result.supported) {
        omniState = { kind: "detected", info: result };
      } else {
        omniState = { kind: "unsupported" };
      }
    } catch {
      omniState = { kind: "unsupported" };
    }
  }

  function getContentTypeLabel(contentType: string | null): string {
    if (!contentType) return $t("omnibox.content_type.unknown");
    const key = `omnibox.content_type.${contentType}`;
    const result = $t(key);
    if (result === key) return $t("omnibox.content_type.unknown");
    return result;
  }

  function capitalize(s: string): string {
    return s.charAt(0).toUpperCase() + s.slice(1);
  }

  async function handleAction() {
    if (omniState.kind !== "detected") return;
    const info = omniState.info;

    if (info.platform === "hotmart") {
      goto("/hotmart");
      return;
    }

    const settings = getSettings();
    let outputDir = settings?.download.default_output_dir ?? "";

    if (settings?.download.always_ask_path || !outputDir) {
      const selected = await open({
        directory: true,
        title: $t("settings.download.default_output_dir"),
      });
      if (!selected) return;
      outputDir = selected;
    }

    const currentUrl = url.trim();
    savedUrl = currentUrl;
    const platform = info.platform;
    omniState = { kind: "preparing", platform };
    url = "";

    try {
      const result = await invoke<DownloadStarted>("download_from_url", {
        url: currentUrl,
        outputDir,
      });
      omniState = {
        kind: "downloading",
        trackingId: result.id,
        platform,
        title: result.title,
      };
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("omnibox.error");
      omniState = {
        kind: "error",
        message: msg,
        originalUrl: currentUrl,
        platform,
      };
    }
  }

  function handleRetry() {
    if (omniState.kind !== "error") return;
    if (omniState.trackingId != null) {
      removeDownload(omniState.trackingId);
    }
    url = omniState.originalUrl;
    omniState = { kind: "detecting" };
    detectPlatform(url.trim());
  }

  function handleDismiss() {
    if (omniState.kind === "error" && omniState.trackingId != null) {
      removeDownload(omniState.trackingId);
    }
    omniState = { kind: "idle" };
    url = "";
  }

  async function handleCancelDownload() {
    if (omniState.kind !== "downloading") return;
    const trackingId = omniState.trackingId;
    try {
      await invoke("cancel_generic_download", { downloadId: trackingId });
    } catch {
    }
    removeDownload(trackingId);
    omniState = { kind: "idle" };
    url = "";
  }

  async function handleRevealFile() {
    if (omniState.kind !== "complete" || !omniState.filePath) return;
    try {
      await invoke("reveal_file", { path: omniState.filePath });
    } catch {
    }
  }
</script>

<div class="home">
  <Mascot emotion={mascotEmotion} />

  <div class="omnibox-area">
    {#if showLoopIcon}
      <img
        src="/loop.png"
        alt=""
        width="40"
        height="40"
        class="loop-icon"
        class:loop-bounce={omniState.kind === "detected"}
        class:loop-pulse={omniState.kind === "downloading" || omniState.kind === "preparing"}
      />
    {/if}

    {#if omniState.kind === "idle" || omniState.kind === "detecting" || omniState.kind === "detected" || omniState.kind === "unsupported"}
      <div class="omnibox-wrapper">
        <input
          class="omnibox"
          type="text"
          placeholder={$t('omnibox.placeholder')}
          bind:value={url}
          oninput={handleInput}
        />
      </div>
    {/if}

    {#if omniState.kind === "detecting"}
      <div class="feedback feedback-enter">
        <span class="feedback-spinner"></span>
      </div>

    {:else if omniState.kind === "detected"}
      <div class="feedback feedback-enter" data-supported="true">
        <span class="feedback-text">
          {capitalize(omniState.info.platform)}
          {#if omniState.info.content_type}
            <span class="feedback-sep">&middot;</span>
            {getContentTypeLabel(omniState.info.content_type)}
          {/if}
        </span>
      </div>

      {#if omniState.info.platform === "hotmart"}
        <button class="button action-btn" onclick={handleAction}>
          {$t('omnibox.go_to_hotmart')}
        </button>
      {:else}
        <button class="button action-btn" onclick={handleAction}>
          {$t('omnibox.download')}
        </button>
      {/if}

    {:else if omniState.kind === "unsupported"}
      <div class="feedback feedback-enter" data-supported="false">
        <svg class="feedback-icon" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10" />
          <path d="M12 8v4m0 4h.01" />
        </svg>
        <span class="feedback-text">{$t('omnibox.unsupported')}</span>
      </div>

    {:else if omniState.kind === "preparing"}
      <div class="feedback-card feedback-enter">
        <div class="card-row">
          <span class="feedback-spinner"></span>
          <span class="card-text">{$t('omnibox.preparing')}</span>
        </div>
      </div>

    {:else if omniState.kind === "downloading"}
      <div class="feedback-card feedback-enter">
        <div class="card-row">
          <span class="card-title">{trackedDownload?.name ?? omniState.title}</span>
          <span class="card-percent">{(trackedDownload?.percent ?? 0).toFixed(0)}%</span>
        </div>
        <div class="progress-track">
          <div
            class="progress-fill"
            style="width: {(trackedDownload?.percent ?? 0).toFixed(1)}%"
          ></div>
        </div>
        <div class="card-row card-actions">
          <span class="card-subtext">{$t('omnibox.preparing')}</span>
          <button class="button card-action-btn" onclick={handleCancelDownload}>
            {$t('downloads.cancel')}
          </button>
        </div>
      </div>

    {:else if omniState.kind === "complete"}
      <div class="feedback-card feedback-enter" data-status="complete">
        <div class="card-row">
          <svg class="card-status-icon complete" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 6L9 17l-5-5" />
          </svg>
          <span class="card-title">{omniState.title}</span>
          <button class="dismiss-btn" onclick={handleDismiss} aria-label={$t('common.close')}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        </div>
        <div class="card-row card-actions">
          <span class="card-subtext">
            {#if omniState.kind === "complete" && omniState.fileCount && omniState.fileCount > 1}
              {$t('omnibox.complete_files', { count: omniState.fileCount })}
            {:else}
              {$t('omnibox.complete')}
            {/if}
          </span>
          {#if omniState.filePath}
            <button class="button card-action-btn" onclick={handleRevealFile}>
              {$t('omnibox.open_folder')}
            </button>
          {/if}
        </div>
      </div>

    {:else if omniState.kind === "error"}
      <div class="feedback-card feedback-enter" data-status="error">
        <div class="card-row">
          <svg class="card-status-icon error" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <path d="M12 8v4m0 4h.01" />
          </svg>
          <span class="card-title card-error-text">{omniState.message}</span>
          <button class="dismiss-btn" onclick={handleDismiss} aria-label={$t('common.close')}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        </div>
        <div class="card-row card-actions">
          <span class="card-subtext">{$t('omnibox.error')}</span>
          <button class="button card-action-btn" onclick={handleRetry}>
            {$t('omnibox.retry')}
          </button>
        </div>
      </div>
    {/if}
  </div>

  <SupportedServices />
</div>

<style>
  .home {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4);
    gap: calc(var(--padding) * 1.5);
  }

  .omnibox-area {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    width: 100%;
    max-width: 560px;
  }

  .omnibox-wrapper {
    width: 100%;
  }

  .omnibox {
    width: 100%;
    padding: var(--padding) calc(var(--padding) + 4px);
    font-size: 14.5px;
    background: var(--button);
    border-radius: var(--border-radius);
    color: var(--secondary);
    border: 1px solid var(--input-border);
  }

  .omnibox::placeholder {
    color: var(--gray);
  }

  .omnibox:focus-visible {
    border-color: var(--secondary);
    outline: none;
  }

  .loop-icon {
    pointer-events: none;
    border-radius: 10px;
    user-select: none;
  }

  .loop-bounce {
    animation: loopBounce 400ms cubic-bezier(0.34, 1.56, 0.64, 1);
  }

  @keyframes loopBounce {
    0% {
      opacity: 0;
      transform: scale(0.6) translateY(6px);
    }
    60% {
      opacity: 1;
      transform: scale(1.08) translateY(-2px);
    }
    100% {
      transform: scale(1) translateY(0);
    }
  }

  .loop-pulse {
    animation: loopPulse 1.8s ease-in-out infinite;
  }

  @keyframes loopPulse {
    0%, 100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.7;
      transform: scale(0.95);
    }
  }

  .feedback {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }

  .feedback-icon {
    flex-shrink: 0;
    pointer-events: none;
  }

  .feedback[data-supported="true"] {
    color: var(--green);
  }

  .feedback[data-supported="false"] {
    color: var(--gray);
  }

  .feedback-text {
    font-size: 12.5px;
    font-weight: 500;
  }

  .feedback-sep {
    opacity: 0.5;
    margin: 0 2px;
  }

  .feedback-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .action-btn {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
    padding: calc(var(--padding) / 2) calc(var(--padding) * 1.5);
    font-size: 14.5px;
  }

  .feedback-card {
    width: 100%;
    background: var(--button);
    border-radius: var(--border-radius);
    box-shadow: var(--button-box-shadow);
    padding: var(--padding);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 0.75);
  }

  .card-row {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
    min-width: 0;
  }

  .card-title {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-text {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .card-percent {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .card-subtext {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    flex: 1;
  }

  .card-error-text {
    color: var(--red);
  }

  .card-status-icon {
    flex-shrink: 0;
    pointer-events: none;
  }

  .card-status-icon.complete {
    color: var(--green);
  }

  .card-status-icon.error {
    color: var(--red);
  }

  .card-actions {
    justify-content: space-between;
  }

  .card-action-btn {
    font-size: 12.5px;
    padding: calc(var(--padding) / 3) calc(var(--padding) * 0.75);
    flex-shrink: 0;
  }

  .dismiss-btn {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: calc(var(--border-radius) / 2);
    background: transparent;
    color: var(--gray);
    border: none;
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
  }

  @media (hover: hover) {
    .dismiss-btn:hover {
      background: var(--button-elevated);
      color: var(--secondary);
    }
  }

  .dismiss-btn:active {
    background: var(--button-elevated);
    color: var(--secondary);
  }

  .dismiss-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .dismiss-btn svg {
    pointer-events: none;
  }

  .progress-track {
    width: 100%;
    height: 6px;
    background: var(--button-elevated);
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: var(--blue);
    border-radius: 3px;
    transition: width 0.1s ease-out;
  }

  .feedback-enter {
    animation: feedbackEnter 150ms ease-out;
  }

  @keyframes feedbackEnter {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .feedback-enter {
      animation: none;
    }

    .feedback-spinner {
      animation-duration: 1.5s;
    }

    .loop-bounce {
      animation: none;
    }

    .loop-pulse {
      animation: none;
    }
  }
</style>
