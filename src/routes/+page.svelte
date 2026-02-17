<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { goto } from "$app/navigation";
  import Mascot from "$components/mascot/Mascot.svelte";
  import SupportedServices from "$components/services/SupportedServices.svelte";
  import { getDownloads } from "$lib/stores/download-store.svelte";
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
    | { kind: "batch"; urls: string[] }
    | { kind: "error"; message: string; originalUrl: string; platform: string };

  let url = $state("");
  let omniState = $state<OmniState>({ kind: "idle" });
  let debounceTimer = $state<ReturnType<typeof setTimeout> | null>(null);

  const STALL_THRESHOLD = 30_000;

  let downloads = $derived(getDownloads());

  let stallTick = $state(0);
  $effect(() => {
    const interval = setInterval(() => { stallTick++; }, 5000);
    return () => clearInterval(interval);
  });

  let mascotEmotion = $derived.by((): "idle" | "downloading" | "error" | "stalled" | "queue" => {
    void stallTick;

    if (omniState.kind === "preparing") return "downloading";
    if (omniState.kind === "error") return "error";

    let hasActiveDownloading = false;
    let hasActiveStalled = false;
    let hasItems = false;
    for (const item of downloads.values()) {
      hasItems = true;
      if (item.status === "downloading") {
        hasActiveDownloading = true;
        const elapsed = Date.now() - item.lastUpdateAt;
        if (elapsed > STALL_THRESHOLD) {
          hasActiveStalled = true;
        }
      }
    }

    if (hasActiveStalled) return "stalled";
    if (hasActiveDownloading) return "downloading";
    if (hasItems) return "queue";
    return "idle";
  });

  let showLoopIcon = $derived(
    omniState.kind === "detected" ||
    omniState.kind === "preparing" ||
    omniState.kind === "batch"
  );

  function isUrl(value: string): boolean {
    return value.startsWith("http://") || value.startsWith("https://");
  }

  function handleInput() {
    if (debounceTimer) clearTimeout(debounceTimer);

    const trimmed = url.trim();
    if (!trimmed) {
      omniState = { kind: "idle" };
      return;
    }

    const urls = trimmed.split(/[\s\n]+/).filter(isUrl);

    if (urls.length > 1) {
      omniState = { kind: "batch", urls };
      return;
    }

    if (!isUrl(trimmed)) {
      omniState = { kind: "idle" };
      return;
    }

    omniState = { kind: "detecting" };
    debounceTimer = setTimeout(() => {
      detectPlatform(trimmed);
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
    const platform = info.platform;
    omniState = { kind: "preparing", platform };
    url = "";

    try {
      await invoke<DownloadStarted>("download_from_url", {
        url: currentUrl,
        outputDir,
      });
      omniState = { kind: "idle" };
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

  async function handleBatchDownload() {
    if (omniState.kind !== "batch") return;
    const batchUrls = omniState.urls;

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

    omniState = { kind: "idle" };
    url = "";

    const results = await Promise.allSettled(
      batchUrls.map(u => invoke<DownloadStarted>("download_from_url", { url: u, outputDir }))
    );

    const queued = results.filter(r => r.status === "fulfilled").length;
    if (queued > 0) {
      showToast("info", $t("omnibox.batch_queued", { count: queued }));
    }
  }

  function handleRetry() {
    if (omniState.kind !== "error") return;
    url = omniState.originalUrl;
    omniState = { kind: "detecting" };
    detectPlatform(url.trim());
  }

  function handleDismiss() {
    omniState = { kind: "idle" };
    url = "";
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
        class:loop-bounce={omniState.kind === "detected" || omniState.kind === "batch"}
        class:loop-pulse={omniState.kind === "preparing"}
      />
    {/if}

    {#if omniState.kind === "idle" || omniState.kind === "detecting" || omniState.kind === "detected" || omniState.kind === "unsupported" || omniState.kind === "batch"}
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

    {:else if omniState.kind === "batch"}
      <div class="feedback feedback-enter" data-supported="true">
        <span class="feedback-text">
          {$t('omnibox.batch_detected', { count: omniState.urls.length })}
        </span>
      </div>

      <button class="button action-btn" onclick={handleBatchDownload}>
        {$t('omnibox.batch_download_all')}
      </button>

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

  <div class="terms-note">
    {$t('terms_note.agreement')}
    <a href="/about/terms" class="terms-link">{$t('terms_note.link')}</a>
  </div>
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

  .terms-note {
    color: var(--gray);
    font-size: 12px;
    text-align: center;
    font-weight: 500;
    padding-bottom: 6px;
  }

  .terms-link {
    color: var(--gray);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  @media (hover: hover) {
    .terms-link:hover {
      color: var(--secondary);
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
