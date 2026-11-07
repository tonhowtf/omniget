<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { activeCue, type Cue } from "$lib/studio/layout";
  import { blurRadius } from "$lib/studio/frame";
  import { formatClock } from "$lib/studio/subtitles";
  import type { CaptionStyle, Reframe } from "$lib/studio/types";
  import CaptionOverlay from "./CaptionOverlay.svelte";
  import IntroCard from "./IntroCard.svelte";

  type IntroView = { title: string[]; subtitle: string[]; background: string; color: string };

  let {
    src,
    name,
    durationMs,
    outW,
    outH,
    reframe,
    cues,
    style,
    showCaptions,
    intro,
    showIntro = false,
    time = $bindable(0),
  }: {
    src: string | null;
    name: string;
    durationMs: number;
    outW: number;
    outH: number;
    reframe: Reframe;
    cues: Cue[];
    style: CaptionStyle;
    showCaptions: boolean;
    intro: IntroView | null;
    showIntro?: boolean;
    time?: number;
  } = $props();

  const MAX_STAGE_H = 520;

  let video = $state<HTMLVideoElement | null>(null);
  let canvas = $state<HTMLCanvasElement | null>(null);
  let boxW = $state(0);
  let playing = $state(false);
  let playable = $state(true);
  let loaded = $state(false);
  let frame = 0;
  let virtualStart = 0;
  let virtualFrom = 0;

  let aspect = $derived(outW / Math.max(1, outH));
  let stageW = $derived(Math.max(1, Math.min(boxW, MAX_STAGE_H * aspect)));
  let stageH = $derived(stageW / aspect);
  let scale = $derived(stageW / Math.max(1, outW));
  let cue = $derived(showCaptions ? activeCue(cues, time) : null);
  let fit = $derived(reframe.aspect !== "keep" && reframe.mode === "fit");
  let fitStyle = $derived(
    reframe.aspect === "keep" ? "fill" : reframe.mode === "crop" ? "cover" : "contain",
  );
  let blurPx = $derived(blurRadius(outW, outH) * scale * 0.9);
  let total = $derived(Math.max(1, durationMs));

  $effect(() => {
    void src;
    playable = !!src;
    loaded = false;
    playing = false;
  });

  function paintBlur() {
    if (!fit || !canvas || !video || !playable || video.readyState < 2) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const cw = canvas.width;
    const ch = canvas.height;
    const vw = video.videoWidth || 1;
    const vh = video.videoHeight || 1;
    const s = Math.max(cw / vw, ch / vh);
    const dw = vw * s;
    const dh = vh * s;
    try {
      ctx.drawImage(video, (cw - dw) / 2, (ch - dh) / 2, dw, dh);
    } catch {
      return;
    }
  }

  function loop() {
    frame = 0;
    if (playable && video) {
      time = video.currentTime * 1000;
      paintBlur();
    } else {
      const next = virtualFrom + (performance.now() - virtualStart);
      time = Math.min(total, next);
      if (next >= total) {
        playing = false;
        return;
      }
    }
    if (playing) frame = requestAnimationFrame(loop);
  }

  function start() {
    if (!frame) frame = requestAnimationFrame(loop);
  }

  export function seek(ms: number) {
    const target = Math.max(0, Math.min(total, ms));
    time = target;
    if (playable && video) {
      video.currentTime = target / 1000;
    } else {
      virtualFrom = target;
      virtualStart = performance.now();
    }
  }

  export function toggle() {
    if (playing) {
      playing = false;
      video?.pause();
      return;
    }
    if (playable && video) {
      if (video.ended) video.currentTime = 0;
      void video.play().catch(() => (playing = false));
    } else {
      if (time >= total) time = 0;
      virtualFrom = time;
      virtualStart = performance.now();
    }
    playing = true;
    start();
  }

  function onError() {
    playable = false;
    playing = false;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === " " || e.key === "Enter" || e.key === "k") {
      e.preventDefault();
      toggle();
    } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      seek(time + (e.key === "ArrowLeft" ? -1000 : 1000) * (e.shiftKey ? 5 : 1));
    }
  }

  $effect(() => {
    void reframe.mode;
    void reframe.aspect;
    void stageW;
    queueMicrotask(paintBlur);
  });

  onDestroy(() => {
    if (frame) cancelAnimationFrame(frame);
  });
</script>

<div class="pv" bind:clientWidth={boxW}>
  <div
    class="pv-stage"
    style:width="{stageW}px"
    style:height="{stageH}px"
    tabindex="0"
    role="button"
    aria-label={$t("studio.preview_label")}
    onclick={toggle}
    onkeydown={onKey}
  >
    {#if playable && src}
      {#if fit}
        <canvas bind:this={canvas} class="pv-blur" width="64" height={Math.max(2, Math.round(64 / aspect))} style:filter="blur({blurPx}px)" aria-hidden="true"></canvas>
      {/if}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        bind:this={video}
        class="pv-video"
        {src}
        preload="auto"
        playsinline
        style:object-fit={fitStyle}
        style:object-position="{reframe.position * 100}% 50%"
        onerror={onError}
        onloadeddata={() => {
          loaded = true;
          paintBlur();
        }}
        onseeked={paintBlur}
        onpause={() => (playing = false)}
        onended={() => (playing = false)}
        ontimeupdate={() => {
          if (!playing && video) time = video.currentTime * 1000;
        }}
      ></video>
      {#if !loaded}
        <span class="spinner pv-spin" aria-hidden="true"></span>
      {/if}
    {:else}
      <div class="pv-stand">
        <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="3" y="5" width="18" height="14" rx="2" />
          <path d="m10 9 5 3-5 3z" />
        </svg>
        <p class="pv-stand-name">{name}</p>
        <p class="pv-stand-note">{$t("studio.preview_unplayable")}</p>
      </div>
    {/if}

    <CaptionOverlay {cue} {style} {time} {outW} {outH} {scale} />

    {#if showIntro && intro}
      <IntroCard title={intro.title} subtitle={intro.subtitle} background={intro.background} color={intro.color} {outW} {outH} {scale} />
    {/if}
  </div>

  <div class="pv-transport">
    <button type="button" class="btn btn-icon btn-sm" onclick={toggle} aria-label={playing ? $t("studio.pause") : $t("studio.play")}>
      {#if playing}
        <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true"><rect x="4" y="3" width="3" height="10" rx="1" /><rect x="9" y="3" width="3" height="10" rx="1" /></svg>
      {:else}
        <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true"><path d="M5 3.2v9.6a.6.6 0 0 0 .9.5l7.6-4.8a.6.6 0 0 0 0-1L5.9 2.7a.6.6 0 0 0-.9.5z" /></svg>
      {/if}
    </button>
    <input
      class="pv-scrub"
      type="range"
      min="0"
      max={total}
      step="10"
      value={time}
      aria-label={$t("studio.scrub")}
      aria-valuetext={`${formatClock(time)} / ${formatClock(total)}`}
      oninput={(e) => seek(Number((e.currentTarget as HTMLInputElement).value))}
    />
    <span class="pv-time">{formatClock(time)} / {formatClock(total)}</span>
  </div>
  <p class="pv-dims">{$t("studio.output_size", { width: outW, height: outH })}</p>
</div>

<style>
  .pv {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-width: 0;
  }

  .pv-stage {
    position: relative;
    overflow: hidden;
    border-radius: var(--radius-md);
    background: var(--primary);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    container-type: size;
  }

  .pv-stage:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 2px;
  }

  .pv-video,
  .pv-blur {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .pv-blur {
    transform: scale(1.15);
  }

  .pv-spin {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 18px;
    height: 18px;
    margin: -9px 0 0 -9px;
  }

  .pv-stand {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-4);
    text-align: center;
    color: var(--text-dim);
    background:
      repeating-linear-gradient(45deg, transparent 0 10px, color-mix(in srgb, var(--content-border) 40%, transparent) 10px 11px),
      var(--surface);
  }

  .pv-stand-name {
    margin: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .pv-stand-note {
    margin: 0;
    max-width: 260px;
    font-size: var(--text-xs);
  }

  .pv-transport {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
  }

  .pv-scrub {
    flex: 1;
    min-width: 0;
    accent-color: var(--accent);
  }

  .pv-time {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    white-space: nowrap;
  }

  .pv-dims {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
