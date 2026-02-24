<script lang="ts">
  import { t } from "$lib/i18n";

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

  let {
    platform,
    formats = $bindable([]),
    selectedFormatId = $bindable<string | null>(null),
    loadingFormats = false,
    formatError = null as string | null,
    onLoadFormats,
    onSelectFormat,
    onClearFormat,
  } = $props();

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

  let bestVA = $derived([...formats].reverse().find(f => f.has_video && f.has_audio) ?? null);
  let bestAudio = $derived([...formats].reverse().find(f => f.has_audio && !f.has_video) ?? null);
  let bestVideo = $derived([...formats].reverse().find(f => f.has_video && !f.has_audio) ?? null);

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

  function isYtdlpPlatform(p: string): boolean {
    return !["hotmart", "telegram", "udemy", "unknown"].includes(p);
  }
</script>

{#if isYtdlpPlatform(platform)}
  <button
    class="button formats-toggle-btn"
    onclick={onLoadFormats}
    disabled={loadingFormats}
  >
    {#if loadingFormats}
      <span class="feedback-spinner small-spinner"></span>
    {:else}
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M4 6h16M4 12h16M4 18h16" />
      </svg>
    {/if}
    {#if loadingFormats}
      {$t('omnibox.view_formats')}...
    {:else if formats.length > 0}
      {$t('omnibox.hide_formats')}
    {:else}
      {$t('omnibox.view_formats')}
    {/if}
  </button>

  {#if formatError && formats.length === 0}
    <div class="formats-error">
      <span class="formats-error-text">{formatError}</span>
      <button class="button formats-retry-btn" onclick={onLoadFormats}>
        {$t('omnibox.retry')}
      </button>
    </div>
  {/if}

  {#if formats.length > 0}
    {#if !selectedFormatId}
      <div class="formats-panel">
        <div class="formats-quick">
          {#if bestVA}
            <button
              class="button format-quick-btn"
              class:active={selectedFormatId === bestVA.format_id}
              onclick={() => onSelectFormat(bestVA!.format_id)}
            >
              {$t('omnibox.best_va')}
            </button>
          {/if}
          {#if bestAudio}
            <button
              class="button format-quick-btn"
              class:active={selectedFormatId === bestAudio.format_id}
              onclick={() => onSelectFormat(bestAudio!.format_id)}
            >
              {$t('omnibox.best_audio')}
            </button>
          {/if}
          {#if bestVideo}
            <button
              class="button format-quick-btn"
              class:active={selectedFormatId === bestVideo.format_id}
              onclick={() => onSelectFormat(bestVideo!.format_id)}
            >
              {$t('omnibox.best_video')}
            </button>
          {/if}
        </div>

        <div class="formats-info">
          <span class="formats-note">
            {$t('omnibox.formats_merge_note')}
          </span>
        </div>

        <div class="formats-list">
          {#each formats as fmt (fmt.format_id)}
            <button
              class="format-row"
              class:format-row-selected={selectedFormatId === fmt.format_id}
              onclick={() => onSelectFormat(fmt.format_id)}
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
    {:else}
      <div class="format-selected">
        <span class="format-selected-label">{selectedFormatLabel}</span>
        <button class="format-clear-btn" onclick={onClearFormat} aria-label={$t('common.close')}>
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>
    {/if}
  {/if}
{/if}

<style>
  .formats-toggle-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 14.5px;
  }

  .small-spinner {
    width: 12px;
    height: 12px;
    border-width: 1.5px;
  }

  .formats-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
    padding: calc(var(--padding) / 2) var(--padding);
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) - 2px);
    border-left: 3px solid var(--red);
  }

  .formats-error-text {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--secondary);
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .formats-retry-btn {
    font-size: 12.5px;
    padding: 4px 10px;
    flex-shrink: 0;
  }

  .formats-panel {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
    width: 100%;
    background: var(--button);
    border-radius: var(--border-radius);
    box-shadow: var(--button-box-shadow);
    padding: var(--padding);
    max-height: 400px;
    overflow: hidden;
  }

  .formats-quick {
    display: flex;
    gap: calc(var(--padding) / 2);
    flex-wrap: wrap;
  }

  .format-quick-btn {
    font-size: 13px;
    padding: 6px 12px;
  }

  .formats-info {
    display: flex;
    justify-content: center;
    align-items: center;
  }

  .formats-note {
    font-size: 11px;
    color: var(--gray);
    text-align: center;
    line-height: 1.4;
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

  .format-selected {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
    padding: var(--padding);
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) - 2px);
    font-size: 13px;
  }

  .format-selected-label {
    color: var(--secondary);
    font-weight: 500;
    flex: 1;
  }

  .format-clear-btn {
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

  @media (max-width: 535px) {
    .format-row {
      grid-template-columns: 40px 40px 64px 24px 56px 56px 56px 40px;
      font-size: 10px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
  }
</style>
