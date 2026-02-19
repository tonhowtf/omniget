<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    getDownloads,
    formatBytes,
    formatSpeed,
    getFinishedCount,
    type CourseDownloadItem,
    type GenericDownloadItem,
  } from "$lib/stores/download-store.svelte";
  import PlatformIcon from "$components/icons/PlatformIcon.svelte";
  import Mascot from "$components/mascot/Mascot.svelte";

  let downloads = $derived(getDownloads());
  let courseList = $derived(
    [...downloads.values()].filter((d): d is CourseDownloadItem => d.kind === "course")
  );
  let genericList = $derived(
    [...downloads.values()].filter((d): d is GenericDownloadItem => d.kind === "generic")
  );

  let activeGeneric = $derived(genericList.filter(d => d.status === "downloading"));
  let pausedGeneric = $derived(genericList.filter(d => d.status === "paused"));
  let queuedGeneric = $derived(genericList.filter(d => d.status === "queued"));
  let finishedGeneric = $derived(genericList.filter(d => d.status === "complete" || d.status === "error"));

  let hasDownloads = $derived(courseList.length > 0 || genericList.length > 0);
  let finishedCount = $derived(getFinishedCount());

  async function cancelDownload(courseId: number) {
    try {
      await invoke("cancel_course_download", { courseId });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "Error";
      showToast("error", msg);
    }
  }

  async function cancelGenericDownload(id: number) {
    try {
      await invoke("cancel_generic_download", { downloadId: id });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "Error";
      showToast("error", msg);
    }
  }

  async function pauseDownload(id: number) {
    try {
      await invoke("pause_download", { downloadId: id });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "Error";
      showToast("error", msg);
    }
  }

  async function resumeDownload(id: number) {
    try {
      await invoke("resume_download", { downloadId: id });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "Error";
      showToast("error", msg);
    }
  }

  async function retryDownload(id: number) {
    try {
      await invoke("retry_download", { downloadId: id });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "Error";
      showToast("error", msg);
    }
  }

  async function removeItem(id: number) {
    try {
      await invoke("remove_download", { downloadId: id });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "Error";
      showToast("error", msg);
    }
  }

  async function clearFinished() {
    try {
      await invoke("clear_finished_downloads");
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "Error";
      showToast("error", msg);
    }
  }
</script>

{#if hasDownloads}
  <div class="downloads-page">
    <div class="downloads-header">
      <h2>{$t('downloads.title')}</h2>
      {#if finishedCount > 0}
        <button class="clear-btn" onclick={clearFinished}>
          {$t('downloads.clear_finished')}
        </button>
      {/if}
    </div>
    <div class="download-list">
      {#each activeGeneric as item (item.id)}
        {@render genericItem(item)}
      {/each}

      {#each pausedGeneric as item (item.id)}
        {@render genericItem(item)}
      {/each}

      {#each courseList as item (item.id)}
        {@render courseItem(item)}
      {/each}

      {#if queuedGeneric.length > 0}
        <h5 class="section-label">{$t('downloads.section_queued')}</h5>
        {#each queuedGeneric as item (item.id)}
          {@render genericItem(item)}
        {/each}
      {/if}

      {#if finishedGeneric.length > 0}
        <h5 class="section-label">{$t('downloads.section_finished')}</h5>
        {#each finishedGeneric as item (item.id)}
          {@render genericItem(item)}
        {/each}
      {/if}
    </div>
  </div>
{:else}
  <div class="downloads-empty">
    <Mascot emotion="idle" />
    <p class="empty-text">{$t('downloads.empty')}</p>
  </div>
{/if}

{#snippet genericItem(item: GenericDownloadItem)}
  <div class="download-item" data-status={item.status}>
    <div class="item-header">
      <div class="item-header-left">
        <PlatformIcon platform={item.platform} size={16} />
        <span class="item-name">{item.name}</span>
      </div>
      <div class="item-header-actions">
        {#if item.status === "downloading"}
          <button
            class="action-icon-btn"
            onclick={() => pauseDownload(item.id)}
            aria-label={$t('downloads.pause')}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <rect x="6" y="4" width="4" height="16" />
              <rect x="14" y="4" width="4" height="16" />
            </svg>
          </button>
          <button
            class="action-icon-btn"
            onclick={() => cancelGenericDownload(item.id)}
            aria-label={$t('downloads.cancel')}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        {:else if item.status === "paused"}
          <button
            class="action-icon-btn"
            onclick={() => resumeDownload(item.id)}
            aria-label={$t('downloads.resume')}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polygon points="5 3 19 12 5 21 5 3" />
            </svg>
          </button>
          <button
            class="action-icon-btn"
            onclick={() => cancelGenericDownload(item.id)}
            aria-label={$t('downloads.cancel')}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        {:else if item.status === "error"}
          <button
            class="action-icon-btn"
            onclick={() => retryDownload(item.id)}
            aria-label={$t('downloads.retry')}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="23 4 23 10 17 10" />
              <path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" />
            </svg>
          </button>
          <button
            class="action-icon-btn"
            onclick={() => removeItem(item.id)}
            aria-label={$t('common.close')}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        {:else if item.status === "queued"}
          <button
            class="action-icon-btn"
            onclick={() => cancelGenericDownload(item.id)}
            aria-label={$t('downloads.cancel')}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        {/if}
        <span class="item-status" data-status={item.status}>
          {$t(`downloads.status.${item.status}`)}
        </span>
      </div>
    </div>

    {#if item.status === "downloading"}
      {#if item.phase === "fetching_info"}
        <span class="item-detail">{$t('downloads.phase_fetching_info')}</span>
      {:else if item.phase === "starting" || item.phase === "connecting"}
        <span class="item-detail">{$t('downloads.phase_starting')}</span>
      {:else}
        <span class="item-detail">{item.platform.charAt(0).toUpperCase() + item.platform.slice(1)}</span>
        <div class="item-stats">
          {#if item.downloadedBytes > 0}
            <span>
              {formatBytes(item.downloadedBytes)}{#if item.totalBytes} / {formatBytes(item.totalBytes)}{/if}
            </span>
            <span class="stats-sep">&middot;</span>
          {/if}
          {#if item.speed > 0}
            <span>{formatSpeed(item.speed)}</span>
          {/if}
        </div>
      {/if}
    {:else if item.status === "paused"}
      <span class="item-detail">{item.platform.charAt(0).toUpperCase() + item.platform.slice(1)}</span>
      {#if item.downloadedBytes > 0}
        <div class="item-stats">
          <span>{formatBytes(item.downloadedBytes)}{#if item.totalBytes} / {formatBytes(item.totalBytes)}{/if}</span>
        </div>
      {/if}
    {:else if item.status === "queued"}
      <span class="item-detail">{item.platform.charAt(0).toUpperCase() + item.platform.slice(1)}</span>
    {:else}
      <span class="item-detail">{item.platform.charAt(0).toUpperCase() + item.platform.slice(1)}</span>
    {/if}

    {#if item.status === "complete" && item.totalBytes}
      <span class="item-detail">{formatBytes(item.totalBytes)}</span>
    {/if}

    {#if item.status === "error" && item.error}
      <span class="item-error">{item.error}</span>
    {/if}

    {#if item.status !== "queued"}
      <div class="progress-track">
        <div
          class="progress-fill"
          data-status={item.status}
          style="width: {item.percent.toFixed(1)}%"
        ></div>
      </div>
      <span class="item-percent">{item.percent.toFixed(0)}%</span>
    {/if}
  </div>
{/snippet}

{#snippet courseItem(item: CourseDownloadItem)}
  <div class="download-item" data-status={item.status}>
    <div class="item-header">
      <span class="item-name">{item.name}</span>
      <div class="item-header-actions">
        {#if item.status === "downloading"}
          <button
            class="action-icon-btn"
            onclick={() => cancelDownload(item.id)}
            aria-label={$t('downloads.cancel')}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        {/if}
        <span class="item-status" data-status={item.status}>
          {$t(`downloads.status.${item.status}`)}
        </span>
      </div>
    </div>

    {#if item.status === "downloading"}
      {#if item.currentModule}
        <span class="item-detail">
          {item.currentModule} &middot; {item.currentPage}
        </span>
      {/if}

      <div class="item-stats">
        {#if item.totalPages > 0}
          <span>{$t('downloads.page_progress', { current: item.completedPages, total: item.totalPages })}</span>
          <span class="stats-sep">&middot;</span>
          <span>{$t('downloads.module_progress', { current: item.currentModuleIndex, total: item.totalModules })}</span>
        {/if}
        {#if item.bytesDownloaded > 0}
          <span class="stats-sep">&middot;</span>
          <span>{formatBytes(item.bytesDownloaded)}</span>
        {/if}
      </div>

      <div class="item-stats">
        <span>{formatSpeed(item.speed)}</span>
      </div>
    {/if}

    {#if item.status === "complete" && item.bytesDownloaded > 0}
      <span class="item-detail">{formatBytes(item.bytesDownloaded)}</span>
    {/if}

    {#if item.status === "error" && item.error}
      <span class="item-error">{item.error}</span>
    {/if}

    <div class="progress-track">
      <div
        class="progress-fill"
        data-status={item.status}
        style="width: {item.percent.toFixed(1)}%"
      ></div>
    </div>

    <span class="item-percent">{item.percent.toFixed(1)}%</span>
  </div>
{/snippet}

<style>
  .downloads-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4);
    gap: calc(var(--padding) * 1.5);
    color: var(--gray);
  }

  .empty-text {
    font-size: 14.5px;
  }

  .downloads-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
    padding: calc(var(--padding) * 1.5);
    max-width: 800px;
    margin: 0 auto;
    width: 100%;
  }

  .downloads-page h2 {
    margin-block: 0;
  }

  .downloads-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
  }

  .clear-btn {
    font-size: 12.5px;
    font-weight: 500;
    padding: calc(var(--padding) / 3) calc(var(--padding) * 0.75);
    background: var(--button-elevated);
    color: var(--gray);
    border: none;
    border-radius: calc(var(--border-radius) / 2);
    cursor: pointer;
    flex-shrink: 0;
  }

  @media (hover: hover) {
    .clear-btn:hover {
      background: var(--button-elevated-hover);
      color: var(--secondary);
    }
  }

  .clear-btn:active {
    background: var(--button-elevated-press);
  }

  .clear-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .section-label {
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-block: 0;
    padding-top: calc(var(--padding) / 2);
  }

  .download-list {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .download-item {
    background: var(--button);
    border-radius: var(--border-radius);
    box-shadow: var(--button-box-shadow);
    padding: var(--padding);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
  }

  .download-item[data-status="queued"] {
    opacity: 0.7;
  }

  .item-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
  }

  .item-header-actions {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
    flex-shrink: 0;
  }

  .item-header-left {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
    min-width: 0;
    flex: 1;
  }

  .item-name {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .action-icon-btn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: calc(var(--border-radius) / 2);
    background: transparent;
    color: var(--gray);
    border: none;
    cursor: pointer;
    padding: 0;
  }

  @media (hover: hover) {
    .action-icon-btn:hover {
      background: var(--button-elevated);
      color: var(--secondary);
    }
  }

  .action-icon-btn:active {
    background: var(--button-elevated);
    color: var(--secondary);
  }

  .action-icon-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .action-icon-btn svg {
    pointer-events: none;
  }

  .item-status {
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 2px calc(var(--padding) / 2);
    border-radius: calc(var(--border-radius) / 2);
    flex-shrink: 0;
  }

  .item-status[data-status="downloading"] {
    background: var(--orange);
    color: #000;
  }

  .item-status[data-status="complete"] {
    background: var(--green);
    color: #000;
  }

  .item-status[data-status="error"] {
    background: var(--red);
    color: #fff;
  }

  .item-status[data-status="queued"] {
    background: var(--button-elevated);
    color: var(--gray);
  }

  .item-status[data-status="paused"] {
    background: var(--blue);
    color: #fff;
  }

  .item-detail {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item-stats {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 3);
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    font-variant-numeric: tabular-nums;
  }

  .stats-sep {
    opacity: 0.5;
  }

  .item-error {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--red);
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
    border-radius: 3px;
    transition: width 0.3s ease-out;
  }

  .progress-fill[data-status="downloading"] {
    background: var(--blue);
  }

  .progress-fill[data-status="complete"] {
    background: var(--green);
  }

  .progress-fill[data-status="error"] {
    background: var(--red);
  }

  .progress-fill[data-status="paused"] {
    background: var(--gray);
  }

  .item-percent {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    font-variant-numeric: tabular-nums;
  }
</style>
