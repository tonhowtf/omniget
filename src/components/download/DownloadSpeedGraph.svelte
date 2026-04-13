<script lang="ts">
  import { formatSpeed, type SpeedPoint } from "$lib/stores/download-store.svelte";

  interface Props {
    points: SpeedPoint[];
    windowMs?: number;
    width?: number;
    height?: number;
  }

  let {
    points,
    windowMs = 60_000,
    width = 64,
    height = 20,
  }: Props = $props();

  let endT = $derived(points.length ? points[points.length - 1].t : Date.now());
  let startT = $derived(endT - windowMs);

  let rawWindowPoints = $derived(points.filter((p) => p.t >= startT));

  let windowPoints = $derived.by(() => {
    if (rawWindowPoints.length <= 2) return rawWindowPoints;
    const alpha = 0.35;
    let ema = rawWindowPoints[0].bps;
    return rawWindowPoints.map((p, idx) => {
      if (idx === 0) return p;
      ema = alpha * p.bps + (1 - alpha) * ema;
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
    const toX = (t: number) => ((t - startT) / windowMs) * 100;
    const toY = (bps: number) => 100 - (Math.max(0, bps) / maxBps) * 100;
    let d = "";
    for (let i = 0; i < windowPoints.length; i++) {
      const p = windowPoints[i];
      const x = toX(p.t);
      const y = toY(p.bps);
      d += i === 0 ? `M ${x.toFixed(3)} ${y.toFixed(3)}` : ` L ${x.toFixed(3)} ${y.toFixed(3)}`;
    }
    return d;
  });

  let currentBps = $derived(windowPoints.length ? windowPoints[windowPoints.length - 1].bps : 0);
  let lastPoint = $derived(windowPoints.length ? windowPoints[windowPoints.length - 1] : undefined);
  let lastX = $derived(lastPoint ? ((lastPoint.t - startT) / windowMs) * 100 : 100);
  let lastY = $derived(lastPoint ? 100 - (Math.max(0, lastPoint.bps) / maxBps) * 100 : 100);
  let isIdle = $derived(windowPoints.length > 2 && currentBps < 1024);
</script>

<div
  class="speed-graph"
  style="width: {width}px; height: {height}px"
  title="Now: {formatSpeed(currentBps)} • Peak: {formatSpeed(peakBps)}"
  aria-hidden="true"
>
  <svg viewBox="0 0 100 100" preserveAspectRatio="none">
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

  svg {
    width: 100%;
    height: 100%;
    display: block;
    overflow: visible;
  }

  .line {
    fill: none;
    stroke: var(--accent, rgba(80, 160, 255, 1));
    stroke-width: 1px;
    stroke-linejoin: round;
    stroke-linecap: round;
    opacity: 0.78;
    filter: drop-shadow(0 2px 6px rgba(0, 0, 0, 0.25));
    shape-rendering: geometricPrecision;
  }

  .now-dot {
    fill: var(--accent, rgba(80, 160, 255, 1));
    opacity: 0.9;
  }

  .now-dot.idle {
    fill: rgba(255, 195, 90, 0.95);
    opacity: 0.95;
  }
</style>
