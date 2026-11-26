<script lang="ts">
  import { onMount } from "svelte";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "$lib/capture/api";
  import { fileName, folderOf, parseError } from "$lib/capture/record";
  import { formatBytes, formatTimecode } from "$lib/capture/timecode";
  import {
    clamp,
    hasOverlaps,
    keyframeAtOrBefore,
    markIn,
    markOut,
    mergeOverlaps,
    move,
    remove,
    snap,
    stepKeyframe,
    toRequestSpans,
    totalLength,
    updateSegment,
    type Segment,
  } from "$lib/capture/segments";
  import type { ExportEvent, ExportMode, Platform, ProbeResult, Span } from "$lib/capture/types";
  import StatusIcon from "#components/converter/StatusIcon.svelte";
  import ErrorNotice from "./ErrorNotice.svelte";
  import Timeline from "./Timeline.svelte";
  import SegmentList from "./SegmentList.svelte";

  type Phase = "empty" | "probing" | "ready" | "exporting" | "done";

  let {
    path = $bindable(null),
    lastRecording = null,
    platform = "other",
    active = true,
    onhistory,
  }: {
    path?: string | null;
    lastRecording?: string | null;
    platform?: Platform;
    active?: boolean;
    onhistory: () => void;
  } = $props();

  let phase = $state<Phase>("empty");
  let info = $state<ProbeResult | null>(null);
  let loadError = $state<string | null>(null);
  let exportError = $state<string | null>(null);
  let notice = $state<{ text: string; tone: "info" | "warn" } | null>(null);
  let root = $state<HTMLDivElement | null>(null);
  let loadedPath: string | null = null;

  let video = $state<HTMLVideoElement | null>(null);
  let videoError = $state(false);
  let playing = $state(false);
  let current = $state(0);

  let segments = $state<Segment[]>([]);
  let pendingIn = $state<number | null>(null);
  let selectedId = $state<string | null>(null);

  let snapKeys = $state(true);
  let mode = $state<ExportMode>("merge");
  let precise = $state(false);
  let stripAudio = $state(false);
  let outDir = $state<string | null>(null);

  let jobId: string | null = null;
  let percent = $state(0);
  let step = $state(0);
  let steps = $state(0);
  let cancelling = $state(false);
  let outputs = $state<string[]>([]);
  let outSize = $state(0);
  let snapping = $state(false);
  let dragging = $state(false);
  let raf = 0;

  let duration = $derived(info?.duration ?? 0);
  let keyframes = $derived(info?.keyframes ?? []);
  let fps = $derived(info?.video?.fps ?? 30);
  let src = $derived(info ? convertFileSrc(info.path) : "");
  let overlaps = $derived(hasOverlaps(segments));
  let spans = $derived(toRequestSpans(segments, duration));
  let keptLength = $derived(totalLength(spans));
  let lossyStartGap = $derived.by(() => {
    if (precise || keyframes.length === 0) return 0;
    let gap = 0;
    for (const s of spans) {
      const k = keyframeAtOrBefore(s.start, keyframes);
      if (k !== null) gap = Math.max(gap, s.start - k);
    }
    return gap;
  });
  let exportBlocked = $derived(spans.length === 0 ? "capture.export_need_segment" : null);
  let announcement = $derived.by(() => {
    if (phase === "exporting") return $t("capture.exporting", { percent: Math.floor(percent / 10) * 10, step, steps }) as string;
    if (phase === "done") return (outputs.length === 1 ? $t("capture.export_done_one") : $t("capture.export_done_many", { count: outputs.length })) as string;
    return notice?.text ?? "";
  });
  let exportLabel = $derived(
    mode === "merge" || spans.length <= 1 ? ($t("capture.export_merged") as string) : ($t("capture.export_separate", { count: spans.length }) as string),
  );

  $effect(() => {
    const next = path;
    if (next && next !== loadedPath) void load(next);
  });

  function resetCut() {
    segments = [];
    pendingIn = null;
    selectedId = null;
    current = 0;
    playing = false;
    videoError = false;
    outputs = [];
    exportError = null;
    notice = null;
  }

  async function load(p: string) {
    if (phase === "exporting") return;
    loadedPath = p;
    resetCut();
    loadError = null;
    info = null;
    phase = "probing";
    try {
      const result = await api.probe(p);
      if (loadedPath !== p) return;
      info = result;
      phase = "ready";
    } catch (e) {
      if (loadedPath !== p) return;
      loadError = api.errorText(e);
      phase = "empty";
    }
  }

  function closeFile() {
    if (phase === "exporting") return;
    path = null;
    loadedPath = null;
    info = null;
    loadError = null;
    resetCut();
    phase = "empty";
  }

  async function chooseFile() {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      filters: [{ name: $t("capture.video_files") as string, extensions: ["mp4", "mov", "mkv", "webm", "m4v", "avi", "ts", "mts", "flv", "wmv"] }],
    }).catch(() => null);
    const p = Array.isArray(picked) ? picked[0] : picked;
    if (typeof p === "string") path = p;
  }

  async function chooseFolder() {
    const picked = await openDialog({ directory: true, multiple: false, defaultPath: outDir ?? (info ? folderOf(info.path) : undefined) }).catch(() => null);
    if (typeof picked === "string") outDir = picked;
  }

  function seek(time: number) {
    const next = clamp(time, duration);
    current = next;
    if (video && !videoError) video.currentTime = next;
  }

  function follow() {
    cancelAnimationFrame(raf);
    const loop = () => {
      if (video && !video.paused) {
        current = video.currentTime;
        raf = requestAnimationFrame(loop);
      }
    };
    raf = requestAnimationFrame(loop);
  }

  function togglePlay() {
    if (!video || videoError) return;
    if (video.paused) void video.play().catch(() => {});
    else video.pause();
  }

  function setIn() {
    if (phase !== "ready") return;
    const time = snap(current, keyframes, snapKeys);
    ({ segments, pendingIn } = markIn({ segments, pendingIn }, time));
    notice = { text: $t("capture.in_set", { time: formatTimecode(time) }) as string, tone: "info" };
  }

  function setOut() {
    if (phase !== "ready") return;
    const time = snap(current, keyframes, snapKeys);
    const result = markOut({ segments, pendingIn }, time, api.newId);
    if (result.error) {
      notice = { text: $t(result.error === "before_in" ? "capture.out_before_in" : "capture.out_too_short") as string, tone: "warn" };
      return;
    }
    segments = result.marks.segments;
    pendingIn = result.marks.pendingIn;
    selectedId = result.added?.id ?? null;
    notice = { text: $t("capture.segment_added", { num: segments.length }) as string, tone: "info" };
  }

  function wholeFile() {
    if (phase !== "ready" || duration <= 0) return;
    const seg = { id: api.newId(), start: 0, end: duration };
    segments = [...segments, seg];
    pendingIn = null;
    selectedId = seg.id;
  }

  function selectSeg(seg: Segment) {
    selectedId = seg.id;
    seek(seg.start);
  }

  function editSeg(id: string, patch: Partial<Span>) {
    const snapped: Partial<Span> = {};
    if (patch.start !== undefined) snapped.start = snap(patch.start, keyframes, snapKeys);
    if (patch.end !== undefined) snapped.end = snap(patch.end, keyframes, snapKeys);
    segments = updateSegment(segments, id, snapped, duration);
  }

  function removeSeg(id: string) {
    segments = remove(segments, id);
    if (selectedId === id) selectedId = null;
  }

  function fixOverlaps() {
    segments = mergeOverlaps(segments);
    selectedId = null;
  }

  async function takeSnapshot() {
    if (!info || snapping) return;
    snapping = true;
    try {
      const out = await api.snapshot(info.path, current, outDir);
      notice = { text: $t("capture.snapshot_saved", { name: fileName(out) }) as string, tone: "info" };
      onhistory();
    } catch (e) {
      const parsed = parseError(api.errorText(e));
      notice = { text: $t("capture.snapshot_failed", { error: parsed.detail || parsed.code }) as string, tone: "warn" };
    } finally {
      snapping = false;
    }
  }

  async function runExport() {
    if (!info || exportBlocked || phase !== "ready") return;
    video?.pause();
    exportError = null;
    outputs = [];
    percent = 0;
    step = 1;
    steps = mode === "merge" && spans.length > 1 ? spans.length + 1 : spans.length;
    cancelling = false;
    const id = api.newId();
    jobId = id;
    phase = "exporting";
    try {
      const result = await api.exportCut(id, {
        path: info.path,
        segments: spans,
        mode,
        precise,
        strip_audio: stripAudio,
        out_dir: outDir,
      });
      outputs = result.outputs;
      outSize = result.size_bytes;
      phase = "done";
      onhistory();
    } catch (e) {
      const text = api.errorText(e);
      exportError = parseError(text).code === "CAPTURE_CANCELLED" ? null : text;
      if (!exportError) notice = { text: $t("capture.export_cancelled") as string, tone: "info" };
      phase = "ready";
    } finally {
      if (jobId === id) jobId = null;
      cancelling = false;
    }
  }

  async function cancelExport() {
    if (!jobId || cancelling) return;
    cancelling = true;
    await api.cancel(jobId).catch(() => {});
  }

  function onExportEvent(event: ExportEvent) {
    if (!jobId || event.job_id !== jobId) return;
    if (event.state === "running") {
      percent = Math.max(percent, Math.min(100, event.percent));
      step = event.step;
      steps = event.steps;
    }
  }

  function backToEdit() {
    outputs = [];
    phase = "ready";
  }

  function typing(target: EventTarget | null): boolean {
    const el = target as HTMLElement | null;
    if (!el) return false;
    const tag = el.tagName;
    return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable;
  }

  function ours(target: EventTarget | null): boolean {
    const el = target as HTMLElement | null;
    if (!el || el === document.body || el === document.documentElement) return true;
    return !!root && root.contains(el) && !el.closest("[role=tablist]");
  }

  function onKey(event: KeyboardEvent) {
    if (!active || phase !== "ready" || typing(event.target) || !ours(event.target) || event.metaKey || event.ctrlKey || event.altKey) return;
    const k = event.key;
    const onButton = (event.target as HTMLElement | null)?.tagName === "BUTTON";
    if (k === "i" || k === "I") setIn();
    else if (k === "o" || k === "O") setOut();
    else if (k === " ") {
      if (onButton) return;
      togglePlay();
    } else if (k === "ArrowLeft" || k === "ArrowRight") {
      const dir = k === "ArrowLeft" ? -1 : 1;
      if (event.shiftKey) {
        const next = stepKeyframe(current, keyframes, dir);
        if (next !== null) seek(next);
      } else {
        video?.pause();
        seek(current + dir / (fps || 30));
      }
    } else if (k === "Home") seek(0);
    else if (k === "End") seek(duration);
    else if ((k === "Delete" || k === "Backspace") && selectedId && !onButton) removeSeg(selectedId);
    else if (k === "Escape" && pendingIn !== null) pendingIn = null;
    else return;
    event.preventDefault();
  }

  onMount(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];
    const keep = (fn: () => void) => (disposed ? fn() : cleanups.push(fn));
    void api.onExport(onExportEvent).then(keep).catch(() => {});
    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          if (!active) return;
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter" || p.type === "over") dragging = phase !== "exporting";
          else if (p.type === "leave") dragging = false;
          else if (p.type === "drop") {
            dragging = false;
            const first = p.paths?.[0];
            if (first && phase !== "exporting") path = first;
          }
        })
        .then(keep)
        .catch(() => {});
    } catch {
      dragging = false;
    }
    return () => {
      disposed = true;
      cancelAnimationFrame(raf);
      cleanups.forEach((fn) => fn());
    };
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="cut" class:dragging bind:this={root}>
  <p class="cut-sr" role="status" aria-live="polite" aria-atomic="true">{announcement}</p>
  {#if loadError}
    <ErrorNotice error={loadError} {platform} retryLabel={$t("capture.choose_other")} onretry={chooseFile} ondismiss={() => (loadError = null)} />
  {/if}

  {#if phase === "empty"}
    <div class="cut-drop">
      <svg class="cut-drop-art" viewBox="0 0 48 48" width="48" height="48" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <rect x="6" y="12" width="36" height="24" rx="3" />
        <path d="M18 12v24M30 12v24" stroke-dasharray="3 3" />
        <path d="M21 21l6 3-6 3z" />
      </svg>
      <h2 class="cut-drop-title">{$t("capture.cut_empty_title")}</h2>
      <p class="cut-drop-body">{$t("capture.cut_empty_body")}</p>
      <div class="cut-drop-actions">
        <button type="button" class="btn btn-primary" onclick={chooseFile}>{$t("capture.choose_video")}</button>
        {#if lastRecording}
          <button type="button" class="btn btn-secondary" onclick={() => (path = lastRecording)}>{$t("capture.open_last")}</button>
        {/if}
      </div>
    </div>
  {:else if phase === "probing"}
    <div class="cut-drop cut-drop-busy">
      <span class="spinner" aria-hidden="true"></span>
      <p class="cut-drop-body">{$t("capture.probing")}</p>
    </div>
  {:else if info}
    <div class="cut-file">
      <div class="cut-file-copy">
        <span class="cut-file-name" title={info.path}>{info.name}</span>
        <span class="cut-file-meta">
          {formatTimecode(info.duration)} · {info.video ? `${info.video.width}×${info.video.height} ${info.video.codec}` : ""}{info.video?.fps ? ` · ${$t("capture.fps_value", { fps: Math.round(info.video.fps * 100) / 100 })}` : ""} · {formatBytes(info.size_bytes)} · {$t("capture.keyframe_count", { count: keyframes.length })}
        </span>
      </div>
      <button type="button" class="btn btn-ghost btn-sm" disabled={phase === "exporting"} onclick={chooseFile}>{$t("capture.open_other")}</button>
      <button type="button" class="btn btn-ghost btn-sm" disabled={phase === "exporting"} onclick={closeFile}>{$t("capture.close_file")}</button>
    </div>

    <div class="cut-player">
      {#if videoError}
        <div class="cut-noplay">
          <StatusIcon name="warning" size={18} />
          <p>{$t("capture.no_preview")}</p>
        </div>
      {:else}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video
          bind:this={video}
          {src}
          preload="metadata"
          onplay={() => {
            playing = true;
            follow();
          }}
          onpause={() => (playing = false)}
          onseeked={() => video && (current = video.currentTime)}
          onerror={() => (videoError = true)}
        ></video>
      {/if}
    </div>

    <div class="cut-transport">
      <button type="button" class="btn btn-secondary btn-sm" disabled={videoError || phase !== "ready"} onclick={togglePlay}>
        {playing ? $t("capture.pause") : $t("capture.play")}
        <kbd class="kbd">{$t("capture.key_space")}</kbd>
      </button>
      <span class="cut-now" aria-live="off">{formatTimecode(current)} / {formatTimecode(duration)}</span>
      <span class="cut-spacer"></span>
      <button type="button" class="btn btn-secondary btn-sm" disabled={phase !== "ready"} onclick={setIn}>{$t("capture.set_in")} <kbd class="kbd">I</kbd></button>
      <button type="button" class="btn btn-secondary btn-sm" disabled={phase !== "ready"} onclick={setOut}>{$t("capture.set_out")} <kbd class="kbd">O</kbd></button>
      <button type="button" class="btn btn-ghost btn-sm" disabled={phase !== "ready"} onclick={wholeFile}>{$t("capture.whole_file")}</button>
      <button type="button" class="btn btn-ghost btn-sm" disabled={snapping || phase === "exporting"} onclick={takeSnapshot}>
        {snapping ? $t("capture.snapshot_saving") : $t("capture.snapshot")}
      </button>
    </div>

    <Timeline {duration} {current} {keyframes} {segments} {pendingIn} {selectedId} onseek={seek} onselect={(id) => (selectedId = id)} />
    <p class="cut-keys">{$t("capture.keys_hint")}</p>

    {#if notice}
      <p class="cut-notice" class:warn={notice.tone === "warn"}>
        <StatusIcon name={notice.tone === "warn" ? "warning" : "ready"} size={14} />
        <span>{notice.text}</span>
      </p>
    {/if}

    {#if exportError}
      <ErrorNotice error={exportError} {platform} retryLabel={$t("capture.try_again")} onretry={runExport} ondismiss={() => (exportError = null)} />
    {/if}

    {#if phase === "done"}
      <div class="cut-done">
        <StatusIcon name="done" size={20} />
        <div class="cut-done-copy">
          <h3>{outputs.length === 1 ? $t("capture.export_done_one") : $t("capture.export_done_many", { count: outputs.length })}</h3>
          <ul>
            {#each outputs as out (out)}
              <li>{fileName(out)}</li>
            {/each}
          </ul>
          <p class="cut-done-meta">{formatBytes(outSize)} · {folderOf(outputs[0] ?? "")}</p>
          <div class="cut-done-actions">
            <button type="button" class="btn btn-primary" onclick={() => outputs[0] && void api.reveal(outputs[0]).catch(() => {})}>{$t("capture.show_in_folder")}</button>
            <button type="button" class="btn btn-secondary" onclick={() => outputs[0] && (path = outputs[0])}>{$t("capture.open_result")}</button>
            <button type="button" class="btn btn-ghost" onclick={backToEdit}>{$t("capture.keep_editing")}</button>
          </div>
        </div>
      </div>
    {:else}
      <section class="cut-block">
        <div class="cut-block-head">
          <h2>{$t("capture.segments")}</h2>
          {#if segments.length}
            <span class="cut-block-sub">{$t("capture.kept_total", { time: formatTimecode(keptLength) })}</span>
          {/if}
        </div>
        {#if segments.length === 0}
          <p class="cut-hint">{$t("capture.no_segments")}</p>
        {:else}
          <SegmentList
            {segments}
            {selectedId}
            disabled={phase === "exporting"}
            onselect={selectSeg}
            onedit={editSeg}
            onmove={(from, to) => (segments = move(segments, from, to))}
            onremove={removeSeg}
          />
          {#if overlaps}
            <p class="cut-notice warn">
              <StatusIcon name="warning" size={14} />
              <span>{$t("capture.overlap_warning")}</span>
              <button type="button" class="btn btn-ghost btn-sm" disabled={phase === "exporting"} onclick={fixOverlaps}>{$t("capture.merge_overlaps")}</button>
            </p>
          {/if}
        {/if}
      </section>

      <fieldset class="cut-opts" disabled={phase === "exporting"}>
        <div class="cut-opt">
          <span class="field-label" id="cut-mode">{$t("capture.export_as")}</span>
          <div class="mac-segmented" role="radiogroup" aria-labelledby="cut-mode">
            <button type="button" role="radio" aria-checked={mode === "merge"} class="mac-segmented-btn" class:active={mode === "merge"} onclick={() => (mode = "merge")}>{$t("capture.mode_merge")}</button>
            <button type="button" role="radio" aria-checked={mode === "separate"} class="mac-segmented-btn" class:active={mode === "separate"} onclick={() => (mode = "separate")}>{$t("capture.mode_separate")}</button>
          </div>
        </div>
        <label class="cut-check">
          <input type="checkbox" class="checkbox" bind:checked={snapKeys} />
          <span>
            <span class="field-label">{$t("capture.snap_keyframes")}</span>
            <span class="field-hint">{$t("capture.snap_keyframes_hint")}</span>
          </span>
        </label>
        <label class="cut-check">
          <input type="checkbox" class="checkbox" bind:checked={precise} />
          <span>
            <span class="field-label">{$t("capture.precise")}</span>
            <span class="field-hint">{$t("capture.precise_hint")}</span>
          </span>
        </label>
        <label class="cut-check">
          <input type="checkbox" class="checkbox" bind:checked={stripAudio} disabled={!info.audio.length} />
          <span>
            <span class="field-label">{$t("capture.strip_audio")}</span>
            <span class="field-hint">{info.audio.length ? $t("capture.strip_audio_hint") : $t("capture.no_audio_track")}</span>
          </span>
        </label>
        <div class="cut-out">
          <span class="field-label">{$t("capture.save_to")}</span>
          <span class="cut-path" title={outDir ?? folderOf(info.path)}>{outDir ?? $t("capture.next_to_source")}</span>
          <button type="button" class="btn btn-secondary btn-sm" onclick={chooseFolder}>{$t("capture.choose_folder")}</button>
          {#if outDir}
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => (outDir = null)}>{$t("capture.use_source_folder")}</button>
          {/if}
        </div>
      </fieldset>

      {#if lossyStartGap > 0.05 && !snapKeys}
        <p class="cut-hint">{$t("capture.lossless_gap", { secs: lossyStartGap.toFixed(2) })}</p>
      {/if}

      <footer class="cut-bar">
        {#if phase === "exporting"}
          <div class="cut-progress">
            <span class="cut-progress-text">{$t("capture.exporting", { percent: Math.round(percent), step, steps })}</span>
            <div class="progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(percent)} aria-label={$t("capture.export_progress")}>
              <div class="cut-progress-fill" style:width="{percent}%"></div>
            </div>
          </div>
          <button type="button" class="btn btn-secondary" disabled={cancelling} onclick={cancelExport}>{cancelling ? $t("capture.cancelling") : $t("capture.cancel_export")}</button>
        {:else}
          <p class="cut-bar-hint">{exportBlocked ? $t(exportBlocked) : precise ? $t("capture.precise_note") : $t("capture.lossless_note")}</p>
          <button type="button" class="btn btn-primary" disabled={!!exportBlocked} onclick={runExport}>{exportLabel}</button>
        {/if}
      </footer>
    {/if}
  {/if}

  {#if dragging && phase !== "empty"}
    <div class="cut-dropping" aria-hidden="true"><p>{$t("capture.drop_to_open")}</p></div>
  {/if}
</div>

<style>
  .cut-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }

  .cut {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .cut-drop {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    min-height: 300px;
    padding: var(--space-7) var(--space-5);
    border: 1.5px dashed var(--content-border);
    border-radius: var(--radius-xl);
    background: var(--surface);
    text-align: center;
    transition: border-color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }

  .cut.dragging .cut-drop {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .cut-drop-busy {
    border-style: solid;
  }

  .cut-drop-art {
    color: var(--text-dim);
  }

  .cut-drop-title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .cut-drop-body {
    margin: 0;
    max-width: 440px;
    font-size: var(--text-base);
    color: var(--text-dim);
  }

  .cut-drop-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2);
  }

  .cut-file {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .cut-file-copy {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .cut-file-name {
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cut-file-meta {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .cut-player {
    display: flex;
    align-items: center;
    justify-content: center;
    aspect-ratio: 16 / 9;
    max-height: 420px;
    border-radius: var(--radius-lg);
    background: var(--bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
  }

  .cut-player video {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .cut-noplay {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    max-width: 420px;
    padding: var(--space-4);
    color: var(--warning);
  }

  .cut-noplay p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .cut-transport {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .cut-transport .kbd {
    margin-inline-start: var(--space-1);
  }

  .cut-now {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .cut-spacer {
    flex: 1;
  }

  .cut-keys,
  .cut-hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .cut-notice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .cut-notice.warn {
    color: var(--text);
  }

  .cut-block {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .cut-block-head {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
  }

  .cut-block-head h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .cut-block-sub {
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .cut-opts {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: var(--space-3) var(--space-5);
    margin: 0;
    padding: var(--space-4);
    border: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cut-opt {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .cut-check {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    cursor: pointer;
  }

  .cut-check > span {
    display: flex;
    flex-direction: column;
  }

  .cut-out {
    grid-column: 1 / -1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
  }

  .cut-path {
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cut-bar {
    position: sticky;
    bottom: 0;
    z-index: 1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) 0;
    background: var(--pane-bg);
  }

  .cut-bar-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .cut-progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    flex: 1;
    min-width: 200px;
  }

  .cut-progress-text {
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .cut-progress-fill {
    height: 100%;
    border-radius: inherit;
    background: var(--accent);
    transition: width var(--duration-base) var(--ease-out);
  }

  .cut-done {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--success);
  }

  .cut-done-copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    color: var(--text);
  }

  .cut-done-copy h3 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
  }

  .cut-done-copy ul {
    margin: 0;
    padding-inline-start: var(--space-4);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .cut-done-meta {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }

  .cut-done-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .cut-dropping {
    position: absolute;
    inset: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px dashed var(--accent);
    border-radius: var(--radius-xl);
    background: color-mix(in srgb, var(--pane-bg) 82%, transparent);
    pointer-events: none;
  }

  .cut-dropping p {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  @media (prefers-reduced-motion: reduce) {
    .cut-drop,
    .cut-progress-fill {
      transition: none;
    }
  }
</style>
