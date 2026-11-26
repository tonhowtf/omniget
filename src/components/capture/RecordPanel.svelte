<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "$lib/capture/api";
  import { formatBytes, formatElapsed, formatTimecode } from "$lib/capture/timecode";
  import { CONTAINERS, COUNTDOWN_SECONDS, FPS_CHOICES, QUALITIES, buildOptions, fileName, pickDefaultMic, type RegionDraft } from "$lib/capture/record";
  import type { Container, Platform, Quality, RecordOutcome, RecordingInfo, Sources } from "$lib/capture/types";
  import StatusIcon from "#components/converter/StatusIcon.svelte";
  import ErrorNotice from "./ErrorNotice.svelte";

  type Phase = "loading" | "ready" | "countdown" | "starting" | "recording" | "stopping" | "done";

  let {
    recording = $bindable(false),
    oncut,
    onhistory,
    onplatform,
  }: {
    recording?: boolean;
    oncut: (path: string) => void;
    onhistory: () => void;
    onplatform?: (platform: Platform) => void;
  } = $props();

  let phase = $state<Phase>("loading");
  let sources = $state<Sources | null>(null);
  let defaultDir = $state("");
  let error = $state<string | null>(null);
  let info = $state<RecordingInfo | null>(null);
  let outcome = $state<RecordOutcome | null>(null);
  let now = $state(Date.now());
  let count = $state(COUNTDOWN_SECONDS);
  let refreshing = $state(false);

  let screen = $state("");
  let useRegion = $state(false);
  let region = $state<RegionDraft>({ x: "0", y: "0", w: "1280", h: "720" });
  let fps = $state<number>(30);
  let micOn = $state(true);
  let mic = $state<string | null>(null);
  let cursor = $state(true);
  let container = $state<Container>("mp4");
  let quality = $state<Quality>("balanced");
  let countdown = $state(true);
  let outDir = $state<string | null>(null);

  let timer: ReturnType<typeof setInterval> | null = null;
  let countTimer: ReturnType<typeof setInterval> | null = null;

  let built = $derived(buildOptions({ screen, useRegion, region, fps, micOn, mic, cursor, container, quality, outDir }));
  let blocked = $derived.by(() => {
    if (!sources) return "capture.blocked_loading";
    if (!sources.ffmpeg) return "capture.blocked_ffmpeg";
    if (sources.screens.length === 0) return "capture.blocked_no_screen";
    if (built.problem) return `capture.problem_${built.problem}`;
    return null;
  });
  let elapsed = $derived(info ? now - info.started_at : 0);
  let platform = $derived(sources?.platform ?? "other");
  let locked = $derived(phase === "countdown" || phase === "starting" || phase === "recording" || phase === "stopping");

  $effect(() => {
    recording = phase === "recording" || phase === "stopping";
  });

  function applySources(next: Sources) {
    sources = next;
    onplatform?.(next.platform);
    if (!next.screens.some((s) => s.id === screen)) screen = next.screens[0]?.id ?? "";
    if (!next.audio.some((a) => a.id === mic)) mic = pickDefaultMic(next);
    if (next.audio.length === 0) micOn = false;
  }

  async function loadSources() {
    refreshing = true;
    try {
      applySources(await api.sources());
    } catch (e) {
      error = api.errorText(e);
    } finally {
      refreshing = false;
      if (phase === "loading") phase = "ready";
    }
  }

  function tick() {
    stopTimer();
    now = Date.now();
    timer = setInterval(() => (now = Date.now()), 250);
  }

  function stopTimer() {
    if (timer) clearInterval(timer);
    timer = null;
  }

  function beginCountdown() {
    if (blocked || locked) return;
    error = null;
    outcome = null;
    if (!countdown) {
      void begin();
      return;
    }
    count = COUNTDOWN_SECONDS;
    phase = "countdown";
    countTimer = setInterval(() => {
      count -= 1;
      if (count <= 0) {
        cancelCountdown(false);
        void begin();
      }
    }, 1000);
  }

  function cancelCountdown(back = true) {
    if (countTimer) clearInterval(countTimer);
    countTimer = null;
    if (back) phase = "ready";
  }

  async function begin() {
    const options = built.options;
    if (!options) {
      phase = "ready";
      return;
    }
    phase = "starting";
    try {
      info = await api.start(options);
      phase = "recording";
      tick();
    } catch (e) {
      error = api.errorText(e);
      phase = "ready";
    }
  }

  async function stop() {
    if (phase !== "recording") return;
    phase = "stopping";
    try {
      const result = await api.stop();
      if (phase === "stopping") finish(result);
    } catch (e) {
      if (phase !== "stopping") return;
      error = api.errorText(e);
      stopTimer();
      info = null;
      phase = "ready";
    }
  }

  function finish(result: RecordOutcome) {
    stopTimer();
    info = null;
    if (result.ok) {
      outcome = result;
      phase = "done";
    } else {
      error = result.error ?? "CAPTURE_RECORD_FAILED";
      phase = "ready";
    }
    onhistory();
  }

  async function chooseFolder() {
    const picked = await openDialog({ directory: true, multiple: false, defaultPath: outDir ?? defaultDir }).catch(() => null);
    if (typeof picked === "string") outDir = picked;
  }

  function again() {
    outcome = null;
    phase = "ready";
  }

  function onKey(event: KeyboardEvent) {
    if (phase === "countdown" && event.key === "Escape") {
      event.preventDefault();
      cancelCountdown();
    }
  }

  onMount(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];
    const keep = (fn: () => void) => (disposed ? fn() : cleanups.push(fn));
    void api.defaultDir().then((d) => (defaultDir = d)).catch(() => {});
    void api
      .status()
      .then((s) => {
        if (s && !disposed) {
          info = s;
          phase = "recording";
          tick();
        }
      })
      .catch(() => {})
      .finally(() => void loadSources());
    void api
      .onRecording((event) => {
        if (info && event.id === info.id && (phase === "recording" || phase === "stopping")) finish(event.outcome);
        else onhistory();
      })
      .then(keep)
      .catch(() => {});
    return () => {
      disposed = true;
      cleanups.forEach((fn) => fn());
      stopTimer();
      cancelCountdown(false);
    };
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="rec">
  {#if error}
    <ErrorNotice
      {error}
      {platform}
      retryLabel={$t("capture.check_again")}
      onretry={() => {
        error = null;
        void loadSources();
      }}
      ondismiss={() => (error = null)}
      onopenprivacy={(kind) => void api.openPrivacy(kind).catch(() => {})}
    />
  {/if}

  {#if phase === "loading"}
    <div class="rec-stage rec-stage-busy">
      <span class="spinner" aria-hidden="true"></span>
      <p>{$t("capture.loading_sources")}</p>
    </div>
  {:else if phase === "countdown"}
    <div class="rec-stage">
      <p class="rec-count" aria-live="assertive">{count}</p>
      <p class="rec-stage-hint">{$t("capture.countdown_hint")}</p>
      <button type="button" class="btn btn-secondary" onclick={() => cancelCountdown()}>{$t("capture.cancel_countdown")}</button>
    </div>
  {:else if phase === "starting" || phase === "recording" || phase === "stopping"}
    <div class="rec-stage rec-live" aria-live="polite">
      <div class="rec-indicator">
        {#if phase === "starting"}
          <span class="spinner" aria-hidden="true"></span>
          <span>{$t("capture.starting")}</span>
        {:else if phase === "stopping"}
          <span class="spinner" aria-hidden="true"></span>
          <span>{$t("capture.finalizing")}</span>
        {:else}
          <span class="rec-dot" aria-hidden="true"></span>
          <span>{$t("capture.recording_now")}</span>
        {/if}
      </div>
      <p class="rec-time" role="timer" aria-label={$t("capture.elapsed")}>{formatElapsed(elapsed)}</p>
      {#if info}
        <p class="rec-stage-hint">{$t("capture.saving_to", { name: fileName(info.path) })}</p>
      {/if}
      <button type="button" class="btn btn-primary btn-lg rec-stop" disabled={phase !== "recording"} onclick={stop}>
        <span class="rec-stop-square" aria-hidden="true"></span>
        {phase === "stopping" ? $t("capture.stopping") : $t("capture.stop")}
      </button>
    </div>
  {:else if phase === "done" && outcome}
    <div class="rec-done">
      <StatusIcon name={outcome.forced ? "warning" : "done"} size={20} />
      <div class="rec-done-copy">
        <h3>{$t("capture.saved_title")}</h3>
        <p class="rec-done-file">{fileName(outcome.path)}</p>
        <p class="rec-done-meta">{formatTimecode(outcome.duration, false)} · {formatBytes(outcome.size_bytes)}</p>
        {#if outcome.forced}
          <p class="rec-done-warn">{$t("capture.saved_forced")}</p>
        {/if}
        <div class="rec-done-actions">
          <button type="button" class="btn btn-primary" onclick={() => oncut(outcome!.path)}>{$t("capture.open_in_cutter")}</button>
          <button type="button" class="btn btn-secondary" onclick={() => void api.reveal(outcome!.path).catch(() => {})}>{$t("capture.show_in_folder")}</button>
          <button type="button" class="btn btn-ghost" onclick={again}>{$t("capture.record_again")}</button>
        </div>
      </div>
    </div>
  {/if}

  {#if phase === "ready" || phase === "done" || phase === "loading"}
    <fieldset class="rec-form" disabled={locked || phase === "loading"}>
      <div class="rec-row">
        <label class="rec-field rec-grow">
          <span class="field-label">{$t("capture.source")}</span>
          <select class="input" bind:value={screen} disabled={!sources?.screens.length}>
            {#if !sources?.screens.length}
              <option value="">{$t("capture.no_screens_option")}</option>
            {/if}
            {#each sources?.screens ?? [] as s (s.id)}
              <option value={s.id}>{s.kind === "window" ? $t("capture.window_prefix", { name: s.name }) : s.name}</option>
            {/each}
          </select>
        </label>
        <button type="button" class="btn btn-ghost btn-sm rec-refresh" disabled={refreshing} onclick={loadSources}>
          {refreshing ? $t("capture.refreshing") : $t("capture.refresh_sources")}
        </button>
      </div>

      <div class="rec-row">
        <div class="rec-field">
          <span class="field-label" id="rec-fps">{$t("capture.fps")}</span>
          <div class="mac-segmented" role="radiogroup" aria-labelledby="rec-fps">
            {#each FPS_CHOICES as f (f)}
              <button type="button" role="radio" aria-checked={fps === f} class="mac-segmented-btn" class:active={fps === f} onclick={() => (fps = f)}>{f}</button>
            {/each}
          </div>
        </div>
        <label class="rec-field">
          <span class="field-label">{$t("capture.container")}</span>
          <select class="input" bind:value={container}>
            {#each CONTAINERS as c (c)}
              <option value={c}>{$t(`capture.container_${c}`)}</option>
            {/each}
          </select>
        </label>
        <label class="rec-field">
          <span class="field-label">{$t("capture.quality")}</span>
          <select class="input" bind:value={quality}>
            {#each QUALITIES as q (q)}
              <option value={q}>{$t(`capture.quality_${q}`)}</option>
            {/each}
          </select>
        </label>
      </div>

      <div class="rec-checks">
        <label class="rec-check">
          <input type="checkbox" class="checkbox" bind:checked={micOn} disabled={!sources?.audio.length} />
          <span class="field-label">{$t("capture.mic")}</span>
        </label>
        {#if micOn}
          <select class="input rec-mic" bind:value={mic} aria-label={$t("capture.mic_device")}>
            {#each sources?.audio ?? [] as a (a.id)}
              <option value={a.id}>{a.name}</option>
            {/each}
          </select>
        {:else if !sources?.audio.length}
          <span class="field-hint">{$t("capture.no_mics")}</span>
        {/if}
      </div>

      <div class="rec-checks">
        <label class="rec-check">
          <input type="checkbox" class="checkbox" bind:checked={cursor} />
          <span class="field-label">{$t("capture.cursor")}</span>
        </label>
        <label class="rec-check">
          <input type="checkbox" class="checkbox" bind:checked={countdown} />
          <span class="field-label">{$t("capture.countdown")}</span>
        </label>
        <label class="rec-check">
          <input type="checkbox" class="checkbox" bind:checked={useRegion} />
          <span class="field-label">{$t("capture.region")}</span>
        </label>
      </div>

      {#if useRegion}
        <div class="rec-region">
          {#each ["x", "y", "w", "h"] as const as k (k)}
            <label class="rec-field">
              <span class="field-label">{$t(`capture.region_${k}`)}</span>
              <input class="input rec-num" type="text" inputmode="numeric" bind:value={region[k]} />
            </label>
          {/each}
          <p class="field-hint rec-region-hint">{$t("capture.region_hint")}</p>
        </div>
      {/if}

      <div class="rec-row rec-out">
        <div class="rec-field rec-grow">
          <span class="field-label">{$t("capture.save_to")}</span>
          <span class="rec-path" title={outDir ?? defaultDir}>{outDir ?? (defaultDir || $t("capture.default_folder"))}</span>
        </div>
        <button type="button" class="btn btn-secondary btn-sm" onclick={chooseFolder}>{$t("capture.choose_folder")}</button>
        {#if outDir}
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => (outDir = null)}>{$t("capture.use_default_folder")}</button>
        {/if}
      </div>
    </fieldset>

    <footer class="rec-bar">
      <p class="rec-bar-hint" aria-live="polite">{blocked ? $t(blocked) : $t("capture.ready_hint")}</p>
      <button type="button" class="btn btn-lg" class:btn-primary={phase !== "done" && !error} class:btn-secondary={phase === "done" || !!error} disabled={!!blocked || phase === "loading"} onclick={beginCountdown}>
        <span class="rec-dot rec-dot-btn" aria-hidden="true"></span>
        {$t("capture.start")}
      </button>
    </footer>
  {/if}
</div>

<style>
  .rec {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .rec-stage {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    min-height: 260px;
    padding: var(--space-6) var(--space-5);
    border-radius: var(--radius-xl);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    text-align: center;
  }

  .rec-stage-busy p,
  .rec-stage-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }

  .rec-count {
    margin: 0;
    font-family: var(--font-display);
    font-size: calc(var(--text-3xl) * 2);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }

  .rec-live {
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--error) 55%, var(--content-border));
  }

  .rec-indicator {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .rec-dot {
    width: 10px;
    height: 10px;
    border-radius: var(--radius-full);
    background: var(--error);
    animation: rec-pulse 1.2s var(--ease-in-out) infinite;
  }

  .rec-dot-btn {
    animation: none;
    background: currentColor;
  }

  @keyframes rec-pulse {
    50% {
      opacity: 0.35;
    }
  }

  .rec-time {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--text-3xl);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }

  .rec-stop {
    margin-top: var(--space-2);
  }

  .rec-stop-square {
    width: 10px;
    height: 10px;
    border-radius: 2px;
    background: currentColor;
  }

  .rec-done {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--success);
  }

  .rec-done-copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    color: var(--text);
  }

  .rec-done-copy h3 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
  }

  .rec-done-file {
    margin: 0;
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .rec-done-meta,
  .rec-done-warn {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .rec-done-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .rec-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    margin: 0;
    padding: var(--space-4);
    border: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .rec-row {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-3) var(--space-4);
  }

  .rec-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .rec-grow {
    flex: 1 1 260px;
  }

  .rec-grow select {
    width: 100%;
  }

  .rec-checks {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-5);
  }

  .rec-check {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    cursor: pointer;
  }

  .rec-mic {
    min-width: 220px;
  }

  .rec-region {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-3);
  }

  .rec-num {
    width: 84px;
    font-variant-numeric: tabular-nums;
  }

  .rec-region-hint {
    flex-basis: 100%;
    margin: 0;
  }

  .rec-path {
    font-size: var(--text-sm);
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .rec-bar {
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

  .rec-bar-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  @media (prefers-reduced-motion: reduce) {
    .rec-dot {
      animation: none;
    }
  }
</style>
