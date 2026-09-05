<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";
  import { setToolbar, type ToolbarAction } from "$lib/stores/toolbar-store.svelte";
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
    type StreamInfo,
  } from "$lib/stores/download-store.svelte";
  import { getDownloadStats } from "$lib/stores/download-stats.svelte";
  import PlatformIcon from "$components/icons/PlatformIcon.svelte";
  import QueueKindBadge from "$lib/study-components/QueueKindBadge.svelte";
  import Mascot from "$components/mascot/Mascot.svelte";
  import RootCauseHint from "$components/downloads/RootCauseHint.svelte";
  import DownloadSpeedGraph from "$components/download/DownloadSpeedGraph.svelte";
  import DownloadLog from "$components/download/DownloadLog.svelte";
  import DownloadPoster from "$components/download/DownloadPoster.svelte";
  import DownloadPhases from "$components/download/DownloadPhases.svelte";
  import DownloadCommand from "$components/download/DownloadCommand.svelte";
  import ReencodeDialog from "$components/dialog/ReencodeDialog.svelte";
  import ToolsPanel from "$components/downloads/ToolsPanel.svelte";
  import VideoOpsOverlay from "$components/downloads/VideoOpsOverlay.svelte";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
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

  function platformLabel(platform: string): string {
    if (!platform) return "";
    if (platform === "generic" || platform === "generic_ytdlp") return "Web";
    return platform.charAt(0).toUpperCase() + platform.slice(1);
  }

  // Rótulo do stream real ("1080p60 · mp4 · avc1"), montado do `info.*` que o
  // yt-dlp manda no template de progresso — é o que está descendo, não o que
  // foi pedido.
  function streamLabel(s: StreamInfo): string {
    const parts: string[] = [];
    const v = s.vcodec && s.vcodec !== "none" ? s.vcodec.split(".")[0] : null;
    const a = s.acodec && s.acodec !== "none" ? s.acodec.split(".")[0] : null;
    if (v && s.height) parts.push(`${s.height}p${s.fps && s.fps >= 48 ? Math.round(s.fps) : ""}`);
    else if (v) parts.push(s.format_note ?? s.format_id);
    else if (a) parts.push($t('omnibox.quality_audio') as string);
    if (s.ext) parts.push(s.ext);
    if (v) parts.push(v);
    else if (a) parts.push(a);
    return parts.join(" · ");
  }

  function formatChip(item: GenericDownloadItem): string | null {
    const all: StreamInfo[] = [...(item.streamsDone ?? []), ...(item.stream ? [item.stream] : [])];
    if (!all.length) return null;
    const video = all.find((s) => s.vcodec && s.vcodec !== "none");
    const audio = all.find((s) => s.acodec && s.acodec !== "none");
    if (video) {
      let label = streamLabel(video);
      if (audio && audio !== video) {
        const a = audio.acodec?.split(".")[0];
        if (a) label += ` + ${a}`;
      }
      return label;
    }
    return audio ? streamLabel(audio) : streamLabel(all[all.length - 1]);
  }

  // Tamanho anunciado pelos streams, quando o total real ainda não existe
  // (bv+ba: o áudio só revela o tamanho quando começa).
  function plannedSize(item: GenericDownloadItem): number | null {
    const all: StreamInfo[] = [...(item.streamsDone ?? []), ...(item.stream ? [item.stream] : [])];
    let sum = 0;
    for (const s of all) if (s.filesize) sum += s.filesize;
    return sum > 0 ? sum : null;
  }

  const NON_TRANSFER_PHASES = new Set([
    "preparing", "fetching_info", "starting", "connecting", "queued_starting",
    "waiting_rate_limit", "merging", "extracting_audio", "embedding_subtitles", "postprocessing",
  ]);

  function isTransferPhase(phase: string | undefined): boolean {
    return !phase || !NON_TRANSFER_PHASES.has(phase);
  }

  let clock = $state(Date.now());

  function elapsedLabel(item: GenericDownloadItem): string | null {
    if (!item.startedAtMs) return null;
    const s = Math.max(0, Math.floor((clock - item.startedAtMs) / 1000));
    if (s < 1) return null;
    const m = Math.floor(s / 60);
    const h = Math.floor(m / 60);
    const time = h > 0
      ? `${h}:${String(m % 60).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`
      : `${m}:${String(s % 60).padStart(2, "0")}`;
    return $t('downloads.detail.elapsed', { time }) as string;
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

  // The toolbar acts on the list beneath it: view switcher in the centre,
  // bulk actions trailing. Registered here so the shell renders them.
  $effect(() => {
    const actions: ToolbarAction[] = [];
    if (viewMode === "active") {
      if (grouped.active.length > 0) {
        actions.push({ id: "pause-all", label: $t("downloads.pause_all") as string, icon: "M8 5v14M16 5v14", onClick: pauseAll });
      }
      if (grouped.paused.length > 0) {
        actions.push({ id: "resume-all", label: $t("downloads.resume_all") as string, icon: "M7 4l13 8-13 8z", onClick: resumeAll });
      }
      if (finishedCount > 0) {
        actions.push({ id: "clear-finished", label: $t("downloads.clear_finished") as string, icon: "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3", onClick: clearFinished });
      }
    } else if (viewMode === "history" && historyEntries.length > 0) {
      actions.push({ id: "clear-history", label: $t("downloads.history_clear") as string, icon: "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3", onClick: clearHistory });
    }
    return setToolbar({
      segments: [
        { id: "active", label: $t("downloads.view_queue") as string, count: filterCounts.active },
        { id: "history", label: $t("downloads.view_history") as string },
        { id: "tools", label: $t("downloads.view_tools") as string },
      ],
      activeSegment: viewMode,
      onSegment: (id) => {
        if (id === "history") { if (viewMode !== "history") toggleHistoryView(); }
        else if (id === "tools") { if (viewMode !== "tools") toggleToolsView(); }
        else viewMode = "active";
      },
      actions,
    });
  });
  let historyEntries = $state<HistoryEntry[]>([]);
  let historyLoading = $state(false);

  async function openFileFolder(filePath: string) {
    try {
      await invoke("reveal_file", { path: filePath });
    } catch {
      try {
        await invoke("open_path_default", { path: filePath });
      } catch {}
    }
  }

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
    const tick = setInterval(() => { clock = Date.now(); }, 1000);
    return () => {
      window.removeEventListener("keydown", onKeydown);
      clearInterval(tick);
    };
  });

</script>

{#if hasDownloads || viewMode !== "active"}
  <div class="downloads-page downloads-mac">
    {#if viewMode === "active"}
    <div class="downloads-header">
      {#if dlStats.totalDownloads > 0}
        <span class="downloads-stats">{$t('downloads.stats_line', { count: String(dlStats.totalDownloads), size: formatBytes(dlStats.totalBytes) })}</span>
      {/if}
      <div class="bulk-actions">
        <label class="speed-limit-field">
          <span class="speed-limit-label">{$t('settings.download.speed_limit')}</span>
          <select
            class="speed-limit-selector"
            value={getSettings()?.download.speed_limit || "unlimited"}
            onchange={(e) => {
              const val = e.currentTarget.value;
              updateSettings({ download: { speed_limit: val === "unlimited" ? "" : val } });
            }}
          >
            <option value="unlimited">{$t('downloads.speed_unlimited')}</option>
            <option value="1M">1 MB/s</option>
            <option value="2M">2 MB/s</option>
            <option value="5M">5 MB/s</option>
            <option value="10M">10 MB/s</option>
          </select>
        </label>
      </div>
    </div>
    {/if}

    {#if viewMode === "active"}
    <div class="filter-pills segmented" role="tablist" aria-label={$t('downloads.filter_label')}>
      {#each [
        { value: 'all', labelKey: 'downloads.filter.all', count: filterCounts.all },
        { value: 'active', labelKey: 'downloads.filter.active', count: filterCounts.active },
        { value: 'queued', labelKey: 'downloads.filter.queued', count: filterCounts.queued },
        { value: 'completed', labelKey: 'downloads.filter.completed', count: filterCounts.completed },
        { value: 'failed', labelKey: 'downloads.filter.failed', count: filterCounts.failed },
      ] as pill}
        <button
          type="button"
          class="filter-pill segmented-btn"
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
            <img class="empty-state-art" src="/emoji/hourglass_not_done.png" alt="" width="72" height="72" draggable="false" />
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
  <div class="downloads-empty downloads-mac-empty">
    <Mascot emotion="idle" />
    <div class="empty-copy">
      <p class="empty-text">{$t('downloads.empty')}</p>
      <p class="empty-hint">{$t('downloads.empty_hint')}</p>
    </div>
    <a href="/" class="btn btn-primary btn-lg">{$t('downloads.empty_cta')}</a>
    {#if dlStats.totalDownloads > 0}
      <p class="empty-value">{$t('downloads.stats_line', { count: String(dlStats.totalDownloads), size: formatBytes(dlStats.totalBytes) })}</p>
    {/if}
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
  <div class="download-item" data-status={item.status} data-phase={item.phase}>
    <div class="item-row">
      <DownloadPoster
        src={item.thumbnail_url}
        kind={item.queueKind}
        loading={item.status === "downloading" && (item.phase === "fetching_info" || item.phase === "preparing" || item.phase === "queued_starting")}
        durationSeconds={item.durationSeconds}
        size={item.status === "downloading" || item.status === "paused" || item.status === "seeding" ? "md" : "sm"}
      />
      <div class="item-body">
        <div class="item-header">
          <div class="item-header-left">
            <span class="item-name" title={item.name}>{item.name}</span>
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
              {#if item.filePath}
                <button
                  class="action-icon-btn"
                  onclick={() => openFileFolder(item.filePath!)}
                  aria-label={$t('downloads.open_folder')}
                  title={$t('downloads.open_folder')}
                >
                  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                  </svg>
                </button>
              {/if}
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

        <div class="item-meta">
          <PlatformIcon platform={item.platform} size={14} />
          <span class="meta-text">{platformLabel(item.platform)}</span>
          {#if item.author}
            <span class="meta-sep" aria-hidden="true">·</span>
            <span class="meta-text meta-author" title={item.author}>{item.author}</span>
          {/if}
          <QueueKindBadge kind={item.queueKind} size={13} />
          {#if formatChip(item)}
            <span class="format-chip" title={$t('downloads.quality_hint')}>{formatChip(item)}</span>
          {:else if qualityChip(item)}
            <span class="quality-chip" title={$t('downloads.quality_hint')}>{qualityChip(item)}</span>
          {/if}
          {#if item.status === "complete" && (item.totalBytes || item.downloadedBytes)}
            <span class="meta-sep" aria-hidden="true">·</span>
            <span class="meta-text">{formatBytes(item.totalBytes ?? item.downloadedBytes)}</span>
          {:else if item.status === "downloading" && !item.totalBytes && plannedSize(item)}
            <span class="meta-sep" aria-hidden="true">·</span>
            <span class="meta-text">≈ {formatBytes(plannedSize(item)!)}</span>
          {/if}
        </div>

        {#if item.status === "downloading"}
          <DownloadPhases
            phase={item.phase}
            plannedFormats={item.plannedFormats}
            stream={item.stream}
            streamsDone={item.streamsDone}
            fragmentIndex={item.fragmentIndex}
            fragmentCount={item.fragmentCount}
            command={item.command}
            downloadMode={item.downloadMode}
          />
          {#if isTransferPhase(item.phase)}
            <div class="item-stats">
              {#if item.downloadedBytes > 0}
                <span>{formatBytes(item.downloadedBytes)}{#if item.totalBytes}{" / "}{formatBytes(item.totalBytes)}{/if}</span>
                <span class="stats-sep">&middot;</span>
              {/if}
              {#if item.speed > 0}
                <span class="stat-speed">{formatSpeed(item.speed)}</span>
                {#if formatEta(item.etaSeconds)}
                  <span class="stats-sep">&middot;</span>
                  <span class="eta-pill">ETA {formatEta(item.etaSeconds)}</span>
                {/if}
              {/if}
              {#if elapsedLabel(item)}
                <span class="stats-sep">&middot;</span>
                <span>{elapsedLabel(item)}</span>
              {/if}
              {#if item.speed > 0}
                <DownloadSpeedGraph points={getSpeedHistory(item.id)} />
              {/if}
            </div>
          {/if}
        {:else if item.status === "seeding"}
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
          {#if item.downloadedBytes > 0}
            <div class="item-stats">
              <span>{formatBytes(item.downloadedBytes)}{#if item.totalBytes}{" / "}{formatBytes(item.totalBytes)}{/if}</span>
            </div>
          {/if}
        {/if}

        {#if item.status === "error" && item.error}
          <RootCauseHint error={item.error} onRetry={() => retryDownload(item.id)} />
        {/if}

        {#if item.status !== "queued" && item.status !== "complete"}
          <div class="progress-row">
            <div class="progress-track">
              <div
                class="progress-fill"
                data-status={item.status}
                class:indeterminate={item.status === "downloading" && item.percent <= 0 && !isTransferPhase(item.phase)}
                style:width="{Math.max(0, item.percent).toFixed(1)}%"
              ></div>
            </div>
            <span class="item-percent">{Math.max(0, item.percent).toFixed(0)}%</span>
          </div>
        {/if}

        {#if item.status !== "queued"}
          <DownloadLog id={item.id} status={item.status} />
          <DownloadCommand id={item.id} command={item.command} status={item.status} />
        {/if}
      </div>
    </div>
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
      <RootCauseHint error={item.error} />
    {/if}

    <div class="progress-track">
      <div
        class="progress-fill"
        data-status={item.status}
        style:width="{Math.max(0, item.percent).toFixed(1)}%"
      ></div>
    </div>

    <span class="item-percent">{Math.max(0, item.percent).toFixed(1)}%</span>

    {#if item.status !== "queued"}
      <DownloadLog id={item.id} />
    {/if}
  </div>
{/snippet}

<style>
  /* ---------- Empty state ---------- */

  .downloads-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    min-height: 0;
    gap: var(--space-4);
    color: var(--text-dim);
  }

  .empty-copy {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
    text-align: center;
  }

  .empty-text {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: 600;
    letter-spacing: var(--track-snug);
    color: var(--text);
  }

  .empty-hint {
    margin: 0;
    max-width: 380px;
    font-size: var(--text-base);
    line-height: var(--leading-base);
    color: var(--text-dim);
  }

  .empty-value {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .empty-links {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .history-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-full);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
  }

  @media (hover: hover) {
    .history-link:hover {
      background: var(--fill-1);
      color: var(--text);
    }
  }

  /* ---------- Page ---------- */

  .downloads-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5) var(--space-6);
    max-width: 880px;
    margin: 0 auto;
    width: 100%;
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }

  .downloads-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-height: 28px;
  }

  .downloads-stats {
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .bulk-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-left: auto;
  }

  .speed-limit-field {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .speed-limit-label {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .speed-limit-selector {
    height: 24px;
    font-size: var(--text-sm);
    padding-left: var(--space-2);
    border-radius: 5px;
  }

  .section-label {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    margin: var(--space-3) 0 0;
    padding: 0 var(--space-1);
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: 0;
    text-transform: none;
    color: var(--text-dim);
  }

  .section-label:first-child {
    margin-top: 0;
  }

  .queue-reorder-hint {
    font-size: var(--text-xs);
    font-weight: 400;
    color: var(--text-faint);
  }

  .filter-pills {
    align-self: flex-start;
  }

  .filter-pill {
    gap: 6px;
  }

  .filter-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    font-size: var(--text-caption);
    font-weight: 600;
    line-height: 1;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .filter-pill.active .filter-count {
    background: var(--accent-soft);
    color: var(--accent-hi);
  }

  /* ---------- List (grouped rows) ---------- */

  .download-list {
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  /* One row of title + actions, then one wrapped meta line (platform · size · speed),
     then the progress bar. Everything that is not the header flows inline. */
  .download-item {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    column-gap: 6px;
    row-gap: var(--space-2);
    padding: var(--space-3) var(--space-3);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    border-radius: var(--radius-lg);
    transition: background var(--duration-fast) var(--ease-out);
    contain: content;
  }

  .download-item + .download-item,
  .queue-drop-zone + .queue-drop-zone .download-item,
  .queue-drop-zone + .download-item,
  .download-item + .queue-drop-zone .download-item {
    margin-top: var(--space-2);
  }

  @media (hover: hover) {
    .download-item:hover {
      background: var(--surface-hi);
    }
  }

  .download-item[data-status="queued"] .item-name {
    color: var(--text-muted);
  }

  .item-header {
    flex-basis: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-height: 24px;
  }

  .item-detail + .item-detail::before,
  .item-detail + .item-stats::before,
  .item-stats + .item-detail::before {
    content: "·";
    color: var(--text-faint);
    margin-right: 6px;
  }

  .download-item > :global(.root-cause),
  .download-item > :global(.download-log) {
    flex-basis: 100%;
  }

  .download-item > :global(.download-log) {
    margin-top: 0;
  }

  .download-item[data-status="complete"] .progress-track {
    display: none;
  }

  .item-header-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }

  .item-header-left {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    flex: 1;
  }

  .queue-thumb {
    width: 44px;
    height: 28px;
    object-fit: cover;
    border-radius: 5px;
    flex-shrink: 0;
    background: var(--fill-2);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .item-name {
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--text);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .action-icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0;
    transition: background var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .action-icon-btn:hover {
      background: var(--fill-2);
      color: var(--text);
    }
  }

  .action-icon-btn:active {
    background: var(--fill-3);
  }

  .action-icon-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }

  .action-icon-btn svg {
    pointer-events: none;
    width: 15px;
    height: 15px;
  }

  .action-icon-btn.confirm-remove {
    background: var(--danger);
    color: var(--on-status);
    animation: confirm-pulse var(--duration-slow) var(--ease-out);
  }

  @keyframes confirm-pulse {
    from { transform: scale(0.9); }
    to { transform: scale(1); }
  }

  .item-status {
    display: inline-flex;
    align-items: center;
    height: 20px;
    padding: 0 var(--space-2);
    margin-left: var(--space-1);
    font-size: var(--text-xs);
    font-weight: 600;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-muted);
    white-space: nowrap;
  }

  .item-status[data-status="downloading"] {
    background: var(--accent-soft);
    color: var(--accent-hi);
  }

  .item-status[data-status="complete"] {
    background: color-mix(in srgb, var(--success) 14%, transparent);
    color: var(--success);
  }

  .item-status[data-status="error"] {
    background: color-mix(in srgb, var(--danger) 14%, transparent);
    color: var(--danger);
  }

  .item-status[data-status="queued"] {
    background: var(--fill-1);
    color: var(--text-dim);
  }

  .item-status[data-status="paused"] {
    background: color-mix(in srgb, var(--warning) 14%, transparent);
    color: var(--warning);
  }

  .item-detail {
    font-size: var(--text-sm);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item-stats {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .stats-sep {
    opacity: 0.5;
  }

  .phase-merging-badge {
    color: var(--accent-hi);
    font-weight: 500;
  }

  .eta-pill {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: 500;
  }

  .progress-track {
    flex: 1 1 200px;
    min-width: 120px;
    height: 4px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    overflow: hidden;
    margin-top: 2px;
  }

  .download-item[data-status="complete"] .item-percent {
    display: none;
  }

  .progress-fill {
    height: 100%;
    border-radius: var(--radius-full);
    background: var(--accent);
    transition: width var(--duration-base) var(--ease-out);
  }

  .progress-fill[data-status="downloading"] {
    background: var(--accent);
  }

  .progress-fill[data-status="seeding"] {
    background: var(--info);
  }

  .progress-fill[data-status="complete"] {
    background: var(--success);
  }

  .progress-fill[data-status="error"] {
    background: var(--danger);
  }

  .progress-fill[data-status="paused"] {
    background: var(--text-dim);
  }

  .item-percent {
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    min-width: 32px;
    text-align: right;
  }

  .quality-chip {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 6px;
    border-radius: 4px;
    background: var(--fill-1);
    color: var(--text-dim);
    font-size: var(--text-caption);
    font-weight: 600;
    flex-shrink: 0;
    letter-spacing: 0.02em;
  }

  .show-more-btn {
    align-self: center;
    margin-top: var(--space-2);
  }

  /* ---------- History ---------- */

  .history-view {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .history-empty {
    color: var(--text-dim);
    font-size: var(--text-base);
    text-align: center;
    padding: var(--space-6) 0;
  }

  .history-empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8) var(--space-4);
    color: var(--text-faint);
    text-align: center;
  }

  .history-empty-text {
    margin: 0;
    max-width: 380px;
    font-size: var(--text-base);
    line-height: var(--leading-base);
    color: var(--text-dim);
  }

  .history-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
  }

  .history-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-2) var(--space-3);
    min-height: var(--row-base);
    position: relative;
    transition: background var(--duration-fast) var(--ease-out);
  }

  .history-item + .history-item::before {
    content: "";
    position: absolute;
    top: 0;
    left: var(--space-3);
    right: 0;
    height: var(--hairline);
    background: var(--separator);
  }

  @media (hover: hover) {
    .history-item:hover {
      background: var(--fill-1);
    }
  }

  .history-item[data-success="false"] .history-title {
    color: var(--text-muted);
  }

  .history-item-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .history-title {
    flex: 1;
    min-width: 0;
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .history-time {
    flex-shrink: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .history-item-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    min-height: 0;
  }

  .history-item-meta:empty {
    display: none;
  }

  .history-meta-chip {
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .history-meta-error {
    color: var(--danger);
  }

  .history-item-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-left: auto;
  }

  /* ---------- Drag reorder ---------- */

  .queue-drop-zone {
    position: relative;
    border-radius: var(--radius-lg);
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
    left: var(--space-2);
    right: var(--space-2);
    height: 2px;
    border-radius: 1px;
    background: var(--accent);
    pointer-events: none;
  }

  .queue-drop-zone.drop-before::before {
    top: -5px;
  }

  .queue-drop-zone.drop-after::after {
    bottom: -5px;
  }

  @media (prefers-reduced-motion: reduce) {
    .download-item,
    .action-icon-btn.confirm-remove {
      transition: none;
      animation: none;
    }
  }

  /* ---------- Card do item (poster + corpo) ---------- */
  .download-item {
    display: block;
    padding: var(--space-3);
  }

  .item-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    min-width: 0;
  }

  .item-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .item-body .item-header {
    flex-basis: auto;
    min-height: 26px;
  }

  .item-meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .meta-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta-author {
    max-width: 220px;
    color: var(--text-muted);
  }

  .meta-sep {
    color: var(--text-faint);
  }

  .format-chip {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 6px;
    border-radius: 4px;
    background: var(--accent-soft);
    color: var(--accent-hi);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: 0.02em;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .stat-speed {
    color: var(--text);
    font-weight: 500;
  }

  .progress-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .progress-row .progress-track {
    flex: 1;
    margin-top: 0;
  }

  .progress-fill.indeterminate {
    width: 30% !important;
    animation: progress-indeterminate 1.4s ease-in-out infinite;
  }

  @keyframes progress-indeterminate {
    0% { transform: translateX(-100%); }
    100% { transform: translateX(340%); }
  }

  @media (prefers-reduced-motion: reduce) {
    .progress-fill.indeterminate { animation: none; width: 8% !important; }
  }

  .download-item[data-status="error"] .item-name {
    color: var(--text);
  }

  @media (max-width: 640px) {
    .item-row { gap: var(--space-2); }
    .item-row :global(.poster) { width: 84px; }
    .item-body :global(.phase-rail) { display: none; }
    .meta-author { max-width: 120px; }
  }
</style>
