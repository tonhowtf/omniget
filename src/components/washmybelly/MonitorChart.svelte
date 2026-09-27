<script lang="ts">
  import { HISTORY, linePath, niceMax } from "$lib/washmybelly/monitor";

  // Live line chart with the area under it, drawn in SVG. Design after the
  // 21st.dev "Line Charts" cards (sean0205): title, big current value, a
  // soft gradient under the line; ported without Recharts.
  let {
    label,
    value,
    values,
    max = 0,
    floor = 1,
    color = "var(--accent)",
    second,
    secondColor = "var(--wmb-chart-2, #bf5af2)",
    sub = "",
    height = 56,
  }: {
    label: string;
    value: string;
    values: number[];
    /// Fixed ceiling (percentages: 100). 0 = follow the data.
    max?: number;
    floor?: number;
    color?: string;
    /// Optional second line (network send, disk write).
    second?: number[];
    secondColor?: string;
    sub?: string;
    height?: number;
  } = $props();

  let width = $state(240);
  const gid = `wmb-g-${Math.random().toString(36).slice(2, 8)}`;
  let ceiling = $derived(max > 0 ? max : niceMax([...values, ...(second ?? [])], floor));
  let a = $derived(linePath(values, ceiling, width, height, HISTORY));
  let b = $derived(second ? linePath(second, ceiling, width, height, HISTORY) : null);
</script>

<div class="chart">
  <div class="head">
    <span class="l">{label}</span>
    <span class="v">{value}</span>
  </div>
  <div class="plot" bind:clientWidth={width} style:height="{height}px">
    <svg {width} {height} viewBox="0 0 {width} {height}" role="img" aria-label={`${label}: ${value}`} preserveAspectRatio="none">
      <defs>
        <linearGradient id={gid} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stop-color={color} stop-opacity="0.28" />
          <stop offset="100%" stop-color={color} stop-opacity="0" />
        </linearGradient>
      </defs>
      <line x1="0" y1={height / 2} x2={width} y2={height / 2} class="grid" />
      {#if a.area}<path d={a.area} fill="url(#{gid})" />{/if}
      {#if a.line}<path d={a.line} fill="none" stroke={color} stroke-width="1.6" stroke-linejoin="round" vector-effect="non-scaling-stroke" />{/if}
      {#if b?.line}<path d={b.line} fill="none" stroke={secondColor} stroke-width="1.4" stroke-dasharray="3 2" vector-effect="non-scaling-stroke" />{/if}
    </svg>
  </div>
  {#if sub}<span class="s">{sub}</span>{/if}
</div>

<style>
  .chart { display: flex; flex-direction: column; gap: 6px; padding: 12px 14px; border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); min-width: 0; }
  .head { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; }
  .l { font-size: 12px; color: var(--text-dim); }
  .v { font-size: 17px; font-weight: 600; font-variant-numeric: tabular-nums; color: var(--text); }
  .plot { width: 100%; overflow: hidden; }
  svg { display: block; }
  .grid { stroke: var(--separator); stroke-width: 1; stroke-dasharray: 2 3; vector-effect: non-scaling-stroke; }
  .s { font-size: 11px; color: var(--text-dim); font-variant-numeric: tabular-nums; }
</style>
