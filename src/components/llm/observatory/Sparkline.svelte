<script lang="ts">
  import { MAX_POINTS } from "$lib/stores/llm-telemetry-store.svelte";

  interface Props {
    values: number[];
    times?: number[];
    format?: (v: number) => string;
    label: string;
    height?: number;
    tone?: "accent" | "warn" | "error";
  }

  let { values, times = [], format = (v) => String(Math.round(v)), label, height = 44, tone = "accent" }: Props = $props();

  const W = 300;

  let data = $derived(values.length > MAX_POINTS ? values.slice(values.length - MAX_POINTS) : values);
  let stamps = $derived(times.length === values.length ? times.slice(times.length - data.length) : []);
  let max = $derived(Math.max(1, ...data));
  let step = $derived(data.length > 1 ? W / (data.length - 1) : W);

  function y(v: number): number {
    return height - 3 - (Math.max(0, v) / max) * (height - 6);
  }

  let line = $derived(data.map((v, i) => `${i === 0 ? "M" : "L"}${(i * step).toFixed(2)} ${y(v).toFixed(2)}`).join(" "));
  let area = $derived(data.length ? `${line} L${((data.length - 1) * step).toFixed(2)} ${height} L0 ${height} Z` : "");
  let lastIdx = $derived(data.length - 1);

  let hover = $state(-1);

  function onMove(e: PointerEvent) {
    const rect = (e.currentTarget as SVGSVGElement).getBoundingClientRect();
    if (rect.width === 0 || data.length === 0) return;
    const x = ((e.clientX - rect.left) / rect.width) * W;
    hover = Math.max(0, Math.min(lastIdx, Math.round(x / step)));
  }

  function timeLabel(i: number): string {
    const t = stamps[i];
    if (!t) return "";
    return new Date(t).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  }
</script>

<div class="spark" style:--spark-h="{height}px">
  <svg
    viewBox="0 0 {W} {height}"
    preserveAspectRatio="none"
    role="img"
    aria-label={label}
    class="tone-{tone}"
    onpointermove={onMove}
    onpointerleave={() => (hover = -1)}
  >
    {#if data.length > 1}
      <path class="area" d={area} />
      <path class="line" d={line} vector-effect="non-scaling-stroke" />
      {#if hover >= 0}
        <line class="crosshair" x1={hover * step} x2={hover * step} y1="0" y2={height} vector-effect="non-scaling-stroke" />
      {/if}
      <circle class="dot-ring" cx={lastIdx * step} cy={y(data[lastIdx])} r="6" />
      <circle class="dot" cx={lastIdx * step} cy={y(data[lastIdx])} r="4" />
    {/if}
  </svg>
  {#if hover >= 0 && data.length > 1}
    <span class="readout" style:left="{(hover / lastIdx) * 100}%">
      <strong>{format(data[hover])}</strong>{#if stamps.length}<span class="at">{timeLabel(hover)}</span>{/if}
    </span>
  {/if}
</div>

<style>
  .spark {
    position: relative;
    width: 100%;
    height: var(--spark-h);
  }
  svg {
    width: 100%;
    height: 100%;
    display: block;
    overflow: visible;
    --series: var(--accent);
  }
  svg.tone-warn { --series: var(--warning); }
  svg.tone-error { --series: var(--error); }
  .area {
    fill: var(--series);
    opacity: 0.1;
  }
  .line {
    fill: none;
    stroke: var(--series);
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .crosshair {
    stroke: var(--border);
    stroke-width: 1;
  }
  .dot {
    fill: var(--series);
  }
  .dot-ring {
    fill: var(--surface);
  }
  .readout {
    position: absolute;
    top: -2px;
    transform: translateX(-50%);
    display: flex;
    gap: var(--space-1);
    align-items: baseline;
    padding: 2px 6px;
    border-radius: var(--radius-xs);
    background: var(--surface-hi);
    border: 1px solid var(--border);
    font-size: var(--text-caption);
    color: var(--text);
    white-space: nowrap;
    pointer-events: none;
  }
  .readout .at {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
</style>
