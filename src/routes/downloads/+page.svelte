<script lang="ts">
  import {
    getDownloads,
    formatBytes,
    formatSpeed,
    formatEta,
    type DownloadStatus,
  } from "$lib/stores/download-store.svelte";

  let downloads = $derived(getDownloads());
  let downloadList = $derived([...downloads.values()]);
  let hasDownloads = $derived(downloadList.length > 0);

  function statusLabel(status: DownloadStatus): string {
    switch (status) {
      case "downloading":
        return "Baixando";
      case "complete":
        return "Concluído";
      case "error":
        return "Erro";
    }
  }
</script>

{#if hasDownloads}
  <div class="downloads-page">
    <h2>Downloads</h2>
    <div class="download-list">
      {#each downloadList as item (item.courseId)}
        <div class="download-item" data-status={item.status}>
          <div class="item-header">
            <span class="item-name">{item.courseName}</span>
            <span class="item-status" data-status={item.status}>
              {statusLabel(item.status)}
            </span>
          </div>

          {#if item.status === "downloading"}
            {#if item.currentModule}
              <span class="item-detail">
                {item.currentModule} &middot; {item.currentPage}
              </span>
            {/if}

            <div class="item-stats">
              {#if item.totalPages > 0}
                <span>Página {item.completedPages}/{item.totalPages}</span>
                <span class="stats-sep">&middot;</span>
                <span>Módulo {item.currentModuleIndex}/{item.totalModules}</span>
              {/if}
              {#if item.bytesDownloaded > 0}
                <span class="stats-sep">&middot;</span>
                <span>{formatBytes(item.bytesDownloaded)}</span>
              {/if}
            </div>

            <div class="item-stats">
              <span>{formatSpeed(item.speed)}</span>
              <span class="stats-sep">&middot;</span>
              <span>{formatEta(item)}</span>
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
    <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M12 3v12m0 0l-4-4m4 4l4-4" />
      <path d="M4 17v2a1 1 0 001 1h14a1 1 0 001-1v-2" />
    </svg>
    <p class="empty-text">Nenhum download em andamento</p>
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

  .item-name {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
