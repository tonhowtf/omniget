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
  let downloadMode = $state<"auto" | "audio" | "mute">("auto");
  let selectedQuality = $state("best");
  let selectedFormatId = $state<string | null>(null);
  let formats = $state<FormatInfo[]>([]);
  let formatsOpen = $state(false);
  let loadingFormats = $state(false);

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
    downloadMode = "auto";
    selectedQuality = "best";
    selectedFormatId = null;
    formats = [];
    formatsOpen = false;

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

  function isYtdlpPlatform(platform: string): boolean {
    return platform === "youtube";
  }

  async function loadFormats() {
    if (loadingFormats || formats.length > 0) {
      formatsOpen = !formatsOpen;
      return;
    }
    loadingFormats = true;
    try {
      const result = await invoke<FormatInfo[]>("get_media_formats", { url: url.trim() });
      formats = result;
      formatsOpen = true;
    } catch {
      formats = [];
    } finally {
      loadingFormats = false;
    }
  }

  function selectFormat(formatId: string) {
    selectedFormatId = formatId;
    formatsOpen = false;
  }

  function clearFormatSelection() {
    selectedFormatId = null;
  }

  function formatFilesize(bytes: number | null): string {
    if (bytes === null) return "—";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function formatCodec(codec: string | null): string {
    if (!codec || codec === "none") return "—";
    return codec.split(".")[0];
  }

  function bestVideoAudio(): FormatInfo | null {
    return formats.find(f => f.has_video && f.has_audio) ?? null;
  }

  function bestAudioOnly(): FormatInfo | null {
    return [...formats].reverse().find(f => f.has_audio && !f.has_video) ?? null;
  }

  function bestVideoOnly(): FormatInfo | null {
    return formats.find(f => f.has_video && !f.has_audio) ?? null;
  }

  let selectedFormatLabel = $derived.by(() => {
    if (!selectedFormatId) return null;
    const f = formats.find(fmt => fmt.format_id === selectedFormatId);
    if (!f) return selectedFormatId;
    const parts: string[] = [];
    if (f.resolution) parts.push(f.resolution);
    parts.push(f.ext);
    if (f.has_video && f.has_audio) parts.push("V+A");
    else if (f.has_video) parts.push("V");
    else if (f.has_audio) parts.push("A");
    return parts.join(" · ");
  });

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
        quality: selectedQuality === "best" ? null : selectedQuality,
        formatId: selectedFormatId,
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
        quality: selectedQuality === "best" ? null : selectedQuality,
        formatId: null,
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

      {#if omniState.info.platform !== "hotmart"}
        <div class="download-options feedback-enter">
          <div class="mode-switcher">
            <button
              class="button mode-btn"
              class:active={downloadMode === "auto"}
              onclick={() => { downloadMode = "auto"; selectedFormatId = null; }}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M12 3l1.912 5.813a2 2 0 001.272 1.272L21 12l-5.813 1.912a2 2 0 00-1.272 1.272L12 21l-1.912-5.813a2 2 0 00-1.272-1.272L3 12l5.813-1.912a2 2 0 001.272-1.272z" />
              </svg>
              {$t('omnibox.mode_auto')}
            </button>
            <button
              class="button mode-btn"
              class:active={downloadMode === "audio"}
              onclick={() => { downloadMode = "audio"; selectedFormatId = null; }}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M9 18V5l12-2v13" />
                <circle cx="6" cy="18" r="3" />
                <circle cx="18" cy="16" r="3" />
              </svg>
              {$t('omnibox.mode_audio')}
            </button>
            <button
              class="button mode-btn"
              class:active={downloadMode === "mute"}
              onclick={() => { downloadMode = "mute"; selectedFormatId = null; }}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M11 5L6 9H2v6h4l5 4V5z" />
                <line x1="23" y1="9" x2="17" y2="15" />
                <line x1="17" y1="9" x2="23" y2="15" />
              </svg>
              {$t('omnibox.mode_mute')}
            </button>
          </div>

          {#if !selectedFormatId}
            <div class="quality-select-wrapper">
              <span class="quality-label">{$t('omnibox.quality')}</span>
              <select class="quality-select" bind:value={selectedQuality}>
                <option value="best">best</option>
                <option value="1080p">1080p</option>
                <option value="720p">720p</option>
                <option value="480p">480p</option>
                <option value="360p">360p</option>
              </select>
            </div>
          {:else}
            <div class="format-selected">
              <span class="format-selected-label">{selectedFormatLabel}</span>
              <button class="format-clear-btn" onclick={clearFormatSelection} aria-label={$t('common.close')}>
                <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M18 6L6 18M6 6l12 12" />
                </svg>
              </button>
            </div>
          {/if}
        </div>

        {#if isYtdlpPlatform(omniState.info.platform)}
          <button
            class="button formats-toggle-btn"
            onclick={loadFormats}
            disabled={loadingFormats}
          >
            {#if loadingFormats}
              <span class="feedback-spinner small-spinner"></span>
            {:else}
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M4 6h16M4 12h16M4 18h16" />
              </svg>
            {/if}
            {$t('omnibox.view_formats')}
          </button>

          {#if formatsOpen && formats.length > 0}
            <div class="formats-panel feedback-enter">
              <div class="formats-quick">
                {#if bestVideoAudio()}
                  <button
                    class="button format-quick-btn"
                    class:active={selectedFormatId === bestVideoAudio()?.format_id}
                    onclick={() => selectFormat(bestVideoAudio()!.format_id)}
                  >
                    {$t('omnibox.best_va')}
                  </button>
                {/if}
                {#if bestAudioOnly()}
                  <button
                    class="button format-quick-btn"
                    class:active={selectedFormatId === bestAudioOnly()?.format_id}
                    onclick={() => selectFormat(bestAudioOnly()!.format_id)}
                  >
                    {$t('omnibox.best_audio')}
                  </button>
                {/if}
                {#if bestVideoOnly()}
                  <button
                    class="button format-quick-btn"
                    class:active={selectedFormatId === bestVideoOnly()?.format_id}
                    onclick={() => selectFormat(bestVideoOnly()!.format_id)}
                  >
                    {$t('omnibox.best_video')}
                  </button>
                {/if}
              </div>

              <div class="formats-list">
                {#each formats as fmt (fmt.format_id)}
                  <button
                    class="format-row"
                    class:format-row-selected={selectedFormatId === fmt.format_id}
                    onclick={() => selectFormat(fmt.format_id)}
                  >
                    <span class="format-id">{fmt.format_id}</span>
                    <span class="format-ext">{fmt.ext}</span>
                    <span class="format-res">{fmt.resolution ?? "—"}</span>
                    <span class="format-codec">
                      {#if fmt.has_video && fmt.has_audio}
                        V+A
                      {:else if fmt.has_video}
                        V
                      {:else if fmt.has_audio}
                        A
                      {:else}
                        —
                      {/if}
                    </span>
                    <span class="format-vcodec">{formatCodec(fmt.vcodec)}</span>
                    <span class="format-acodec">{formatCodec(fmt.acodec)}</span>
                    <span class="format-size">{formatFilesize(fmt.filesize)}</span>
                    {#if fmt.tbr}
                      <span class="format-tbr">{fmt.tbr.toFixed(0)}k</span>
                    {:else}
                      <span class="format-tbr">—</span>
                    {/if}
                  </button>
                {/each}
              </div>
            </div>
          {/if}
        {/if}
      {/if}

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

  .download-options {
    display: flex;
    align-items: center;
    gap: var(--padding);
    flex-wrap: wrap;
    justify-content: center;
  }

  .mode-switcher {
    display: flex;
    gap: 0;
  }

  .mode-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: calc(var(--padding) / 3) calc(var(--padding) * 0.75);
    font-size: 12.5px;
    border-radius: 0;
  }

  .mode-btn:first-child {
    border-radius: var(--border-radius) 0 0 var(--border-radius);
  }

  .mode-btn:last-child {
    border-radius: 0 var(--border-radius) var(--border-radius) 0;
  }

  .mode-btn + .mode-btn {
    margin-left: -1px;
  }

  .mode-btn svg {
    pointer-events: none;
    flex-shrink: 0;
  }

  .quality-select-wrapper {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }

  .quality-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .quality-select {
    font-size: 12.5px;
    font-weight: 500;
    font-family: inherit;
    padding: calc(var(--padding) / 3) calc(var(--padding) * 0.75);
    background: var(--button);
    color: var(--secondary);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) / 2);
    cursor: pointer;
  }

  .quality-select:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
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

  .format-selected {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: calc(var(--padding) / 3) calc(var(--padding) * 0.75);
    background: var(--button);
    border-radius: calc(var(--border-radius) / 2);
    box-shadow: var(--button-box-shadow);
  }

  .format-selected-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--secondary);
  }

  .format-clear-btn {
    width: 18px;
    height: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    background: transparent;
    color: var(--gray);
    border: none;
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
  }

  @media (hover: hover) {
    .format-clear-btn:hover {
      color: var(--secondary);
    }
  }

  .format-clear-btn svg {
    pointer-events: none;
  }

  .formats-toggle-btn {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
    font-size: 12.5px;
    padding: calc(var(--padding) / 3) calc(var(--padding) * 0.75);
  }

  .formats-toggle-btn svg {
    pointer-events: none;
    flex-shrink: 0;
  }

  .small-spinner {
    width: 12px;
    height: 12px;
  }

  .formats-panel {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
  }

  .formats-quick {
    display: flex;
    gap: calc(var(--padding) / 3);
    flex-wrap: wrap;
    justify-content: center;
  }

  .format-quick-btn {
    font-size: 12px;
    padding: calc(var(--padding) / 4) calc(var(--padding) * 0.6);
  }

  .formats-list {
    max-height: 240px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    background: var(--button);
    border-radius: var(--border-radius);
    box-shadow: var(--button-box-shadow);
    scrollbar-width: none;
  }

  .formats-list::-webkit-scrollbar {
    display: none;
  }

  .format-row {
    display: grid;
    grid-template-columns: 48px 48px 80px 32px 64px 64px 64px 48px;
    align-items: center;
    gap: 2px;
    padding: calc(var(--padding) / 2) calc(var(--padding) / 2 + 4px);
    font-size: 11px;
    font-weight: 500;
    color: var(--gray);
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    border-bottom: 1px solid var(--button-stroke);
  }

  .format-row:last-child {
    border-bottom: none;
  }

  @media (hover: hover) {
    .format-row:hover {
      background: var(--button-hover);
      color: var(--secondary);
    }
  }

  .format-row:active {
    background: var(--button-press);
  }

  .format-row-selected {
    background: var(--button-elevated);
    color: var(--secondary);
  }

  .format-id {
    color: var(--secondary);
    font-weight: 500;
  }

  .format-ext {
    color: var(--blue);
  }

  .format-res {
    color: var(--secondary);
  }

  .format-codec {
    text-align: center;
  }

  @media (max-width: 535px) {
    .format-row {
      grid-template-columns: 40px 40px 64px 24px 56px 56px 56px 40px;
      font-size: 10px;
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
