<script lang="ts" module>
  export type ChartSeries = { id: string; label: string; values: number[]; color?: string };
</script>

<script lang="ts">
  import { areaPath, linePath, niceMax } from "$lib/washmybelly/ui";

  /// Live multi-series chart for the monitor: lines with a soft area, a quiet
  /// axis on the right, a legend with the latest value, and a crosshair on
  /// hover. Own SVG, no chart library. New points enter from the right.
  let {
    series,
    max,
    capacity = 60,
    height = 140,
    format = (v: number) => `${Math.round(v)}`,
    showLegend = true,
    label = "",
  }: {
    series: ChartSeries[];
    /// Fixed top of the axis (100 for percent). Omitted: rounded from data.
    max?: number;
    capacity?: number;
    height?: number;
    format?: (v: number) => string;
    showLegend?: boolean;
    label?: string;
  } = $props();

  const PALETTE = ["var(--blue)", "var(--purple)", "var(--success)", "var(--accent)", "var(--teal)", "var(--danger)"];
  const uid = `wmb-lc-${Math.random().toString(36).slice(2, 9)}`;

  let width = $state(320);
  let hoverIdx = $state<number | null>(null);

  let top = $derived.by(() => {
    if (max !== undefined) return max;
    let m = 0;
    for (const s of series) for (const v of s.values.slice(-capacity)) if (v > m) m = v;
    return niceMax(m);
  });
  let plotW = $derived(Math.max(40, width - 44));
  let plotH = $derived(Math.max(20, height - 8));
  let longest = $derived(Math.min(capacity, Math.max(0, ...series.map((s) => s.values.length))));
  let drawn = $derived(
    series.map((s, i) => {
      const values = s.values.slice(-capacity);
      const color = s.color ?? PALETTE[i % PALETTE.length];
      return {
        ...s,
        color,
        values,
        line: linePath(values, plotW, plotH, top, capacity),
        area: areaPath(values, plotW, plotH, top, capacity),
      };
    }),
  );
  let step = $derived(plotW / Math.max(1, capacity - 1));

  function onMove(e: PointerEvent) {
    const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
    const x = e.clientX - rect.left;
    // Index counted from the right edge (the newest point).
    const fromRight = Math.round((plotW - x) / step);
    const idx = longest - 1 - fromRight;
    hoverIdx = idx >= 0 && idx < longest ? idx : null;
  }

  function valueAt(values: number[], idx: number | null): number | undefined {
    if (idx === null) return values[values.length - 1];
    const off = longest - values.length;
    return values[idx - off];
  }

  let hoverX = $derived(hoverIdx === null ? null : plotW - (longest - 1 - hoverIdx) * step);
</script>

<figure class="wmb-lc" aria-label={label || undefined}>
  {#if showLegend}
    <figcaption class="legend">
      {#each drawn as s (s.id)}
        {@const v = valueAt(s.values, hoverIdx)}
        <span class="key">
          <span class="sw" style:background={s.color}></span>
          <span class="name">{s.label}</span>
          <span class="val">{v === undefined ? "–" : format(v)}</span>
        </span>
      {/each}
    </figcaption>
  {/if}
  <div class="plot" bind:clientWidth={width}>
    <svg
      width={width}
      {height}
      viewBox="0 0 {width} {height}"
      role="img"
      aria-hidden={label ? undefined : "true"}
      onpointermove={onMove}
      onpointerleave={() => (hoverIdx = null)}
    >
      <defs>
        {#each drawn as s (s.id)}
          <linearGradient id="{uid}-{s.id}" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" style:stop-color={s.color} stop-opacity="0.22" />
            <stop offset="100%" style:stop-color={s.color} stop-opacity="0" />
          </linearGradient>
        {/each}
      </defs>
      <g transform="translate(0 4)">
        {#each [0, 0.5, 1] as f (f)}
          <line class="grid" x1="0" x2={plotW} y1={plotH * (1 - f)} y2={plotH * (1 - f)} />
          <text class="tick" x={plotW + 6} y={plotH * (1 - f)} dy="0.32em">{format(top * f)}</text>
        {/each}
        {#each drawn as s (s.id)}
          {#if s.area}<path d={s.area} fill="url(#{uid}-{s.id})" />{/if}
          {#if s.line}<path d={s.line} fill="none" style:stroke={s.color} stroke-width="1.7" stroke-linejoin="round" stroke-linecap="round" />{/if}
        {/each}
        {#if hoverX !== null}
          <line class="cross" x1={hoverX} x2={hoverX} y1="0" y2={plotH} />
          {#each drawn as s (s.id)}
            {@const v = valueAt(s.values, hoverIdx)}
            {#if v !== undefined}
              <circle cx={hoverX} cy={plotH - (Math.min(v, top) / (top || 1)) * plotH} r="3" style:fill={s.color} class="dot" />
            {/if}
          {/each}
        {/if}
      </g>
    </svg>
  </div>
</figure>

<style>
  .wmb-lc {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-4);
    font-size: var(--text-xs);
  }
  .key {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-dim);
  }
  .sw {
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }
  .val {
    color: var(--text);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .plot {
    width: 100%;
    min-width: 0;
  }
  svg {
    display: block;
    overflow: visible;
    touch-action: none;
  }
  .grid {
    stroke: var(--separator);
    stroke-width: 1;
    shape-rendering: crispEdges;
  }
  .tick {
    fill: var(--text-faint);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
  }
  .cross {
    stroke: var(--border-hi);
    stroke-width: 1;
    shape-rendering: crispEdges;
  }
  .dot {
    stroke: var(--surface);
    stroke-width: 1.5;
  }
</style>
