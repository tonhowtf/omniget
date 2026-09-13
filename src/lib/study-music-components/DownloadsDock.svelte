<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    downloadStore,
    type DownloadJobState,
  } from "$lib/study-music/download-store.svelte";
  import { soundcloudStore } from "$lib/study-music/soundcloud-store.svelte";
  import { colorFromString } from "$lib/study-music/format";
  import { getLastCodec, getLastDownloadDir } from "$lib/study-music/local-prefs";

  const jobs = $derived(downloadStore.jobsList);
  const activeCount = $derived(downloadStore.activeCount);
  const hasJobs = $derived(downloadStore.hasJobs);
  const drawerOpen = $derived(downloadStore.drawerOpen);

  function stageLabel(job: DownloadJobState): string {
    if (job.error) return $t("study.music.dock.failed");
    if (job.stage === "pending") return $t("study.music.dock.pending");
    if (job.stage === "starting") return $t("study.music.dock.preparing");
    if (job.stage === "downloading") {
      if (job.kind === "bulk") {
        return $t("study.music.dock.downloading", { a: job.currentCount ?? 0, b: job.totalCount ?? 0 });
      }
      return $t("study.music.dock.saving_track");
    }
    if (job.stage === "skipped") return $t("study.music.dock.already_had");
    if (job.stage === "done") {
      if (job.kind === "bulk") {
        const ok = job.successCount ?? 0;
        const fail = job.failedCount ?? 0;
        return $t("study.music.dock.done_bulk", { ok, fail });
      }
      return $t("study.music.dock.done");
    }
    return job.stage;
  }

  function fallbackColor(job: DownloadJobState): string {
    return colorFromString(job.title || "track");
  }

  async function retrySingle(job: DownloadJobState) {
    if (job.trackId === undefined) return;
    const codec = getLastCodec() ?? "mp3";
    const dir = getLastDownloadDir();
    if (!dir) {
      showToast("error", $t("study.music.dock.pick_folder_again"));
      downloadStore.removeJob(job.id);
      return;
    }
    downloadStore.removeJob(job.id);
    const optimisticId = downloadStore.addOptimisticSingleJob({
      trackId: job.trackId,
      title: job.title,
      artist: job.artist,
      artwork: job.artwork,
    });
    try {
      await soundcloudStore.download({
        trackId: job.trackId,
        codec,
        outputDir: dir,
        quality: "progressive",
      });
    } catch (e) {
      downloadStore.markJobError(
        optimisticId,
        e instanceof Error ? e.message : String(e),
      );
    }
  }

  async function retryFailedFromBulk(
    job: DownloadJobState,
    failed: { id: number; title: string; error?: string },
  ) {
    const codec = job.codec ?? getLastCodec() ?? "mp3";
    const dir = job.outputDir ?? getLastDownloadDir();
    if (!dir) {
      showToast("error", $t("study.music.dock.pick_folder_again"));
      return;
    }
    downloadStore.removeFailedTrackFromBulk(job.id, failed.id);
    const optId = downloadStore.addOptimisticSingleJob({
      trackId: failed.id,
      title: failed.title,
    });
    try {
      await soundcloudStore.download({
        trackId: failed.id,
        codec,
        outputDir: dir,
        quality: "progressive",
      });
    } catch (e) {
      downloadStore.markJobError(optId, e instanceof Error ? e.message : String(e));
    }
  }

  async function retryAllFailed(job: DownloadJobState) {
    const codec = job.codec ?? getLastCodec() ?? "mp3";
    const dir = job.outputDir ?? getLastDownloadDir();
    if (!dir) {
      showToast("error", $t("study.music.dock.pick_folder_again"));
      return;
    }
    const failed = downloadStore.consumeFailedTracks(job.id);
    if (failed.length === 0) return;
    for (const f of failed) {
      const optId = downloadStore.addOptimisticSingleJob({
        trackId: f.id,
        title: f.title,
      });
      try {
        await soundcloudStore.download({
          trackId: f.id,
          codec,
          outputDir: dir,
          quality: "progressive",
        });
      } catch (e) {
        downloadStore.markJobError(optId, e instanceof Error ? e.message : String(e));
      }
    }
  }

  function toggleExpand(job: DownloadJobState) {
    downloadStore.toggleExpanded(job.id);
  }

  function dismiss(job: DownloadJobState) {
    downloadStore.removeJob(job.id);
  }

  function openExternal(url?: string | null) {
    if (!url) return;
    void (async () => {
      try {
        const opener = await import("@tauri-apps/plugin-opener");
        await opener.openUrl(url);
      } catch {
        try {
          window.open(url, "_blank");
        } catch {
          /* ignore */
        }
      }
    })();
  }

  function toggleDrawer() {
    downloadStore.toggleDrawer();
  }

  function clearDone() {
    downloadStore.clearCompleted();
  }
</script>

{#if hasJobs}
  <button
    type="button"
    class="fab"
    class:active={activeCount > 0}
    onclick={toggleDrawer}
    aria-label={drawerOpen ? "Fechar downloads" : "Abrir downloads"}
    title={drawerOpen ? "Fechar downloads" : "Downloads"}
  >
    <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
      <polyline points="7 10 12 15 17 10"/>
      <line x1="12" y1="15" x2="12" y2="3"/>
    </svg>
    {#if activeCount > 0}
      <span class="badge">{activeCount}</span>
    {/if}
  </button>
{/if}

{#if drawerOpen && hasJobs}
  <aside class="drawer" role="dialog" aria-label="Downloads">
    <header class="drawer-head">
      <span class="drawer-title">
        Downloads <span class="muted">({jobs.length})</span>
      </span>
      <div class="head-actions">
        <button type="button" class="link" onclick={clearDone}>Limpar concluídos</button>
        <button type="button" class="close" onclick={toggleDrawer} aria-label="Fechar">×</button>
      </div>
    </header>

    <div class="list">
      {#each jobs as job (job.id)}
        {@const failedList = job.failedTracks ?? []}
        {@const canExpand = job.kind === "bulk" && failedList.length > 0}
        <div class="row" class:err={!!job.error}>
          <div class="cover">
            {#if job.artwork}
              <img src={job.artwork} alt="" loading="lazy" />
            {:else}
              <div class="cover-fb" style:background={fallbackColor(job)}></div>
            {/if}
          </div>
          <div class="meta">
            <span class="title" title={job.title}>{job.title}</span>
            <span class="sub">
              {#if job.kind === "single" && job.artist}
                {job.artist} ·
              {/if}
              {stageLabel(job)}
            </span>
            {#if job.kind === "bulk" && job.stage === "downloading" && job.currentTrackTitle}
              <span class="sub thin">↳ {job.currentTrackTitle}</span>
            {/if}
            <div class="bar" class:err={!!job.error}>
              <div class="fill" style:width={`${job.progressPct}%`}></div>
            </div>
            {#if job.error}
              <span class="err-text" title={job.error}>{job.error}</span>
            {/if}
            {#if canExpand}
              <div class="bulk-actions">
                <button
                  type="button"
                  class="expand-btn"
                  onclick={() => toggleExpand(job)}
                  aria-expanded={!!job.expanded}
                >
                  <span class="chev" class:open={job.expanded}>▸</span>
                  {$t("study.music.dock.failed_count", { n: failedList.length })}
                </button>
                <button
                  type="button"
                  class="retry-all"
                  onclick={() => retryAllFailed(job)}
                  title="Tentar todas falhadas"
                >
                  Tentar todas
                </button>
              </div>
              {#if job.expanded}
                <ul class="failed-list">
                  {#each failedList as ft (ft.id)}
                    <li class="failed-row">
                      <span class="failed-title" title={ft.title}>{ft.title}</span>
                      {#if ft.error}
                        <span class="failed-err" title={ft.error}>{ft.error}</span>
                      {/if}
                      <button
                        type="button"
                        class="failed-retry"
                        onclick={() => retryFailedFromBulk(job, ft)}
                      >
                        Tentar
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
            {/if}
          </div>
          <div class="row-actions">
            {#if job.error && job.kind === "single"}
              <button type="button" class="action-btn" onclick={() => retrySingle(job)}>
                Tentar de novo
              </button>
            {/if}
            {#if job.stage === "done" && job.permalinkUrl}
              <button
                type="button"
                class="ghost-btn"
                onclick={() => openExternal(job.permalinkUrl)}
                title="Abrir no SoundCloud"
                aria-label="Abrir no SoundCloud"
              >
                ↗
              </button>
            {/if}
            {#if job.stage === "done" || job.stage === "skipped" || job.error}
              <button
                type="button"
                class="ghost-btn"
                onclick={() => dismiss(job)}
                aria-label="Remover"
                title="Remover"
              >
                ×
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  </aside>
{/if}

<style>
  .fab {
    position: fixed;
    bottom: 96px;
    right: 18px;
    width: 52px;
    height: 52px;
    border-radius: 50%;
    background: #ff5500;
    color: white;
    border: 0;
    display: grid;
    place-items: center;
    cursor: pointer;
    box-shadow: 0 6px 22px rgba(0, 0, 0, 0.35), 0 0 0 1px rgba(0, 0, 0, 0.18) inset;
    z-index: 110;
    transition: transform 160ms ease, background 200ms ease;
  }
  .fab:hover { background: #ff7733; transform: translateY(-1px); }
  .fab:focus-visible { outline: 2px solid white; outline-offset: 2px; }
  .fab.active { animation: pulse 2.4s ease-in-out infinite; }
  @keyframes pulse {
    0%, 100% { box-shadow: 0 6px 22px rgba(0, 0, 0, 0.35), 0 0 0 0 rgba(255, 85, 0, 0.45); }
    50% { box-shadow: 0 6px 22px rgba(0, 0, 0, 0.35), 0 0 0 12px rgba(255, 85, 0, 0); }
  }
  .badge {
    position: absolute;
    top: -4px;
    right: -4px;
    min-width: 20px;
    height: 20px;
    padding: 0 5px;
    border-radius: 10px;
    background: white;
    color: #ff5500;
    font-size: 11px;
    font-weight: 800;
    display: grid;
    place-items: center;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
  }
  .drawer {
    position: fixed;
    bottom: 96px;
    right: 86px;
    width: min(420px, calc(100vw - 32px));
    max-height: min(70vh, 640px);
    background: rgba(20, 20, 20, 0.97);
    border: none;
    border-radius: 14px;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.5);
    z-index: 109;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    color: white;
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    animation: slide-in 200ms ease;
  }
  @keyframes slide-in {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }
  .drawer-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 16px;
    border-bottom: none;
  }
  .drawer-title { font-size: 13px; font-weight: 700; letter-spacing: 0.02em; }
  .muted { color: rgba(255, 255, 255, 0.5); font-weight: 500; }
  .head-actions { display: flex; align-items: center; gap: 8px; }
  .link {
    background: transparent;
    border: 0;
    color: rgba(255, 255, 255, 0.6);
    font-family: inherit;
    font-size: 11px;
    cursor: pointer;
    padding: 4px 6px;
    border-radius: 4px;
  }
  .link:hover { color: white; background: rgba(255, 255, 255, 0.05); }
  .close {
    background: transparent;
    border: 0;
    color: rgba(255, 255, 255, 0.6);
    font-size: 22px;
    line-height: 1;
    cursor: pointer;
    padding: 0 6px;
    border-radius: 4px;
  }
  .close:hover { color: white; }
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .row {
    display: grid;
    grid-template-columns: 40px 1fr auto;
    gap: 12px;
    padding: 10px 10px;
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid transparent;
    align-items: flex-start;
  }
  .row.err { border-color: rgba(226, 33, 52, 0.4); background: rgba(226, 33, 52, 0.06); }
  .cover {
    width: 40px;
    height: 40px;
    border-radius: 6px;
    overflow: hidden;
    background: rgba(255, 255, 255, 0.05);
    flex-shrink: 0;
  }
  .cover img { width: 100%; height: 100%; object-fit: cover; }
  .cover-fb { width: 100%; height: 100%; }
  .meta { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .title {
    font-size: 13px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: 11px;
    color: rgba(255, 255, 255, 0.55);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub.thin { color: rgba(255, 255, 255, 0.4); }
  .bar {
    height: 4px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 2px;
    overflow: hidden;
    margin-top: 2px;
  }
  .bar.err .fill { background: #e22134; }
  .fill {
    height: 100%;
    background: #ff5500;
    transition: width 200ms ease;
  }
  .err-text {
    font-size: 11px;
    color: #e22134;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-actions {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 4px;
  }
  .action-btn {
    padding: 4px 10px;
    background: #ff5500;
    color: white;
    border: 0;
    border-radius: 999px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    cursor: pointer;
    white-space: nowrap;
  }
  .action-btn:hover { background: #ff7733; }
  .ghost-btn {
    padding: 2px 8px;
    background: rgba(255, 255, 255, 0.06);
    color: rgba(255, 255, 255, 0.7);
    border: 0;
    border-radius: 999px;
    font-family: inherit;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    line-height: 1.3;
  }
  .ghost-btn:hover { background: rgba(255, 255, 255, 0.12); color: white; }
  .bulk-actions {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 4px;
    flex-wrap: wrap;
  }
  .expand-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 8px;
    background: rgba(226, 33, 52, 0.12);
    color: #ffb4ba;
    border: 0;
    border-radius: 999px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
  }
  .expand-btn:hover { background: rgba(226, 33, 52, 0.2); color: white; }
  .chev {
    display: inline-block;
    transition: transform 160ms ease;
    font-size: 9px;
  }
  .chev.open { transform: rotate(90deg); }
  .retry-all {
    padding: 3px 10px;
    background: #ff5500;
    color: white;
    border: 0;
    border-radius: 999px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    cursor: pointer;
  }
  .retry-all:hover { background: #ff7733; }
  .failed-list {
    margin: 6px 0 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 180px;
    overflow-y: auto;
  }
  .failed-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 8px;
    align-items: center;
    padding: 6px 8px;
    background: rgba(255, 255, 255, 0.03);
    border-radius: 6px;
    font-size: 11px;
  }
  .failed-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: rgba(255, 255, 255, 0.85);
  }
  .failed-err {
    display: none;
  }
  .failed-retry {
    padding: 3px 10px;
    background: rgba(255, 85, 0, 0.18);
    color: #ff8c4d;
    border: 0;
    border-radius: 999px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    cursor: pointer;
  }
  .failed-retry:hover { background: #ff5500; color: white; }

  @media (prefers-reduced-motion: reduce) {
    .drawer { animation: none; }
    .fab { transition: none; }
    .fab.active { animation: none; }
    .fill { transition: none; }
    .chev { transition: none; }
  }
</style>
