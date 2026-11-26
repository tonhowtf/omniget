<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { t } from "$lib/i18n";
  import * as api from "$lib/studio/api";
  import * as session from "$lib/studio/session";
  import { introLines, outputSize } from "$lib/studio/frame";
  import { loadBundledFont, tokenSwatches } from "$lib/studio/fonts";
  import { activeSegmentIndex, layoutCues } from "$lib/studio/layout";
  import { buildRequest, captionsUsable, introUsable, nothingToChange, stemOf, type PlanInput } from "$lib/studio/plan";
  import { applyPreset } from "$lib/studio/presets";
  import { formatClock, fromRaw, parseSubtitles, toSrt, toVtt } from "$lib/studio/subtitles";
  import type {
    AsrProgress,
    AsrStatus,
    CaptionStyle,
    IntroSettings,
    Probe,
    Quality,
    Reframe,
    RenderOutcome,
    RenderProgress,
    RenderStage,
    Segment,
    Tools,
    TranscribeProgress,
    Transcript,
  } from "$lib/studio/types";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import StatusIcon from "#components/converter/StatusIcon.svelte";
  import Preview from "#components/studio/Preview.svelte";
  import TranscriptPanel, { type AsrState } from "#components/studio/TranscriptPanel.svelte";
  import StylePanel from "#components/studio/StylePanel.svelte";
  import FramePanel from "#components/studio/FramePanel.svelte";

  type Phase = "empty" | "probing" | "error" | "ready";
  type Tab = "captions" | "style" | "frame";
  type RenderPhase = "idle" | "rendering" | "done" | "failed" | "cancelled";
  type Notice = { kind: "success" | "error"; text: string; path?: string } | null;

  const VIDEO_EXTS = ["mp4", "mov", "m4v", "mkv", "webm", "avi", "mts", "m2ts", "wmv", "flv", "3gp", "mpg", "mpeg", "ts"];
  const TABS: Tab[] = ["captions", "style", "frame"];
  const QUALITIES: Quality[] = ["fast", "balanced", "high"];

  let phase = $state<Phase>("empty");
  let probeError = $state<string | null>(null);
  let probe = $state<Probe | null>(null);
  let src = $state<string | null>(null);
  let tools = $state<Tools | null>(null);
  let dragging = $state(false);
  let tab = $state<Tab>("captions");
  let time = $state(0);
  let preview = $state<ReturnType<typeof Preview> | null>(null);
  let swatches = $state<string[]>([]);
  let notice = $state<Notice>(null);

  let segments = $state<Segment[]>([]);
  let burnCaptions = $state(true);
  let asr = $state<AsrStatus | null>(null);
  let asrState = $state<AsrState>("idle");
  let asrPercent = $state(0);
  let asrError = $state<string | null>(null);
  let model = $state("whisper-base");
  let lang = $state("auto");
  let transcribeJob: string | null = null;
  const downloadParts = new Map<string, { done: number; total: number }>();

  let style = $state<CaptionStyle>(applyPreset("bold_pop"));
  let reframe = $state<Reframe>({ aspect: "9:16", mode: "crop", position: 0.5 });
  let intro = $state<IntroSettings>({ enabled: false, seconds: 3, title: "", subtitle: "", background: "#1C1C1E", color: "#FFFFFF", fade: true });
  let showIntro = $state(false);
  let fadeOut = $state(false);
  let quality = $state<Quality>("balanced");
  let outDir = $state<string | null>(null);

  let renderPhase = $state<RenderPhase>("idle");
  let renderPercent = $state(0);
  let renderStage = $state<RenderStage>("preparing");
  let renderError = $state<string | null>(null);
  let outcome = $state<RenderOutcome | null>(null);
  let stopping = $state(false);
  let renderJob: string | null = null;

  let rendering = $derived(renderPhase === "rendering");
  let asrBusy = $derived(asrState === "downloading" || asrState === "transcribing");
  let locked = $derived(rendering);
  let canBurn = $derived(tools?.subtitles ?? false);
  let canDrawText = $derived(tools?.drawtext ?? false);
  let out = $derived(probe ? outputSize(probe.width, probe.height, reframe.aspect, reframe.mode) : ([1920, 1080] as [number, number]));
  let cues = $derived(layoutCues(segments, { maxChars: style.maxChars, maxLines: style.maxLines, uppercase: style.uppercase }));
  let activeIndex = $derived(activeSegmentIndex(segments, time));
  let introView = $derived.by(() => {
    if (!intro.enabled) return null;
    const lines = introLines(out[0], out[1], intro.title, intro.subtitle);
    return { title: lines.title, subtitle: lines.subtitle, background: intro.background, color: intro.color };
  });
  let plan = $derived<PlanInput | null>(
    probe ? { probe, segments, burnCaptions: burnCaptions && canBurn, style, reframe, intro: { ...intro, enabled: intro.enabled && canDrawText }, fadeOut, quality, outDir } : null,
  );
  let idle = $derived(plan ? nothingToChange(plan) : true);
  let renderBlock = $derived.by(() => {
    if (!tools) return $t("studio.block_checking") as string;
    if (!tools.ffmpeg) return $t("studio.block_ffmpeg") as string;
    if (asrState === "downloading") return $t("studio.block_download") as string;
    if (asrBusy) return $t("studio.block_asr") as string;
    if (idle) return $t("studio.block_nothing") as string;
    return null;
  });
  let summary = $derived.by(() => {
    if (!plan) return [] as string[];
    const parts: string[] = [];
    if (plan.burnCaptions && captionsUsable(segments)) parts.push($t("studio.sum_captions", { count: cues.length }) as string);
    if (reframe.aspect !== "keep") parts.push($t(reframe.mode === "crop" ? "studio.sum_crop" : "studio.sum_fit", { aspect: reframe.aspect }) as string);
    if (introUsable(plan.intro)) parts.push($t("studio.sum_intro", { num: intro.seconds }) as string);
    if (fadeOut) parts.push($t("studio.sum_fade") as string);
    return parts;
  });
  let liveText = $derived.by(() => {
    if (phase === "probing") return $t("studio.probing") as string;
    if (rendering) return $t(`studio.stage_${renderStage}`, { percent: session.milestone(renderPercent) }) as string;
    if (renderPhase === "done") return $t("studio.render_done") as string;
    if (renderPhase === "failed") return $t("studio.render_failed") as string;
    return "";
  });

  let openBlock = $derived(asrState === "downloading" ? ($t("studio.block_download") as string) : asrBusy ? ($t("studio.block_asr") as string) : undefined);

  async function openVideo(path: string) {
    if (rendering || asrBusy) return;
    transcribeJob = null;
    phase = "probing";
    probeError = null;
    notice = null;
    try {
      const p = await api.probe(path);
      probe = p;
      src = api.mediaSrc(p.path);
      segments = [];
      time = 0;
      renderPhase = "idle";
      outcome = null;
      renderError = null;
      asrError = null;
      asrState = "idle";
      if (p.width < p.height && reframe.aspect === "9:16") reframe = { ...reframe, aspect: "keep" };
      tab = "captions";
      phase = "ready";
    } catch (e) {
      probeError = api.errorText(e);
      phase = probe ? "ready" : "error";
      if (probe) notice = { kind: "error", text: probeError };
    }
  }

  async function chooseVideo() {
    const picked = await openDialog({ multiple: false, directory: false, filters: [{ name: $t("studio.filter_videos") as string, extensions: VIDEO_EXTS }] }).catch(() => null);
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (typeof path === "string") await openVideo(path);
  }

  async function chooseFolder() {
    const picked = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof picked === "string") outDir = picked;
  }

  async function refreshAsr() {
    try {
      asr = await api.asrStatus();
      const current = asr.models.find((m) => m.id === model);
      if (!current) model = asr.default_model;
    } catch {
      asr = null;
    }
  }

  async function downloadModel() {
    if (asrBusy) return;
    await followDownload(session.trackDownload(model, api.asrInstall(model)));
  }

  async function followDownload(entry: session.PendingDownload) {
    model = entry.model;
    asrState = "downloading";
    asrPercent = 0;
    asrError = null;
    downloadParts.clear();
    try {
      asr = await entry.promise;
      asrState = "idle";
    } catch (e) {
      asrError = api.errorText(e);
      asrState = "error";
      await refreshAsr();
    }
  }

  async function transcribe() {
    if (!probe || asrBusy) return;
    const job = api.newJobId("tr");
    const path = probe.path;
    const promise = api.transcribe(job, path, lang === "auto" ? null : lang, model, probe.duration_ms);
    await followTranscribe(session.trackTranscribe(job, path, promise));
  }

  async function followTranscribe(entry: session.PendingTranscribe) {
    transcribeJob = entry.job;
    asrState = "transcribing";
    asrPercent = 0;
    asrError = null;
    let result: Transcript | null = null;
    let failure: string | null = null;
    try {
      result = await entry.promise;
    } catch (e) {
      failure = api.errorText(e);
    }
    void refreshAsr();
    if (transcribeJob !== entry.job) return;
    transcribeJob = null;
    asrState = "idle";
    if (probe?.path !== entry.path) return;
    if (failure !== null || !result) {
      asrError = failure ?? "";
      asrState = "error";
      return;
    }
    const list = fromRaw(result.segments);
    segments = list;
    if (list.length === 0) {
      asrError = $t("studio.no_speech") as string;
      asrState = "error";
    } else {
      burnCaptions = canBurn;
      notice = { kind: "success", text: $t("studio.transcribed", { count: list.length }) as string };
    }
  }

  async function importSubs() {
    const picked = await openDialog({ multiple: false, directory: false, filters: [{ name: $t("studio.filter_subs") as string, extensions: ["srt", "vtt"] }] }).catch(() => null);
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (typeof path !== "string") return;
    try {
      const list = parseSubtitles(await api.readText(path));
      if (list.length === 0) {
        notice = { kind: "error", text: $t("studio.import_empty") as string };
        return;
      }
      segments = list;
      burnCaptions = canBurn;
      notice = { kind: "success", text: $t("studio.imported", { count: list.length }) as string };
    } catch (e) {
      notice = { kind: "error", text: $t("studio.import_failed", { error: api.errorText(e) }) as string };
    }
  }

  async function exportSubs(format: "srt" | "vtt") {
    if (!probe) return;
    const name = `${stemOf(probe.name)}.${format}`;
    const dir = probe.path.slice(0, Math.max(probe.path.lastIndexOf("/"), probe.path.lastIndexOf("\\")) + 1);
    const path = await saveDialog({ defaultPath: `${dir}${name}`, filters: [{ name: format.toUpperCase(), extensions: [format] }] }).catch(() => null);
    if (typeof path !== "string") return;
    try {
      await api.saveText(path, format === "srt" ? toSrt(segments) : toVtt(segments));
      notice = { kind: "success", text: $t("studio.exported", { name: path.split(/[\\/]/).pop() ?? name }) as string, path };
    } catch (e) {
      notice = { kind: "error", text: $t("studio.export_failed", { error: api.errorText(e) }) as string };
    }
  }

  async function render() {
    if (!plan || renderBlock || rendering) return;
    const request = buildRequest(plan);
    const job = api.newJobId("rd");
    renderJob = job;
    renderPhase = "rendering";
    renderPercent = 0;
    renderStage = "preparing";
    renderError = null;
    outcome = null;
    stopping = false;
    notice = null;
    try {
      outcome = await api.render(job, request);
      renderPhase = "done";
    } catch (e) {
      const message = api.errorText(e);
      if (stopping || message === "Cancelled.") {
        renderPhase = "cancelled";
      } else {
        renderError = message;
        renderPhase = "failed";
      }
    } finally {
      renderJob = null;
      stopping = false;
    }
  }

  async function stop() {
    if (!renderJob || stopping) return;
    stopping = true;
    await api.cancel(renderJob).catch(() => {});
  }

  function onRender(e: RenderProgress) {
    if (e.job_id !== renderJob) return;
    renderStage = e.stage;
    if (Number.isFinite(e.percent)) renderPercent = Math.max(renderPercent, Math.min(100, e.percent));
  }

  function onTranscribe(e: TranscribeProgress) {
    if (e.job_id !== transcribeJob) return;
    if (Number.isFinite(e.percent)) asrPercent = Math.max(asrPercent, Math.min(99, e.percent));
  }

  function onAsr(e: AsrProgress) {
    if (asrState !== "downloading") return;
    const key = `${e.id}`;
    downloadParts.set(key, { done: e.done, total: e.total ?? 0 });
    let done = 0;
    let total = 0;
    for (const part of downloadParts.values()) {
      done += part.done;
      total += part.total;
    }
    const expected = asr?.models.find((m) => m.id === model)?.bytes ?? 0;
    const denom = Math.max(total, expected, 1);
    asrPercent = Math.max(asrPercent, Math.min(99, Math.floor((done / denom) * 100)));
  }

  function onTabKey(e: KeyboardEvent, index: number) {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const next = (index + (e.key === "ArrowRight" ? 1 : TABS.length - 1)) % TABS.length;
    tab = TABS[next];
    document.getElementById(`st-tab-${TABS[next]}`)?.focus();
  }

  function seek(ms: number) {
    preview?.seek(ms);
  }

  function reset() {
    if (rendering || asrBusy) return;
    probe = null;
    src = null;
    segments = [];
    phase = "empty";
    renderPhase = "idle";
    outcome = null;
    notice = null;
  }

  function videoPath(paths: string[]): string | null {
    return paths.find((p) => VIDEO_EXTS.includes(p.split(".").pop()?.toLowerCase() ?? "")) ?? paths[0] ?? null;
  }

  onMount(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];
    const keep = (fn: () => void) => {
      if (disposed) fn();
      else cleanups.push(fn);
    };
    void loadBundledFont();
    swatches = tokenSwatches();
    void api.tools().then((v) => (tools = v)).catch(() => (tools = { ffmpeg: false, subtitles: false, drawtext: false }));
    void refreshAsr();
    const download = session.pendingDownload();
    if (download) void followDownload(download);
    const pending = session.pendingTranscribe();
    if (pending) {
      void openVideo(pending.path).then(() => {
        if (!disposed && probe?.path === pending.path) void followTranscribe(pending);
      });
    }
    void api.onRender(onRender).then(keep).catch(() => {});
    void api.onTranscribe(onTranscribe).then(keep).catch(() => {});
    void api.onAsr(onAsr).then(keep).catch(() => {});
    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter" || p.type === "over") dragging = !rendering && !asrBusy;
          else if (p.type === "leave") dragging = false;
          else if (p.type === "drop") {
            dragging = false;
            const path = videoPath(p.paths ?? []);
            if (path) void openVideo(path);
          }
        })
        .then(keep)
        .catch(() => {});
    } catch {
      dragging = false;
    }
    return () => {
      disposed = true;
      if (renderJob) void api.cancel(renderJob).catch(() => {});
      cleanups.forEach((fn) => fn());
    };
  });
</script>

<section class="st" class:dragging aria-busy={phase === "probing" || rendering}>
  <header class="st-head">
    <NavIcon icon="studio" size={56} />
    <div class="st-head-copy">
      <h1>{$t("studio.title")}</h1>
      <p class="st-subtitle">{$t("studio.subtitle")}</p>
    </div>
    {#if phase === "ready" && probe}
      <button type="button" class="btn btn-secondary st-head-action" disabled={rendering || asrBusy} title={openBlock} onclick={chooseVideo}>{$t("studio.open_another")}</button>
    {/if}
  </header>

  <p class="st-live" role="status" aria-live="polite">{liveText}</p>

  {#if tools && !tools.ffmpeg}
    <div class="st-notice" role="alert">
      <StatusIcon name="failed" />
      <div class="st-notice-copy">
        <p><strong>{$t("studio.ffmpeg_missing_title")}</strong> {$t("studio.ffmpeg_missing_body")}</p>
      </div>
      <button type="button" class="btn btn-secondary btn-sm" onclick={() => goto("/settings")}>{$t("studio.open_settings")}</button>
    </div>
  {/if}

  {#if notice}
    <div class="st-notice" class:ok={notice.kind === "success"} role={notice.kind === "error" ? "alert" : "status"}>
      <StatusIcon name={notice.kind === "success" ? "done" : "warning"} />
      <div class="st-notice-copy"><p>{notice.text}</p></div>
      {#if notice.path}
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => notice?.path && api.reveal(notice.path).catch(() => {})}>{$t("studio.show_in_folder")}</button>
      {/if}
      <button type="button" class="btn btn-ghost btn-sm" aria-label={$t("studio.dismiss")} onclick={() => (notice = null)}>
        <svg viewBox="0 0 12 12" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9" /></svg>
      </button>
    </div>
  {/if}

  {#if phase === "empty" || phase === "error"}
    <div class="st-drop" class:err={phase === "error"}>
      <svg class="st-drop-art" viewBox="0 0 48 48" width="48" height="48" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <rect x="7" y="10" width="34" height="24" rx="3" />
        <path d="M14 40h20M21 18l7 4-7 4z" />
        <path d="M13 29h8M24 29h11" />
      </svg>
      {#if phase === "error"}
        <h2 class="st-drop-title">{$t("studio.probe_failed")}</h2>
        <p class="st-drop-body" role="alert">{probeError}</p>
        <button type="button" class="btn btn-primary" onclick={chooseVideo}>{$t("studio.choose_another")}</button>
      {:else}
        <h2 class="st-drop-title">{$t("studio.empty_title")}</h2>
        <p class="st-drop-body">{$t("studio.empty_body")}</p>
        <button type="button" class="btn btn-primary" onclick={chooseVideo}>{$t("studio.choose_video")}</button>
        <ul class="st-drop-steps">
          <li>{$t("studio.step_captions")}</li>
          <li>{$t("studio.step_reframe")}</li>
          <li>{$t("studio.step_intro")}</li>
        </ul>
      {/if}
    </div>
  {:else if phase === "probing" && !probe}
    <div class="st-drop st-drop-busy">
      <span class="spinner" aria-hidden="true"></span>
      <p class="st-drop-body">{$t("studio.probing")}</p>
    </div>
  {:else if probe}
    <div class="st-meta">
      <span class="st-meta-name" title={probe.path}>{probe.name}</span>
      <span>{formatClock(probe.duration_ms)}</span>
      <span>{probe.width}×{probe.height}</span>
      <span>{api.formatBytes(probe.size_bytes)}</span>
      <span class="st-meta-audio" class:none={!probe.has_audio}>
        <StatusIcon name={probe.has_audio ? "done" : "warning"} size={12} />
        {probe.has_audio ? $t("studio.has_audio") : $t("studio.no_audio")}
      </span>
      {#if phase === "probing"}<span class="spinner st-meta-spin" aria-hidden="true"></span>{/if}
    </div>

    <div class="st-work">
      <div class="st-preview">
        <Preview
          bind:this={preview}
          bind:time
          {src}
          name={probe.name}
          durationMs={probe.duration_ms}
          outW={out[0]}
          outH={out[1]}
          {reframe}
          {cues}
          {style}
          showCaptions={burnCaptions || !canBurn}
          intro={introView}
          showIntro={showIntro && intro.enabled}
        />
      </div>

      <div class="st-panel">
        <div class="st-tabs" role="tablist" aria-label={$t("studio.sections")}>
          {#each TABS as id, i (id)}
            <button
              type="button"
              role="tab"
              id="st-tab-{id}"
              class="st-tab"
              aria-selected={tab === id}
              aria-controls="st-pane-{id}"
              tabindex={tab === id ? 0 : -1}
              onclick={() => (tab = id)}
              onkeydown={(e) => onTabKey(e, i)}
            >
              {$t(`studio.tab_${id}`)}
              {#if id === "captions" && segments.length}<span class="st-badge">{segments.length}</span>{/if}
            </button>
          {/each}
        </div>
        <div class="st-pane" role="tabpanel" id="st-pane-captions" aria-labelledby="st-tab-captions" hidden={tab !== "captions"}>
          <TranscriptPanel
            bind:segments
            bind:burnCaptions
            bind:model
            bind:lang
            {asr}
            {asrState}
            {asrPercent}
            {asrError}
            hasAudio={probe.has_audio}
            {canBurn}
            durationMs={probe.duration_ms}
            {activeIndex}
            {locked}
            ondownload={downloadModel}
            ontranscribe={transcribe}
            onimport={importSubs}
            onexport={exportSubs}
            onseek={seek}
          />
        </div>
        <div class="st-pane" role="tabpanel" id="st-pane-style" aria-labelledby="st-tab-style" hidden={tab !== "style"}>
          <StylePanel bind:style {swatches} disabled={locked} />
        </div>
        <div class="st-pane" role="tabpanel" id="st-pane-frame" aria-labelledby="st-tab-frame" hidden={tab !== "frame"}>
          <FramePanel bind:reframe bind:intro bind:fadeOut bind:showIntro srcW={probe.width} srcH={probe.height} {canDrawText} {swatches} disabled={locked} />
        </div>
      </div>
    </div>

    {#if renderPhase === "done" && outcome}
      <div class="st-result" role="status">
        <StatusIcon name="done" size={20} />
        <div class="st-result-copy">
          <h2>{$t("studio.render_done")}</h2>
          <p>{$t("studio.render_done_body", { name: outcome.output.split(/[\\/]/).pop() ?? "", width: outcome.width, height: outcome.height, size: api.formatBytes(outcome.size_bytes) })}</p>
        </div>
        <div class="st-result-actions">
          <button type="button" class="btn btn-secondary" onclick={() => outcome && api.reveal(outcome.output).catch(() => {})}>{$t("studio.show_in_folder")}</button>
          <button type="button" class="btn btn-primary" onclick={() => outcome && api.openFile(outcome.output).catch(() => {})}>{$t("studio.play_video")}</button>
        </div>
      </div>
    {:else if renderPhase === "failed"}
      <div class="st-result failed" role="alert">
        <StatusIcon name="failed" size={20} />
        <div class="st-result-copy">
          <h2>{$t("studio.render_failed")}</h2>
          <p>{renderError}</p>
          <p class="st-hint">{$t("studio.render_failed_fix")}</p>
        </div>
      </div>
    {:else if renderPhase === "cancelled"}
      <div class="st-result" role="status">
        <StatusIcon name="stopped" size={20} />
        <div class="st-result-copy">
          <h2>{$t("studio.render_cancelled")}</h2>
          <p>{$t("studio.render_cancelled_body")}</p>
        </div>
      </div>
    {/if}

    <footer class="st-bar">
      <div class="st-bar-side">
        <label class="st-quality">
          <span class="st-bar-label">{$t("studio.quality")}</span>
          <select class="input" bind:value={quality} disabled={locked}>
            {#each QUALITIES as q (q)}
              <option value={q}>{$t(`studio.quality_${q}`)}</option>
            {/each}
          </select>
        </label>
        <button type="button" class="btn btn-ghost btn-sm st-dest" disabled={locked} onclick={chooseFolder} title={outDir ?? undefined}>
          {outDir ? $t("studio.save_to", { folder: outDir.split(/[\\/]/).filter(Boolean).pop() ?? outDir }) : $t("studio.save_next")}
        </button>
        {#if outDir}
          <button type="button" class="btn btn-ghost btn-sm" disabled={locked} onclick={() => (outDir = null)}>{$t("studio.save_next_reset")}</button>
        {/if}
      </div>

      <div class="st-bar-main">
        {#if rendering}
          <div class="st-progress" aria-hidden="true">
            <span>{$t(`studio.stage_${renderStage}`, { percent: renderPercent })}</span>
            <progress max="100" value={renderStage === "preparing" ? undefined : renderPercent}></progress>
          </div>
          <button type="button" class="btn btn-secondary" disabled={stopping} onclick={stop}>{stopping ? $t("studio.stopping") : $t("studio.stop")}</button>
        {:else}
          {#if renderBlock}
            <span class="st-why" id="st-why">{renderBlock}</span>
          {:else if summary.length}
            <span class="st-why">{summary.join(" · ")}</span>
          {/if}
          <button
            type="button"
            class={renderPhase === "done" ? "btn btn-secondary" : "btn btn-primary"}
            disabled={!!renderBlock}
            aria-describedby={renderBlock ? "st-why" : undefined}
            onclick={render}
          >
            {renderPhase === "done" || renderPhase === "failed" ? $t("studio.render_again") : $t("studio.render")}
          </button>
        {/if}
      </div>
    </footer>
  {/if}

  {#if dragging}
    <div class="st-dropping" aria-hidden="true">
      <p>{$t("studio.drop_here")}</p>
    </div>
  {/if}
</section>

<style>
  .st {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-6);
  }

  .st-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .st-head-copy {
    flex: 1;
    min-width: 0;
  }

  .st-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .st-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .st-live {
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

  .st-notice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .st-notice.ok :global(.cv-icon) {
    color: var(--success);
  }

  .st-notice-copy {
    flex: 1;
    min-width: 0;
  }

  .st-notice p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .st-drop {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    min-height: 340px;
    padding: var(--space-7) var(--space-5);
    border: 1.5px dashed var(--content-border);
    border-radius: var(--radius-xl);
    background: var(--surface);
    text-align: center;
    transition: border-color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }

  .st.dragging .st-drop {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .st-drop.err {
    border-style: solid;
  }

  .st-drop-busy {
    border-style: solid;
  }

  .st-drop-art {
    color: var(--text-dim);
  }

  .st-drop-title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .st-drop-body {
    margin: 0;
    max-width: 460px;
    font-size: var(--text-base);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }

  .st-drop-steps {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2) var(--space-5);
    margin: var(--space-3) 0 0;
    padding: 0;
    list-style: none;
    counter-reset: step;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .st-drop-steps li {
    counter-increment: step;
  }

  .st-drop-steps li::before {
    content: counter(step) ". ";
    color: var(--text-dim);
  }

  .st-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .st-meta-name {
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
    color: var(--text);
  }

  .st-meta-audio {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  .st-meta-audio.none {
    color: var(--text);
  }

  .st-meta-spin {
    width: 12px;
    height: 12px;
  }

  .st-work {
    display: grid;
    grid-template-columns: minmax(0, 400px) minmax(0, 1fr);
    gap: var(--space-5);
    align-items: start;
  }

  .st-preview {
    position: sticky;
    top: var(--space-3);
    min-width: 0;
  }

  .st-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .st-tabs {
    display: flex;
    gap: var(--space-1);
    padding: 3px;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    align-self: flex-start;
  }

  .st-tab {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    height: 26px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-muted);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
  }

  .st-tab[aria-selected="true"] {
    background: var(--control-bg);
    color: var(--text);
    box-shadow: 0 0 0 var(--hairline) var(--content-border);
  }

  .st-tab:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 1px;
  }

  .st-badge {
    min-width: 16px;
    padding: 0 4px;
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: var(--text-caption);
    line-height: 16px;
    text-align: center;
  }

  .st-pane {
    min-width: 0;
  }

  .st-pane[hidden] {
    display: none;
  }

  .st-result {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .st-result :global(.cv-icon-done) {
    color: var(--success);
  }

  .st-result.failed :global(.cv-icon) {
    color: var(--danger);
  }

  .st-result-copy {
    flex: 1;
    min-width: 220px;
  }

  .st-result-copy h2 {
    margin: 0 0 var(--space-1);
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .st-result-copy p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }

  .st-hint {
    margin-top: var(--space-1) !important;
  }

  .st-result-actions {
    display: flex;
    gap: var(--space-2);
    align-self: center;
  }

  .st-bar {
    position: sticky;
    bottom: 0;
    z-index: 1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) 0;
    border-top: var(--hairline) solid var(--content-border);
    background: var(--pane-bg);
  }

  .st-bar-side,
  .st-bar-main {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .st-quality {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .st-bar-label {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .st-dest {
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .st-why {
    max-width: 380px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .st-progress {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 220px;
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .st-progress progress {
    width: 100%;
    accent-color: var(--accent);
  }

  .st-dropping {
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

  .st-dropping p {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  @media (max-width: 860px) {
    .st-work {
      grid-template-columns: minmax(0, 1fr);
    }

    .st-preview {
      position: static;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .st-drop {
      transition: none;
    }
  }
</style>
