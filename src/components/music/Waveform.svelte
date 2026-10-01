<script lang="ts">
  import { onMount } from "svelte";
  import { fmtTime } from "$lib/music/library";
  import { registerSemantic } from "$lib/native-agent/semantic";

  let {
    peaks = [],
    position = 0,
    duration = 0,
    onseek,
    height = 44,
    barWidth = 2,
    barGap = 1,
    normalize = true,
    label = "Seek",
    markers = [],
  }: {
    peaks?: ArrayLike<number>;
    position?: number;
    duration?: number;
    onseek?: (s: number) => void;
    height?: number;
    barWidth?: number;
    barGap?: number;
    normalize?: boolean;
    label?: string;
    markers?: { at: number; kind: string }[];
  } = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  let wrap: HTMLDivElement | undefined = $state();
  let width = $state(0);
  let hoverX = $state<number | null>(null);
  let dragging = $state(false);
  let dragPos = $state<number | null>(null);

  const rid = `music-waveform-${Math.random().toString(36).slice(2, 8)}`;
  onMount(() =>
    registerSemantic(rid, () => ({ ready: true, kind: "seek bar", position, duration, peaks: peaks.length })),
  );

  let progress = $derived(duration > 0 ? Math.min(1, (dragPos ?? position) / duration) : 0);

  $effect(() => {
    if (!wrap) return;
    const ro = new ResizeObserver((e) => (width = Math.floor(e[0].contentRect.width)));
    ro.observe(wrap);
    return () => ro.disconnect();
  });

  function color(name: string, fallback: string) {
    if (!wrap) return fallback;
    const v = getComputedStyle(wrap).getPropertyValue(name).trim();
    return v || fallback;
  }

  $effect(() => {
    const c = canvas;
    if (!c || width <= 0) return;
    void progress;
    void peaks;
    const dpr = window.devicePixelRatio || 1;
    c.width = Math.round(width * dpr);
    c.height = Math.round(height * dpr);
    const g = c.getContext("2d");
    if (!g) return;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, width, height);
    const played = color("--wf-played", "#ff9f0a");
    const rest = color("--wf-rest", "rgba(255,255,255,.22)");
    const n = peaks.length;
    const step = barWidth + barGap;
    const bars = Math.max(1, Math.floor(width / step));
    if (!n) {
      const y = height / 2 - 2;
      g.fillStyle = rest;
      g.beginPath();
      g.roundRect(0, y, width, 4, 2);
      g.fill();
      g.fillStyle = played;
      g.beginPath();
      g.roundRect(0, y, width * progress, 4, 2);
      g.fill();
      return;
    }
    let max = 1;
    if (normalize) {
      max = 0;
      for (let i = 0; i < n; i++) max = Math.max(max, peaks[i]);
      max = max / 255 || 1;
    }
    const cut = progress * width;
    for (let b = 0; b < bars; b++) {
      const s = Math.floor((b / bars) * n);
      const e = Math.max(s + 1, Math.floor(((b + 1) / bars) * n));
      let v = 0;
      for (let i = s; i < e && i < n; i++) v = Math.max(v, peaks[i]);
      const amp = Math.min(1, v / 255 / max);
      const h = Math.max(2, amp * (height - 2));
      const x = b * step;
      g.fillStyle = x + barWidth <= cut ? played : rest;
      g.beginPath();
      g.roundRect(x, (height - h) / 2, barWidth, h, Math.min(1, barWidth / 2));
      g.fill();
    }
  });

  function timeAt(clientX: number): number {
    if (!wrap || duration <= 0) return 0;
    const r = wrap.getBoundingClientRect();
    return Math.max(0, Math.min(1, (clientX - r.left) / r.width)) * duration;
  }

  function down(e: PointerEvent) {
    if (duration <= 0) return;
    dragging = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    dragPos = timeAt(e.clientX);
  }
  function move(e: PointerEvent) {
    if (!wrap) return;
    const r = wrap.getBoundingClientRect();
    hoverX = e.clientX - r.left;
    if (dragging) dragPos = timeAt(e.clientX);
  }
  function up(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    const t = timeAt(e.clientX);
    dragPos = null;
    onseek?.(t);
  }
  function key(e: KeyboardEvent) {
    if (duration <= 0) return;
    const step = e.shiftKey ? 30 : 5;
    if (e.key === "ArrowRight") onseek?.(Math.min(duration, position + step));
    else if (e.key === "ArrowLeft") onseek?.(Math.max(0, position - step));
    else if (e.key === "Home") onseek?.(0);
    else return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<div
  class="wf"
  bind:this={wrap}
  style:height="{height}px"
  role="slider"
  tabindex="0"
  aria-label={label}
  aria-valuemin={0}
  aria-valuemax={Math.round(duration)}
  aria-valuenow={Math.round(position)}
  aria-valuetext={`${fmtTime(position * 1000)} / ${fmtTime(duration * 1000)}`}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointerleave={() => (hoverX = null)}
  onkeydown={key}
>
  <canvas bind:this={canvas} data-agent-renderer={rid} style:width="{width}px" style:height="{height}px"></canvas>
  {#each markers as m (m.kind + m.at)}
    {#if duration > 0}
      <span class="mark {m.kind}" style:left="{(m.at / duration) * 100}%"></span>
    {/if}
  {/each}
  {#if hoverX != null && duration > 0}
    <span class="hover" style:left="{hoverX}px"></span>
    <span class="tip" style:left="{Math.min(Math.max(hoverX, 22), width - 22)}px">{fmtTime((hoverX / Math.max(1, width)) * duration * 1000)}</span>
  {/if}
</div>

<style>
  .wf {
    --wf-played: var(--accent);
    --wf-rest: color-mix(in srgb, var(--secondary) 24%, transparent);
    position: relative;
    width: 100%;
    cursor: pointer;
    touch-action: none;
    border-radius: 4px;
  }
  .wf:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }
  canvas {
    display: block;
  }
  .hover {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: color-mix(in srgb, var(--secondary) 70%, transparent);
    pointer-events: none;
  }
  .tip {
    position: absolute;
    top: -20px;
    transform: translateX(-50%);
    padding: 1px 5px;
    border-radius: 4px;
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
    color: var(--text);
    background: var(--surface-hi);
    box-shadow: 0 0 0 var(--hairline) var(--content-border);
    pointer-events: none;
    white-space: nowrap;
  }
  .mark {
    position: absolute;
    bottom: -3px;
    width: 5px;
    height: 5px;
    margin-left: -2.5px;
    border-radius: 50%;
    background: var(--text-muted);
    pointer-events: none;
  }
  .mark.outro {
    background: var(--accent);
  }
</style>
