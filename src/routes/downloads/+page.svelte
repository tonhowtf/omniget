<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    getDownloads,
    formatBytes,
    formatSpeed,
    formatEta,
    getFinishedCount,
    getSpeedHistory,
    type CourseDownloadItem,
    type GenericDownloadItem,
    type QueueKind,
  } from "$lib/stores/download-store.svelte";
  import { getDownloadStats } from "$lib/stores/download-stats.svelte";
  import PlatformIcon from "$components/icons/PlatformIcon.svelte";
  import QueueKindBadge from "$lib/study-components/QueueKindBadge.svelte";
  import Mascot from "$components/mascot/Mascot.svelte";
  import ContextHint from "$components/hints/ContextHint.svelte";
  import DownloadSpeedGraph from "$components/download/DownloadSpeedGraph.svelte";
  import DownloadLog from "$components/download/DownloadLog.svelte";
  import ReencodeDialog from "$components/dialog/ReencodeDialog.svelte";
  import ToolsPanel from "$components/downloads/ToolsPanel.svelte";
  import VideoOpsOverlay from "$components/downloads/VideoOpsOverlay.svelte";
  import { locale as i18nLocale } from "$lib/i18n";
  import { get } from "svelte/store";
  import timeAgo from "$lib/time-ago";

  let studyAvailable = $state(false);

  onMount(async () => {
    try {
      const plugins = await invoke<{
        id: string;
        enabled: boolean;
        loaded: boolean;
      }[]>("list_plugins");
      studyAvailable = plugins.some(
        (p) => p.id === "study" && p.enabled && p.loaded,
      );
    } catch {
      studyAvailable = false;
    }
  });

  const VIDEO_EXTENSIONS = new Set([
    "mp4", "mkv", "webm", "mov", "avi", "ts", "m4v", "flv", "wmv", "mpg", "mpeg", "3gp", "ogv", "m2ts", "mts",
  ]);

  function fileExtension(path: string): string | null {
    const name = path.replace(/\\/g, "/").split("/").pop() ?? "";
    const dot = name.lastIndexOf(".");
    if (dot <= 0 || dot === name.length - 1) return null;
    return name.slice(dot + 1).toLowerCase();
  }

  function isVideoItem(item: GenericDownloadItem): boolean {
    if (item.filePath && (item.fileCount ?? 1) <= 1) {
      const ext = fileExtension(item.filePath);
      if (ext) return VIDEO_EXTENSIONS.has(ext);
    }
    if (item.downloadMode === "video") return true;
    return item.queueKind === "video";
  }

  function qualityChip(item: GenericDownloadItem): string | null {
    if (item.downloadMode === "audio") return $t('omnibox.quality_audio') as string;
    if (!item.quality) return null;
    const q = item.quality.toLowerCase();
    if (q === "audio") return $t('omnibox.quality_audio') as string;
    if (!isVideoItem(item)) return null;
    if (q === "best" || q === "highest") return $t('omnibox.quality_best_short') as string;
    return item.quality;
  }

  function canOpenInStudy(item: GenericDownloadItem): boolean {
    return (
      studyAvailable &&
      item.status === "complete" &&
      !!item.filePath &&
      (item.queueKind === "video" || item.queueKind === "audio")
    );
  }

  function openInStudy(filePath: string) {
    const parts = filePath.replace(/\\/g, "/").split("/");
    const name = parts[parts.length - 1] ?? "";
    const url = `/study/watch?path=${encodeURIComponent(filePath)}&name=${encodeURIComponent(name)}`;
    goto(url);
  }

  let downloads = $derived(getDownloads());
  let courseList = $derived(
    [...downloads.values()].filter((d): d is CourseDownloadItem => d.kind === "course")
  );
  let genericList = $derived(
    [...downloads.values()].filter((d): d is GenericDownloadItem => d.kind === "generic")
  );

  let grouped = $derived.by(() => {
    const active: GenericDownloadItem[] = [];
    const paused: GenericDownloadItem[] = [];
    const queued: GenericDownloadItem[] = [];
    const finished: GenericDownloadItem[] = [];
    const errored: GenericDownloadItem[] = [];
    const completed: GenericDownloadItem[] = [];
    for (const d of genericList) {
      if (d.status === "downloading" || d.status === "seeding") active.push(d);
      else if (d.status === "paused") paused.push(d);
      else if (d.status === "queued") queued.push(d);
      else {
        finished.push(d);
        if (d.status === "error") errored.push(d);
        else if (d.status === "complete") completed.push(d);
      }
    }
    return { active, paused, queued, finished, errored, completed };
  });

  type StatusFilter = "all" | "active" | "queued" | "completed" | "failed";
  let statusFilter = $state<StatusFilter>("all");

  let filterCounts = $derived({
    all: genericList.length,
    active: grouped.active.length + grouped.paused.length,
    queued: grouped.queued.length,
    completed: grouped.completed.length,
    failed: grouped.errored.length,
  });

  let showSection = $derived({
    active: statusFilter === "all" || statusFilter === "active",
    queued: statusFilter === "all" || statusFilter === "queued",
    completed: statusFilter === "all" || statusFilter === "completed",
    failed: statusFilter === "all" || statusFilter === "failed",
  });

  let finishedFiltered = $derived.by(() => {
    if (statusFilter === "completed") return grouped.completed;
    if (statusFilter === "failed") return grouped.errored;
    return grouped.finished;
  });

  const FINISHED_PAGE_SIZE = 20;
  let finishedVisibleCount = $state(FINISHED_PAGE_SIZE);

  let visibleFinished = $derived(
    finishedFiltered.length <= finishedVisibleCount
      ? finishedFiltered
      : finishedFiltered.slice(0, finishedVisibleCount)
  );

  let hasDownloads = $derived(courseList.length > 0 || genericList.length > 0);
  let finishedCount = $derived(getFinishedCount());
  let dlStats = $derived(getDownloadStats());

  async function cancelDownload(courseId: number) {
    try {
      await pluginInvoke("courses", "cancel_course_download", { courseId });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  async function cancelGenericDownload(id: number) {
    try {
      await invoke("cancel_generic_download", { downloadId: id });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  async function pauseDownload(id: number) {
    try {
      await invoke("pause_download", { downloadId: id });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  async function resumeDownload(id: number) {
    try {
      await invoke("resume_download", { downloadId: id });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  async function retryDownload(id: number) {
    try {
      await invoke("retry_download", { downloadId: id });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  let pendingRemove = $state<number | null>(null);
  let pendingRemoveTimer = $state<ReturnType<typeof setTimeout> | null>(null);

  function removeItem(id: number) {
    if (pendingRemove === id) {
      if (pendingRemoveTimer) clearTimeout(pendingRemoveTimer);
      pendingRemove = null;
      pendingRemoveTimer = null;
      invoke("remove_download", { downloadId: id }).catch((e: any) => {
        const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
        showToast("error", msg);
      });
    } else {
      if (pendingRemoveTimer) clearTimeout(pendingRemoveTimer);
      pendingRemove = id;
      pendingRemoveTimer = setTimeout(() => {
        pendingRemove = null;
        pendingRemoveTimer = null;
      }, 3000);
    }
  }

  async function removeItemWithFile(id: number) {
    if (!confirm($t("downloads.delete_file_confirm"))) return;
    try {
      await invoke("remove_download", { downloadId: id, deleteFile: true });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  async function clearFinished() {
    if (!confirm($t("downloads.clear_confirm"))) return;
    try {
      await invoke("clear_finished_downloads");
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  async function pauseAll() {
    try {
      await invoke("pause_all_downloads");
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  async function resumeAll() {
    try {
      await invoke("resume_all_downloads");
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  async function revealFile(path: string) {
    try {
      await invoke("reveal_file", { path });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  let reencodePath = $state<string | null>(null);

  function openReencode(path: string) {
    reencodePath = path;
  }

  let vopPath = $state<string | null>(null);

  type HistoryEntry = {
    id: number;
    url: string;
    platform: string;
    title: string;
    file_path: string | null;
    file_size_bytes: number | null;
    total_bytes: number | null;
    success: boolean;
    error: string | null;
    completed_at: number;
    thumbnail_url: string | null;
    kind: QueueKind | null;
  };

  let viewMode = $state<"active" | "history" | "tools">("active");
  let historyEntries = $state<HistoryEntry[]>([]);
  let historyLoading = $state(false);

  async function loadHistory() {
    historyLoading = true;
    try {
      historyEntries = await invoke<HistoryEntry[]>("get_download_history");
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    } finally {
      historyLoading = false;
    }
  }

  async function clearHistory() {
    if (!confirm($t("downloads.history_clear_confirm") as string)) return;
    try {
      await invoke("clear_download_history");
      historyEntries = [];
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  function toggleHistoryView() {
    if (viewMode === "history") {
      viewMode = "active";
    } else {
      viewMode = "history";
      loadHistory();
    }
  }

  function toggleToolsView() {
    viewMode = viewMode === "tools" ? "active" : "tools";
  }

  async function historyRetry(url: string, platform: string) {
    try {
      const settings = (await import("$lib/stores/settings-store.svelte")).getSettings();
      const outputDir = settings?.download.default_output_dir ?? "";
      if (!outputDir) {
        showToast("error", $t("common.error") as string);
        return;
      }
      await invoke("download_from_url", {
        url,
        outputDir,
        downloadMode: null,
        quality: settings?.download.video_quality ?? "best",
        formatId: null,
        referer: null,
      });
      viewMode = "active";
      showToast("info", $t("downloads.history_requeued") as string);
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  function fmtHistoryTime(unixSec: number): string {
    const ms = unixSec * 1000;
    const loc = get(i18nLocale) || "en";
    const lookup = loc.startsWith("pt") ? "pt" : "en";
    return timeAgo(ms, lookup);
  }

  function canPlayInStudyHistory(entry: HistoryEntry): boolean {
    return (
      studyAvailable &&
      entry.success &&
      !!entry.file_path &&
      (entry.kind === "video" || entry.kind === "audio")
    );
  }

  function openHistoryInStudy(filePath: string) {
    const parts = filePath.replace(/\\/g, "/").split("/");
    const name = parts[parts.length - 1] ?? "";
    const url = `/study/watch?path=${encodeURIComponent(filePath)}&name=${encodeURIComponent(name)}`;
    goto(url);
  }

  let dragId = $state<number | null>(null);
  let dropTargetId = $state<number | null>(null);
  let dropPosition = $state<"before" | "after">("before");

  function onDragStart(e: DragEvent, id: number) {
    dragId = id;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", String(id));
    }
  }

  function onDragOver(e: DragEvent, id: number) {
    if (dragId === null || dragId === id) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    const target = e.currentTarget as HTMLElement;
    const rect = target.getBoundingClientRect();
    const midpoint = rect.top + rect.height / 2;
    dropPosition = e.clientY < midpoint ? "before" : "after";
    dropTargetId = id;
  }

  function onDragLeave(e: DragEvent, id: number) {
    if (dropTargetId === id) {
      dropTargetId = null;
    }
  }

  async function onDrop(e: DragEvent, targetId: number) {
    e.preventDefault();
    const movingId = dragId;
    const position = dropPosition;
    dragId = null;
    dropTargetId = null;
    if (movingId === null || movingId === targetId) return;

    const order = grouped.queued.map((q) => q.id);
    const fromIdx = order.indexOf(movingId);
    if (fromIdx === -1) return;
    order.splice(fromIdx, 1);
    let targetIdx = order.indexOf(targetId);
    if (targetIdx === -1) return;
    if (position === "after") targetIdx += 1;
    order.splice(targetIdx, 0, movingId);

    try {
      await invoke<boolean>("reorder_queue", { ids: order });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  function onDragEnd() {
    dragId = null;
    dropTargetId = null;
  }

  function onKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    const tag = target?.tagName?.toLowerCase();
    const isEditable =
      tag === "input" ||
      tag === "textarea" ||
      tag === "select" ||
      target?.isContentEditable;
    if (isEditable) return;
    if (e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey && (e.key === "h" || e.key === "H")) {
      e.preventDefault();
      toggleHistoryView();
    }
  }

  onMount(() => {
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });
</script>

{#if hasDownloads || viewMode !== "active"}
  <div class="downloads-page">
    <div class="downloads-header">
      <div class="downloads-title-row">
        <h2 class="page-title">{$t('downloads.title')}</h2>
        {#if dlStats.totalDownloads > 0}
          <span class="downloads-stats">{$t('downloads.stats_line', { count: String(dlStats.totalDownloads), size: formatBytes(dlStats.totalBytes) })}</span>
        {/if}
      </div>
      <div class="bulk-actions">
        {#if viewMode === "active"}
          {#if grouped.active.length > 0}
            <button class="clear-btn" onclick={pauseAll} title={$t('downloads.pause_all') as string}>
              {$t('downloads.pause_all')}
            </button>
          {/if}
          {#if grouped.paused.length > 0}
            <button class="clear-btn" onclick={resumeAll} title={$t('downloads.resume_all') as string}>
              {$t('downloads.resume_all')}
            </button>
          {/if}
          {#if finishedCount > 0}
            <button class="clear-btn" onclick={clearFinished}>
              {$t('downloads.clear_finished')}
            </button>
          {/if}
        {:else if historyEntries.length > 0}
          <button class="clear-btn" onclick={clearHistory}>
            {$t('downloads.history_clear')}
          </button>
        {/if}
        <button
          class="history-toggle"
          class:on={viewMode === "history"}
          onclick={toggleHistoryView}
          aria-label={$t('downloads.history_toggle') as string}
          title={$t('downloads.history_toggle_hint') as string}
        >
          <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="9" />
            <polyline points="12 7 12 12 15 14" />
          </svg>
        </button>
        <button
          class="history-toggle"
          class:on={viewMode === "tools"}
          onclick={toggleToolsView}
          aria-label={$t('tools.tab') as string}
          title={$t('tools.tab') as string}
        >
          <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2.4-2.4z" />
          </svg>
        </button>
      </div>
    </div>

    {#if viewMode === "active"}
    <div class="filter-pills" role="tablist" aria-label={$t('downloads.filter_label')}>
      {#each [
        { value: 'all', labelKey: 'downloads.filter.all', count: filterCounts.all },
        { value: 'active', labelKey: 'downloads.filter.active', count: filterCounts.active },
        { value: 'queued', labelKey: 'downloads.filter.queued', count: filterCounts.queued },
        { value: 'completed', labelKey: 'downloads.filter.completed', count: filterCounts.completed },
        { value: 'failed', labelKey: 'downloads.filter.failed', count: filterCounts.failed },
      ] as pill}
        <button
          type="button"
          class="filter-pill"
          class:active={statusFilter === pill.value}
          role="tab"
          aria-selected={statusFilter === pill.value}
          onclick={() => { statusFilter = pill.value as StatusFilter; finishedVisibleCount = FINISHED_PAGE_SIZE; }}
        >
          <span>{$t(pill.labelKey)}</span>
          <span class="filter-count">{pill.count}</span>
        </button>
      {/each}
    </div>

    <div class="download-list">
      {#if showSection.active}
        {#each grouped.active as item (item.id)}
          {@render genericItem(item)}
        {/each}

        {#each grouped.paused as item (item.id)}
          {@render genericItem(item)}
        {/each}

        {#each courseList as item (item.id)}
          {@render courseItem(item)}
        {/each}
      {/if}

      {#if showSection.queued && grouped.queued.length > 0}
        <h5 class="section-label">
          {$t('downloads.section_queued')}
          {#if grouped.queued.length > 1}
            <span class="queue-reorder-hint">{$t('downloads.reorder_hint')}</span>
          {/if}
        </h5>
        {#each grouped.queued as item (item.id)}
          <div
            class="queue-drop-zone"
            class:drop-before={dropTargetId === item.id && dropPosition === "before"}
            class:drop-after={dropTargetId === item.id && dropPosition === "after"}
            class:dragging={dragId === item.id}
            draggable="true"
            role="listitem"
            ondragstart={(e) => onDragStart(e, item.id)}
            ondragover={(e) => onDragOver(e, item.id)}
            ondragleave={(e) => onDragLeave(e, item.id)}
            ondrop={(e) => onDrop(e, item.id)}
            ondragend={onDragEnd}
          >
            {@render genericItem(item)}
          </div>
        {/each}
      {/if}

      {#if (showSection.completed || showSection.failed) && finishedFiltered.length > 0}
        <h5 class="section-label">{$t('downloads.section_finished')}</h5>
        {#each visibleFinished as item (item.id)}
          {@render genericItem(item)}
        {/each}
        {#if finishedFiltered.length > finishedVisibleCount}
          <button
            class="button show-more-btn"
            onclick={() => { finishedVisibleCount += FINISHED_PAGE_SIZE; }}
          >
            {$t('downloads.show_more', { count: finishedFiltered.length - finishedVisibleCount })}
          </button>
        {/if}
      {/if}
    </div>
    {:else if viewMode === "history"}
      <div class="history-view">
        {#if historyLoading}
          <p class="history-empty">{$t('downloads.history_loading')}</p>
        {:else if historyEntries.length === 0}
          <div class="history-empty-state">
            <svg viewBox="0 0 24 24" width="36" height="36" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <circle cx="12" cy="12" r="9" />
              <polyline points="12 7 12 12 15 14" />
            </svg>
            <p class="history-empty-text">{$t('downloads.history_empty')}</p>
          </div>
        {:else}
          <ul class="history-list">
            {#each historyEntries as entry (entry.id)}
              <li class="history-item" data-success={entry.success}>
                <div class="history-item-head">
                  {#if entry.thumbnail_url}
                    <img
                      src={entry.thumbnail_url}
                      alt=""
                      class="queue-thumb"
                      loading="lazy"
                      onerror={(e) => { (e.target as HTMLImageElement).style.display = 'none'; }}
                    />
                  {/if}
                  <PlatformIcon platform={entry.platform} size={16} />
                  {#if entry.kind}
                    <QueueKindBadge kind={entry.kind} size={14} />
                  {/if}
                  <span class="history-title">{entry.title || entry.url}</span>
                  <span class="history-time">{fmtHistoryTime(entry.completed_at)}</span>
                </div>
                <div class="history-item-meta">
                  {#if entry.success && entry.file_size_bytes}
                    <span class="history-meta-chip">{formatBytes(entry.file_size_bytes)}</span>
                  {/if}
                  {#if !entry.success && entry.error}
                    <span class="history-meta-chip history-meta-error">{entry.error}</span>
                  {/if}
                </div>
                <div class="history-item-actions">
                  {#if canPlayInStudyHistory(entry) && entry.file_path}
                    <button
                      class="action-icon-btn"
                      onclick={() => openHistoryInStudy(entry.file_path!)}
                      aria-label={$t('downloads.open_in_study')}
                      title={$t('downloads.open_in_study')}
                    >
                      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <circle cx="12" cy="12" r="10" />
                        <polygon points="10 8 16 12 10 16 10 8" fill="currentColor" stroke="none" />
                      </svg>
                    </button>
                  {/if}
                  {#if entry.success && entry.file_path}
                    <button
                      class="action-icon-btn"
                      onclick={() => revealFile(entry.file_path!)}
                      aria-label={$t('downloads.open_folder')}
                      title={$t('downloads.open_folder')}
                    >
                      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
                      </svg>
                    </button>
                  {/if}
                  {#if entry.success && entry.file_path && entry.kind === "video"}
                    <button
                      class="action-icon-btn"
                      onclick={() => openReencode(entry.file_path!)}
                      aria-label={$t('reencode.action_label')}
                      title={$t('reencode.action_label')}
                    >
                      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <polyline points="4 14 10 14 10 20" />
                        <polyline points="20 10 14 10 14 4" />
                        <line x1="14" y1="10" x2="21" y2="3" />
                        <line x1="3" y1="21" x2="10" y2="14" />
                      </svg>
                    </button>
                  {/if}
                  <button
                    class="action-icon-btn"
                    onclick={() => historyRetry(entry.url, entry.platform)}
                    aria-label={$t('downloads.history_redownload')}
                    title={$t('downloads.history_redownload')}
                  >
                    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="23 4 23 10 17 10" />
                      <path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" />
                    </svg>
                  </button>
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {:else}
      <ToolsPanel />
    {/if}
  </div>
{:else}
  <div class="downloads-empty">
    <Mascot emotion="idle" />
    <p class="empty-text">{$t('downloads.empty')} <ContextHint text={$t('hints.downloads_empty')} dismissKey="downloads_empty" /></p>
    <div class="empty-links">
      <button class="history-link" onclick={toggleHistoryView}>
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="12" cy="12" r="9" />
          <polyline points="12 7 12 12 15 14" />
        </svg>
        {$t('downloads.history_view_link')}
      </button>
      <button class="history-link" onclick={toggleToolsView}>
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2.4-2.4z" />
        </svg>
        {$t('tools.tab')}
      </button>
    </div>
  </div>
{/if}

<ReencodeDialog bind:inputPath={reencodePath} />

{#if vopPath}
  <VideoOpsOverlay filePath={vopPath} onClose={() => (vopPath = null)} />
{/if}

{#snippet genericItem(item: GenericDownloadItem)}
  <div class="download-item" data-status={item.status}>
    <div class="item-header">
      <div class="item-header-left">
        {#if item.thumbnail_url}
          <img
            src={item.thumbnail_url}
            alt=""
            class="queue-thumb"
            loading="lazy"
            onerror={(e) => {
              (e.target as HTMLImageElement).style.display = 'none';
            }}
          />
        {/if}
        <PlatformIcon platform={item.platform} size={16} />
        <QueueKindBadge kind={item.queueKind} size={14} />
        <span class="item-name">{item.name}</span>
        {#if qualityChip(item)}
          <span class="quality-chip" title={$t('downloads.quality_hint')}>{qualityChip(item)}</span>
        {/if}
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
        {:else if item.status === "seeding"}
          {#if item.filePath}
            <button
              class="action-icon-btn"
              onclick={() => revealFile(item.filePath!)}
              aria-label={$t('downloads.open_folder')}
              title={$t('downloads.open_folder')}
            >
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
              </svg>
            </button>
          {/if}
          <button
            class="action-icon-btn"
            onclick={() => removeItem(item.id)}
            aria-label={$t('downloads.stop')}
            title={$t('downloads.stop')}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor">
              <rect x="6" y="6" width="12" height="12" />
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
            class:confirm-remove={pendingRemove === item.id}
            onclick={() => removeItem(item.id)}
            aria-label={pendingRemove === item.id ? $t('downloads.confirm_remove') : $t('common.close')}
            title={pendingRemove === item.id ? $t('downloads.confirm_remove') : undefined}
          >
            {#if pendingRemove === item.id}
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M5 12l5 5L20 7" />
              </svg>
            {:else}
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M18 6L6 18M6 6l12 12" />
              </svg>
            {/if}
          </button>
        {:else if item.status === "complete" && item.filePath}
          {#if canOpenInStudy(item)}
            <button
              class="action-icon-btn"
              onclick={() => openInStudy(item.filePath!)}
              aria-label={$t('downloads.open_in_study')}
              title={$t('downloads.open_in_study')}
            >
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="12" r="10" />
                <polygon points="10 8 16 12 10 16 10 8" fill="currentColor" stroke="none" />
              </svg>
            </button>
          {/if}
          <button
            class="action-icon-btn"
            onclick={() => revealFile(item.filePath!)}
            aria-label={$t('downloads.open_folder')}
            title={$t('downloads.open_folder')}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
            </svg>
          </button>
          {#if item.queueKind === "video"}
            <button
              class="action-icon-btn"
              onclick={() => openReencode(item.filePath!)}
              aria-label={$t('reencode.action_label')}
              title={$t('reencode.action_label')}
            >
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="4 14 10 14 10 20" />
                <polyline points="20 10 14 10 14 4" />
                <line x1="14" y1="10" x2="21" y2="3" />
                <line x1="3" y1="21" x2="10" y2="14" />
              </svg>
            </button>
          {/if}
          <button
            class="action-icon-btn"
            class:confirm-remove={pendingRemove === item.id}
            onclick={() => removeItem(item.id)}
            aria-label={pendingRemove === item.id ? $t('downloads.confirm_remove') : $t('common.close')}
            title={pendingRemove === item.id ? $t('downloads.confirm_remove') : undefined}
          >
            {#if pendingRemove === item.id}
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M5 12l5 5L20 7" />
              </svg>
            {:else}
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M18 6L6 18M6 6l12 12" />
              </svg>
            {/if}
          </button>
        {:else if item.status === "complete"}
          {#if item.filePath && item.queueKind === "video"}
            <button
              class="action-icon-btn"
              onclick={() => (vopPath = item.filePath!)}
              aria-label={$t('downloads.vop.action_label')}
              title={$t('downloads.vop.action_label')}
            >
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M12 2 2 7l10 5 10-5-10-5Z" />
                <path d="m2 17 10 5 10-5M2 12l10 5 10-5" />
              </svg>
            </button>
          {/if}
          {#if item.filePath}
            <button
              class="action-icon-btn"
              onclick={() => removeItemWithFile(item.id)}
              aria-label={$t('downloads.delete_file')}
              title={$t('downloads.delete_file')}
            >
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="3 6 5 6 21 6" />
                <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                <line x1="10" y1="11" x2="10" y2="17" />
                <line x1="14" y1="11" x2="14" y2="17" />
              </svg>
            </button>
          {/if}
          <button
            class="action-icon-btn"
            class:confirm-remove={pendingRemove === item.id}
            onclick={() => removeItem(item.id)}
            aria-label={pendingRemove === item.id ? $t('downloads.confirm_remove') : $t('common.close')}
            title={pendingRemove === item.id ? $t('downloads.confirm_remove') : undefined}
          >
            {#if pendingRemove === item.id}
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M5 12l5 5L20 7" />
              </svg>
            {:else}
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M18 6L6 18M6 6l12 12" />
              </svg>
            {/if}
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
      {#if item.phase === "preparing"}
        <span class="item-detail">{$t('downloads.phase_preparing')}</span>
      {:else if item.phase === "fetching_info"}
        <span class="item-detail">{$t('downloads.phase_fetching_info')}</span>
      {:else if item.phase === "starting"}
        <span class="item-detail">{$t('downloads.phase_starting')}</span>
      {:else if item.phase === "connecting"}
        <span class="item-detail">{$t('downloads.phase_connecting')}</span>
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
            {#if formatEta(item.etaSeconds)}
              <span class="stats-sep">&middot;</span>
              <span>ETA {formatEta(item.etaSeconds)}</span>
            {/if}
            <DownloadSpeedGraph points={getSpeedHistory(item.id)} />
          {/if}
        </div>
      {/if}
    {:else if item.status === "seeding"}
      <span class="item-detail">{item.platform.charAt(0).toUpperCase() + item.platform.slice(1)}</span>
      <div class="item-stats">
        {#if item.totalBytes}
          <span>{formatBytes(item.totalBytes)}</span>
          <span class="stats-sep">&middot;</span>
        {/if}
        {#if item.speed > 0}
          <span>{formatSpeed(item.speed)}</span>
          <DownloadSpeedGraph points={getSpeedHistory(item.id)} />
        {/if}
      </div>
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
      <span class="item-error">{translateBackendError(item.error, $t)}</span>
    {/if}

    {#if item.status !== "queued"}
      <div class="progress-track">
        <div
          class="progress-fill"
          data-status={item.status}
          style="width: {Math.max(0, item.percent).toFixed(1)}%"
        ></div>
      </div>
      <span class="item-percent">{Math.max(0, item.percent).toFixed(0)}%</span>
    {/if}

    {#if item.status !== "queued"}
      <DownloadLog id={item.id} />
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
        {#if item.speed > 0}
          <DownloadSpeedGraph points={getSpeedHistory(item.id)} />
        {/if}
      </div>
    {/if}

    {#if item.status === "complete" && item.bytesDownloaded > 0}
      <span class="item-detail">{formatBytes(item.bytesDownloaded)}</span>
    {/if}

    {#if item.status === "error" && item.error}
      <span class="item-error">{translateBackendError(item.error, $t)}</span>
    {/if}

    <div class="progress-track">
      <div
        class="progress-fill"
        data-status={item.status}
        style="width: {Math.max(0, item.percent).toFixed(1)}%"
      ></div>
    </div>

    <span class="item-percent">{Math.max(0, item.percent).toFixed(1)}%</span>

    {#if item.status !== "queued"}
      <DownloadLog id={item.id} />
    {/if}
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

  .downloads-title-row {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }

  .downloads-stats {
    font-size: 12px;
    color: var(--tertiary);
    font-weight: 400;
  }

  .bulk-actions {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 3);
    flex-shrink: 0;
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

  .filter-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: var(--padding);
  }

  .filter-pill {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    font-size: var(--text-sm);
    font-weight: 500;
    border: 1px solid var(--border);
    border-radius: var(--radius-full);
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out), border-color var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .filter-pill:hover {
      background: var(--surface-hi);
      color: var(--text);
    }
  }

  .filter-pill.active {
    background: var(--accent-soft);
    color: var(--accent);
    border-color: transparent;
  }

  @media (prefers-reduced-motion: reduce) {
    .filter-pill {
      transition: none;
    }
  }

  .filter-pill:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .filter-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 20px;
    padding: 0 5px;
    font-size: 10px;
    font-weight: 600;
    background: color-mix(in srgb, var(--surface) 70%, var(--bg-overlay));
    border-radius: 9999px;
  }

  .filter-pill.active .filter-count {
    background: color-mix(in srgb, var(--on-accent) 22%, transparent);
  }

  .download-list {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .download-item {
    background: var(--surface);
    border-radius: var(--radius-md);
    box-shadow: var(--elev-1);
    border-left: 3px solid transparent;
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    transition: transform var(--duration-fast) var(--ease-out), box-shadow var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .download-item:hover {
      transform: translateY(-1px);
      box-shadow: var(--elev-2);
    }
  }

  .download-item[data-status="downloading"] { border-left-color: var(--accent); }
  .download-item[data-status="seeding"]     { border-left-color: var(--success); }
  .download-item[data-status="complete"]    { border-left-color: var(--success); }
  .download-item[data-status="error"]       { border-left-color: var(--danger); }
  .download-item[data-status="paused"]      { border-left-color: var(--warning); }
  .download-item[data-status="queued"]      { opacity: 0.7; }

  @media (prefers-reduced-motion: reduce) {
    .download-item {
      transition: none;
    }
    .download-item:hover {
      transform: none;
    }
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

  .queue-thumb {
    width: 48px;
    height: 48px;
    border-radius: 6px;
    object-fit: cover;
    flex-shrink: 0;
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

  .action-icon-btn.confirm-remove {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 12%, transparent);
    animation: confirm-pulse 1s ease-in-out infinite;
  }

  @keyframes confirm-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.6; }
  }

  .item-status {
    font-size: var(--text-xs);
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-xs);
    flex-shrink: 0;
  }

  .item-status[data-status="downloading"] {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .item-status[data-status="seeding"],
  .item-status[data-status="complete"] {
    background: color-mix(in srgb, var(--success) 16%, transparent);
    color: var(--success);
  }

  .item-status[data-status="error"] {
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    color: var(--danger);
  }

  .item-status[data-status="queued"] {
    background: var(--surface-hi);
    color: var(--text-dim);
  }

  .item-status[data-status="paused"] {
    background: color-mix(in srgb, var(--warning) 16%, transparent);
    color: var(--warning);
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
    height: 4px;
    background: var(--surface-hi);
    border-radius: var(--radius-full);
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    border-radius: var(--radius-full);
    transition: width var(--duration-slow) var(--ease-out);
  }

  .progress-fill[data-status="downloading"] {
    background: var(--blue);
  }

  .progress-fill[data-status="seeding"] {
    background: var(--green);
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

  .quality-chip {
    padding: 1px 7px;
    font-size: 10.5px;
    font-weight: 600;
    line-height: 1.4;
    color: var(--secondary);
    background: var(--button-elevated);
    border: 1px solid var(--button-stroke);
    border-radius: 999px;
    letter-spacing: 0.2px;
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }

  .show-more-btn {
    align-self: center;
    font-size: 13px;
    padding: calc(var(--padding) / 2) var(--padding);
  }

  .history-toggle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: calc(var(--border-radius) / 2);
    color: var(--tertiary);
    cursor: pointer;
    flex-shrink: 0;
  }

  @media (hover: hover) {
    .history-toggle:hover {
      color: var(--secondary);
      background: var(--button-elevated);
    }
  }

  .history-toggle.on {
    color: var(--accent);
    background: var(--accent-soft, color-mix(in srgb, var(--accent) 12%, transparent));
    border-color: color-mix(in srgb, var(--accent) 25%, transparent);
  }

  .history-toggle:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .history-view {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .history-empty {
    color: var(--tertiary);
    font-size: 13px;
    padding: var(--padding) 0;
  }

  .history-empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--padding) / 2);
    padding: calc(var(--padding) * 3) var(--padding);
    color: var(--tertiary);
  }

  .history-empty-text {
    font-size: 13.5px;
    margin: 0;
  }

  .history-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .history-item {
    display: grid;
    grid-template-columns: 1fr auto;
    grid-template-areas:
      "head actions"
      "meta actions";
    column-gap: calc(var(--padding) / 2);
    align-items: center;
    padding: 8px 10px;
    border-radius: var(--border-radius);
    background: var(--button);
  }

  .history-item[data-success="false"] {
    background: color-mix(in srgb, var(--error, #c33) 6%, var(--button));
  }

  .history-item-head {
    grid-area: head;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .history-title {
    font-size: 13.5px;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }

  .history-time {
    font-size: 11.5px;
    color: var(--tertiary);
    flex-shrink: 0;
  }

  .history-item-meta {
    grid-area: meta;
    display: flex;
    gap: 6px;
    margin-top: 2px;
    padding-left: 24px;
  }

  .history-meta-chip {
    font-size: 11px;
    color: var(--tertiary);
    background: var(--button-elevated);
    padding: 2px 6px;
    border-radius: calc(var(--border-radius) / 2);
  }

  .history-meta-error {
    color: var(--error, #c33);
    background: color-mix(in srgb, var(--error, #c33) 12%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }

  .history-item-actions {
    grid-area: actions;
    display: flex;
    gap: 2px;
    align-items: center;
  }

  .empty-links {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    justify-content: center;
  }
  .history-link {
    margin-top: calc(var(--padding) / 2);
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12.5px;
    color: var(--tertiary);
    background: transparent;
    border: none;
    cursor: pointer;
    padding: 4px 6px;
    border-radius: calc(var(--border-radius) / 2);
  }

  @media (hover: hover) {
    .history-link:hover {
      color: var(--secondary);
      background: var(--button);
    }
  }

  .queue-reorder-hint {
    font-size: 10.5px;
    font-weight: 400;
    color: var(--tertiary);
    text-transform: none;
    letter-spacing: 0;
    margin-left: 8px;
  }

  .queue-drop-zone {
    position: relative;
    cursor: grab;
    transition: opacity 0.12s;
  }

  .queue-drop-zone:active {
    cursor: grabbing;
  }

  .queue-drop-zone.dragging {
    opacity: 0.4;
  }

  .queue-drop-zone.drop-before::before,
  .queue-drop-zone.drop-after::after {
    content: "";
    position: absolute;
    left: 8px;
    right: 8px;
    height: 2px;
    background: var(--accent);
    border-radius: 1px;
    pointer-events: none;
  }

  .queue-drop-zone.drop-before::before {
    top: -2px;
  }

  .queue-drop-zone.drop-after::after {
    bottom: -2px;
  }
</style>
