<script lang="ts">
  import { formatSpeed, type SpeedPoint } from "$lib/stores/download-store.svelte";

  interface Props {
    points: SpeedPoint[];
    windowMs?: number;
    width?: number;
    height?: number;
    showTooltip?: boolean;
    status?: boolean;
  }

  let {
    points,
    windowMs = 60_000,
    width = 64,
    height = 20,
    showTooltip = true,
    status = false,
  }: Props = $props();

  let endT = $derived(points.length ? points[points.length - 1].t : Date.now());
  let startT = $derived(endT - windowMs);

  let rawWindowPoints = $derived(points.filter((p) => p.t >= startT));
  let plotStartT = $derived(rawWindowPoints.length ? rawWindowPoints[0].t : endT);
  let plotDuration = $derived(Math.max(
    8_000,
    Math.min(windowMs, endT - plotStartT),
  ));

  let windowPoints = $derived.by(() => {
    if (rawWindowPoints.length <= 2) return rawWindowPoints;
    const alpha = 0.35;
    const lastIndex = rawWindowPoints.length - 1;
    let ema = rawWindowPoints[0].bps;
    return rawWindowPoints.map((p, idx) => {
      if (idx === 0) return p;
      ema = alpha * p.bps + (1 - alpha) * ema;
      if (idx === lastIndex) return p;
      return { ...p, bps: ema };
    });
  });

  let maxBps = $derived.by(() => {
    let max = 0;
    for (const p of windowPoints) if (p.bps > max) max = p.bps;
    return Math.max(1, Math.ceil(max * 1.15));
  });

  let peakBps = $derived.by(() => {
    let max = 0;
    for (const p of windowPoints) if (p.bps > max) max = p.bps;
    return max;
  });

  let dLine = $derived.by(() => {
    if (windowPoints.length === 0) return "";
    const toX = (t: number) => Math.min(100, ((t - plotStartT) / plotDuration) * 100);
    const toY = (bps: number) => 100 - (Math.max(0, bps) / maxBps) * 100;
    if (windowPoints.length === 1) {
      const y = toY(windowPoints[0].bps);
      return `M 0 ${y.toFixed(3)} L 1 ${y.toFixed(3)}`;
    }
    let d = "";
    for (let i = 0; i < windowPoints.length; i++) {
      const p = windowPoints[i];
      const x = toX(p.t);
      const y = toY(p.bps);
      d += i === 0 ? `M ${x.toFixed(3)} ${y.toFixed(3)}` : ` L ${x.toFixed(3)} ${y.toFixed(3)}`;
    }
    return d;
  });

  let dArea = $derived.by(() => {
    if (!dLine) return "";
    const first = windowPoints[0];
    const last = windowPoints[windowPoints.length - 1];
    const x0 = Math.min(100, ((first.t - plotStartT) / plotDuration) * 100);
    const x1 = Math.min(100, ((last.t - plotStartT) / plotDuration) * 100);
    return `M ${x0.toFixed(3)} 100 ${dLine.replace(/^M/, "L")} L ${x1.toFixed(3)} 100 Z`;
  });

  let currentBps = $derived(windowPoints.length ? windowPoints[windowPoints.length - 1].bps : 0);
  let lastPoint = $derived(windowPoints.length ? windowPoints[windowPoints.length - 1] : undefined);
  let lastX = $derived(lastPoint ? Math.min(100, ((lastPoint.t - plotStartT) / plotDuration) * 100) : 0);
  let lastY = $derived(lastPoint ? 100 - (Math.max(0, lastPoint.bps) / maxBps) * 100 : 100);
  let isIdle = $derived(windowPoints.length > 2 && currentBps < 1024);
</script>

<div
  class="speed-graph"
  class:status
  style:width="{width}px" style:height="{height}px"
  title={showTooltip ? `Now: ${formatSpeed(currentBps)} • Peak: ${formatSpeed(peakBps)}` : undefined}
  data-point-count={windowPoints.length}
  data-current-speed={Math.round(currentBps)}
  aria-hidden="true"
>
  <svg viewBox="0 0 100 100" preserveAspectRatio="none">
    <defs>
      <linearGradient id="speedGraphFill" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" stop-color="var(--accent)" stop-opacity="0.32" />
        <stop offset="100%" stop-color="var(--accent)" stop-opacity="0" />
      </linearGradient>
    </defs>
    <path class="guide" d="M 0 50 H 100" vector-effect="non-scaling-stroke" />
    <path class="baseline" d="M 0 99 H 100" vector-effect="non-scaling-stroke" />
    {#if dArea}
      <path class="area" d={dArea} fill="url(#speedGraphFill)" />
    {/if}
    {#if dLine}
      <path class="line" d={dLine} vector-effect="non-scaling-stroke" />
    {:else}
      <path class="line" d="M 0 100 L 100 100" vector-effect="non-scaling-stroke" />
    {/if}
    {#if windowPoints.length}
      <circle class="now-dot" class:idle={isIdle} cx={lastX} cy={lastY} r="1.8" />
    {/if}
  </svg>
</div>

<style>
  .speed-graph {
    display: inline-block;
    vertical-align: middle;
  }

  .speed-graph.status {
    padding: 3px 4px;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--fill-2) 72%, transparent);
  }

  svg {
    width: 100%;
    height: 100%;
    display: block;
    overflow: visible;
  }

  .line {
    fill: none;
    stroke: var(--accent-text);
    stroke-width: 1px;
    stroke-linejoin: round;
    stroke-linecap: round;
    opacity: 0.78;
    filter: drop-shadow(0 1px 2px color-mix(in srgb, var(--accent) 24%, transparent));
    shape-rendering: geometricPrecision;
  }

  .area {
    opacity: 0.72;
  }

  .guide,
  .baseline {
    fill: none;
    stroke: var(--border);
    stroke-width: 0.75px;
    opacity: 0;
  }

  .status .guide,
  .status .baseline {
    opacity: 0.58;
  }

  .now-dot {
    fill: var(--accent-text);
    opacity: 0.9;
    transition:
      cx var(--duration-base) var(--ease-out),
      cy var(--duration-base) var(--ease-out);
  }

  .now-dot.idle {
    fill: var(--warning);
    opacity: 0.95;
  }

  .status .now-dot:not(.idle) {
    animation: speed-current-pulse 1.8s var(--ease-out) infinite;
  }

  @keyframes speed-current-pulse {
    0%, 100% { opacity: 0.95; }
    50% { opacity: 0.48; }
  }

  @media (prefers-reduced-motion: reduce) {
    .speed-graph.status .now-dot {
      transition: none;
      animation: none;
    }
  }

  :global([data-reduce-motion="true"]) .speed-graph.status .now-dot {
    transition: none;
    animation: none;
  }
</style>
