<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    getDownloads,
    formatBytes,
    formatSpeed,
    getEtaI18n,
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
  let hasDownloads = $derived(courseList.length > 0 || genericList.length > 0);

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

</script>

{#if hasDownloads}
  <div class="downloads-page">
    <h2>{$t('downloads.title')}</h2>
    <div class="download-list">
      {#each genericList as item (item.id)}
        <div class="download-item" data-status={item.status}>
          <div class="item-header">
            <div class="item-header-left">
              <PlatformIcon platform={item.platform} size={16} />
              <span class="item-name">{item.name}</span>
            </div>
            <div class="item-header-actions">
              {#if item.status === "downloading"}
                <button
                  class="cancel-btn"
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

          <span class="item-detail">{item.platform.charAt(0).toUpperCase() + item.platform.slice(1)}</span>

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

          <span class="item-percent">{item.percent.toFixed(0)}%</span>
        </div>
      {/each}

      {#each courseList as item (item.id)}
        <div class="download-item" data-status={item.status}>
          <div class="item-header">
            <span class="item-name">{item.name}</span>
            <div class="item-header-actions">
              {#if item.status === "downloading"}
                <button
                  class="cancel-btn"
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
              <span class="stats-sep">&middot;</span>
              {#if true}
                {@const eta = getEtaI18n(item)}
                <span>{$t(eta.key, eta.params)}</span>
              {/if}
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
      {/each}
    </div>
  </div>
{:else}
  <div class="downloads-empty">
    <Mascot emotion="idle" />
    <p class="empty-text">{$t('downloads.empty')}</p>
  </div>
{/if}

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

  .cancel-btn {
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
    .cancel-btn:hover {
      background: var(--button-elevated);
      color: var(--red);
    }
  }

  .cancel-btn:active {
    background: var(--button-elevated);
    color: var(--red);
  }

  .cancel-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .cancel-btn svg {
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

  .item-percent {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    font-variant-numeric: tabular-nums;
  }
</style>
