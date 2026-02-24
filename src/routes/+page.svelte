<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import Mascot from "$components/mascot/Mascot.svelte";
  import SupportedServices from "$components/services/SupportedServices.svelte";
  import OmniboxInput from "$components/omnibox/OmniboxInput.svelte";
  import DownloadModeSelector from "$components/omnibox/DownloadModeSelector.svelte";
  import QualityPicker from "$components/omnibox/QualityPicker.svelte";
  import FormatSelector from "$components/omnibox/FormatSelector.svelte";
  import MediaPreview from "$components/omnibox/MediaPreview.svelte";
  import BatchDownload from "$components/omnibox/BatchDownload.svelte";
  import SearchResults from "$components/omnibox/SearchResults.svelte";
  import { getDownloads } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { onClipboardUrl } from "$lib/stores/clipboard-monitor";
  import { getMediaPreview, clearMediaPreview } from "$lib/stores/media-preview-store.svelte";
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

  type FormatInfo = {
    format_id: string;
    ext: string;
    resolution: string | null;
    width: number | null;
    height: number | null;
    fps: number | null;
    vcodec: string | null;
    acodec: string | null;
    filesize: number | null;
    tbr: number | null;
    has_video: boolean;
    has_audio: boolean;
    format_note: string | null;
  };

  type SearchResult = {
    id: string;
    title: string;
    author: string;
    duration: number | null;
    thumbnail_url: string | null;
    url: string;
    platform: string;
  };

  type OmniState =
    | { kind: "idle" }
    | { kind: "detecting" }
    | { kind: "detected"; info: PlatformInfo }
    | { kind: "unsupported" }
    | { kind: "preparing"; platform: string }
    | { kind: "batch"; urls: string[] }
    | { kind: "searching" }
    | { kind: "search-results"; results: SearchResult[] }
    | { kind: "search-empty" }
    | { kind: "error"; message: string; originalUrl: string; platform: string };

  let url = $state("");
  let omniState = $state<OmniState>({ kind: "idle" });
  let debounceTimer = $state<ReturnType<typeof setTimeout> | null>(null);
  let downloadMode = $state<"auto" | "audio" | "mute">("auto");
  let selectedQuality = $state("best");
  let selectedFormatId = $state<string | null>(null);
  let formats = $state<FormatInfo[]>([]);
  let loadingFormats = $state(false);
  let formatError = $state<string | null>(null);
  let referer = $state("");
  let mediaPreview = $derived(getMediaPreview());
  let previewImageLoading = $state(true);

  onMount(() => {
    onClipboardUrl((detectedUrl) => {
      if (omniState.kind === "preparing") return;
      url = detectedUrl;
      handleInput();
      showToast("info", $t("toast.clipboard_url_detected"));
    });
    return () => {
      onClipboardUrl(null);
    };
  });

  const STALL_THRESHOLD = 30_000;
  let downloads = $derived(getDownloads());
  let stallTick = $state(0);

  $effect(() => {
    const interval = setInterval(() => { stallTick++; }, 5000);
    return () => clearInterval(interval);
  });

  $effect(() => {
    if (omniState.kind !== "detected") {
      clearMediaPreview();
    }
  });

  $effect(() => {
    if (mediaPreview) {
      previewImageLoading = true;
    }
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

  let showOmnibox = $derived(
    omniState.kind === "idle" ||
    omniState.kind === "detecting" ||
    omniState.kind === "detected" ||
    omniState.kind === "unsupported" ||
    omniState.kind === "batch" ||
    omniState.kind === "searching" ||
    omniState.kind === "search-results" ||
    omniState.kind === "search-empty"
  );

  function isUrl(value: string): boolean {
    return value.startsWith("http://") || value.startsWith("https://");
  }

  function handleInput() {
    if (debounceTimer) clearTimeout(debounceTimer);
    downloadMode = "auto";
    selectedQuality = "best";
    selectedFormatId = null;
    formats = [];
    formatError = null;
    referer = "";

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

    if (isUrl(trimmed)) {
      omniState = { kind: "detecting" };
      debounceTimer = setTimeout(() => {
        detectPlatform(trimmed);
      }, 500);
      return;
    }

    if (trimmed.length >= 2) {
      omniState = { kind: "searching" };
      debounceTimer = setTimeout(() => {
        performSearch(trimmed);
      }, 600);
    } else {
      omniState = { kind: "idle" };
    }
  }

  async function detectPlatform(value: string) {
    try {
      const result = await invoke<PlatformInfo>("detect_platform", { url: value });
      if (result.supported) {
        omniState = { kind: "detected", info: result };
        invoke("prefetch_media_info", { url: value }).catch(() => {});
      } else {
        omniState = { kind: "unsupported" };
      }
    } catch {
      omniState = { kind: "unsupported" };
    }
  }

  async function performSearch(query: string) {
    try {
      const results = await invoke<SearchResult[]>("search_videos", {
        query,
        platform: "youtube",
        maxResults: 6,
      });
      if (url.trim() !== query) return;
      if (results.length > 0) {
        omniState = { kind: "search-results", results };
      } else {
        omniState = { kind: "search-empty" };
      }
    } catch {
      if (url.trim() === query) {
        omniState = { kind: "search-empty" };
      }
    }
  }

  function selectSearchResult(result: SearchResult) {
    url = result.url;
    omniState = { kind: "detecting" };
    detectPlatform(result.url);
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

  async function loadFormats() {
    if (loadingFormats) return;
    if (formats.length > 0) {
      formats = [];
      selectedFormatId = null;
      formatError = null;
      return;
    }
    loadingFormats = true;
    formatError = null;
    try {
      const result = await invoke<FormatInfo[]>("get_media_formats", { url: url.trim() });
      formats = result;
      if (result.length === 0) {
        formatError = $t("omnibox.no_formats");
      }
    } catch (e: any) {
      formats = [];
      formatError = typeof e === "string" ? e : e.message ?? $t("omnibox.formats_error");
    } finally {
      loadingFormats = false;
    }
  }

  function selectFormat(formatId: string) {
    selectedFormatId = formatId;
  }

  function clearFormatSelection() {
    selectedFormatId = null;
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
        downloadMode: downloadMode === "auto" ? null : downloadMode,
        quality: selectedQuality,
        formatId: selectedFormatId,
        referer: referer.trim() || null,
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
      batchUrls.map(u => invoke<DownloadStarted>("download_from_url", {
        url: u,
        outputDir,
        downloadMode: downloadMode === "auto" ? null : downloadMode,
        quality: selectedQuality,
        formatId: null,
        referer: null,
      }))
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
    {/if}

    {#if showOmnibox}
      <OmniboxInput bind:url onInput={handleInput} />
    {/if}

    {#if omniState.kind === "detected"}
      <MediaPreview bind:mediaPreview bind:imageLoading={previewImageLoading} />

      {#if omniState.info.platform !== "hotmart"}
        <div class="download-options feedback-enter">
          <DownloadModeSelector bind:downloadMode onChange={() => { selectedFormatId = null; }} />

          <QualityPicker bind:selectedQuality selectedFormatId />

          {#if omniState.info.platform === "vimeo" || omniState.info.platform === "generic"}
            <div class="referer-input-wrapper feedback-enter">
              <label class="referer-label" for="referer-input">{$t('omnibox.referer_label')}</label>
              <input
                id="referer-input"
                class="referer-input"
                type="text"
                placeholder={$t('omnibox.referer_placeholder')}
                bind:value={referer}
                spellcheck="false"
              />
            </div>
          {/if}

          <FormatSelector
            platform={omniState.info.platform}
            bind:formats
            bind:selectedFormatId
            loadingFormats
            {formatError}
            onLoadFormats={loadFormats}
            onSelectFormat={selectFormat}
            onClearFormat={clearFormatSelection}
          />
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
      {:else}
        <button class="button action-btn" onclick={handleAction}>
          {$t('omnibox.go_to_hotmart')}
        </button>
      {/if}

    {:else if omniState.kind === "batch"}
      <BatchDownload count={omniState.urls.length} onDownload={handleBatchDownload} />

    {:else if omniState.kind === "searching"}
      <div class="feedback feedback-enter">
        <span class="feedback-spinner"></span>
        <span class="feedback-text search-hint">{$t('omnibox.searching')}</span>
      </div>

    {:else if omniState.kind === "search-results"}
      <SearchResults results={omniState.results} onSelect={selectSearchResult} />

    {:else if omniState.kind === "search-empty"}
      <div class="feedback feedback-enter" data-supported="false">
        <svg class="feedback-icon" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="11" cy="11" r="8" />
          <path d="M21 21l-4.35-4.35" />
        </svg>
        <span class="feedback-text">{$t('omnibox.search_empty')}</span>
      </div>

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
    border-top-color: var(--secondary);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .feedback-enter {
    animation: feedbackEnter 300ms cubic-bezier(0.34, 1.56, 0.64, 1);
  }

  @keyframes feedbackEnter {
    from {
      opacity: 0;
      transform: scale(0.9);
    }
    to {
      opacity: 1;
      transform: scale(1);
    }
  }

  .download-options {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    width: 100%;
  }

  .referer-input-wrapper {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .referer-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .referer-input {
    padding: 6px var(--padding);
    font-size: 13px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--secondary);
  }

  .referer-input::placeholder {
    color: var(--gray);
  }

  .referer-input:focus-visible {
    border-color: var(--secondary);
    outline: none;
  }

  .action-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: var(--padding) calc(var(--padding) * 2);
    font-size: 14.5px;
    font-weight: 500;
    background: var(--button);
    border: none;
    border-radius: var(--border-radius);
    color: var(--button-text);
    cursor: pointer;
    box-shadow: var(--button-box-shadow);
  }

  @media (hover: hover) {
    .action-btn:hover {
      background: var(--button-hover);
    }
  }

  .action-btn:active {
    background: var(--button-press);
  }

  .action-btn:disabled {
    cursor: default;
  }

  .feedback-card {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: var(--padding) calc(var(--padding) * 1.5);
    background: var(--button-elevated);
    border-radius: var(--border-radius);
    border-left: 3px solid var(--blue);
  }

  .feedback-card[data-status="error"] {
    border-left-color: var(--red);
  }

  .card-row {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }

  .card-text {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
  }

  .card-title {
    font-size: 13px;
    font-weight: 500;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-error-text {
    color: var(--secondary);
  }

  .card-status-icon {
    flex-shrink: 0;
    pointer-events: none;
    color: var(--blue);
  }

  .card-status-icon.error {
    color: var(--red);
  }

  .card-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
  }

  .card-subtext {
    font-size: 11.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .card-action-btn {
    padding: 6px 12px;
    font-size: 13px;
  }

  .dismiss-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--secondary);
    padding: 0;
  }

  .search-hint {
    color: var(--gray);
  }

  .terms-note {
    font-size: 10px;
    color: var(--gray);
    text-align: center;
  }

  .terms-link {
    color: var(--blue);
    text-decoration: none;
  }

  @media (hover: hover) {
    .terms-link:hover {
      text-decoration: underline;
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
