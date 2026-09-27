<script lang="ts">
  import { t } from "$lib/i18n";
  import { formatClock, frametimeAxis, nearest, seriesPaths, timeX, type Chart } from "$lib/washmybelly/perf";

  /// Frametime over the capture: the slowest frame of each slice as a line
  /// over a soft area, the fastest–slowest band behind it, dashed guides at
  /// 60 and 30 FPS, and ticks on top for stutters (amber) and freezes (red).
  /// Hover shows the moment and the range. Line, area gradient and hover
  /// pill ported from Motion's "Line graph" (motiondotdev, 21st.dev).
  let { chart, height = 180 }: { chart: Chart; height?: number } = $props();

  let width = $state(0);
  let hoverX = $state<number | null>(null);
  const TOP = 10;
  const uid = `ft-${Math.random().toString(36).slice(2, 8)}`;

  let plotH = $derived(height - TOP - 16);
  let yMax = $derived(frametimeAxis(chart.series));
  let paths = $derived(seriesPaths(chart.series, width, plotH, yMax));
  let guides = $derived([1000 / 60, 1000 / 30].filter((g) => g < yMax));
  let point = $derived(hoverX == null ? null : nearest(chart.series, hoverX, width));
  let clipped = $derived(chart.series.filter((s) => s[2] > yMax));

  function y(v: number): number {
    return TOP + plotH - (Math.min(v, yMax) / yMax) * plotH;
  }

  function move(e: PointerEvent) {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    hoverX = e.clientX - r.left;
  }
</script>

<div class="ftc" bind:clientWidth={width} style:height="{height}px">
  {#if width > 0 && chart.series.length > 1}
    <svg {width} {height} role="img" aria-label={$t("washmybelly.perf.timeline_title") as string} onpointermove={move} onpointerleave={() => (hoverX = null)}>
      <defs>
        <linearGradient id={uid} x1="0" x2="0" y1="0" y2="1">
          <stop offset="0%" stop-color="var(--accent)" stop-opacity="0.22" />
          <stop offset="100%" stop-color="var(--accent)" stop-opacity="0" />
        </linearGradient>
      </defs>
      {#each guides as g (g)}
        <line class="guide" x1="0" x2={width} y1={y(g)} y2={y(g)} />
        <text class="axis" x={width - 2} y={y(g) - 3} text-anchor="end">{Math.round(1000 / g)} FPS</text>
      {/each}
      <text class="axis" x="2" y={TOP + 8}>{yMax} ms</text>
      <g transform="translate(0 {TOP})">
        <path class="area" d={paths.area} fill="url(#{uid})" />
        <path class="band" d={paths.band} />
        <path class="line" d={paths.line} />
      </g>
      {#each chart.stutter_events as ev, i (i)}
        <line class="tick stutter" x1={timeX(ev.t_ms, chart.series, width)} x2={timeX(ev.t_ms, chart.series, width)} y1="0" y2="6" />
      {/each}
      {#each chart.freeze_events as ev, i (i)}
        <line class="tick freeze" x1={timeX(ev.t_ms, chart.series, width)} x2={timeX(ev.t_ms, chart.series, width)} y1="0" y2="9" />
      {/each}
      {#each clipped as c, i (i)}
        <circle class="clip" cx={timeX(c[0], chart.series, width)} cy={TOP + 1} r="2" />
      {/each}
      <line class="base" x1="0" x2={width} y1={TOP + plotH + 0.5} y2={TOP + plotH + 0.5} />
      <text class="axis" x="0" y={height - 2}>0:00</text>
      <text class="axis" x={width} y={height - 2} text-anchor="end">{formatClock(chart.duration_ms / 1000)}</text>
      {#if point && hoverX != null}
        <line class="cursor" x1={timeX(point[0], chart.series, width)} x2={timeX(point[0], chart.series, width)} y1={TOP} y2={TOP + plotH} />
        <circle class="dot" cx={timeX(point[0], chart.series, width)} cy={y(point[2])} r="3.5" />
      {/if}
    </svg>
    {#if point}
      <div class="readout" role="status">
        <strong>{formatClock(point[0] / 1000)}</strong>
        <span>{point[1].toFixed(1)}–{point[2].toFixed(1)} ms</span>
      </div>
    {/if}
  {/if}
</div>

<style>
  .ftc { position: relative; width: 100%; }
  svg { display: block; overflow: visible; touch-action: none; }
  .guide { stroke: var(--content-border, var(--separator)); stroke-width: 1; stroke-dasharray: 4 4; }
  .base { stroke: var(--content-border, var(--separator)); stroke-width: 1; }
  .axis { font-size: 10px; fill: var(--text-dim); font-variant-numeric: tabular-nums; }
  .band { fill: var(--accent); opacity: 0.18; }
  .line { fill: none; stroke: var(--accent); stroke-width: 1.5; stroke-linejoin: round; }
  .tick { stroke-width: 1.5; stroke-linecap: round; }
  .tick.stutter { stroke: var(--warning); }
  .tick.freeze { stroke: var(--danger); stroke-width: 2; }
  .clip { fill: var(--danger); }
  .cursor { stroke: var(--text-dim); stroke-width: 1; opacity: 0.6; }
  .dot { fill: var(--surface); stroke: var(--accent); stroke-width: 1.5; }
  .readout {
    position: absolute;
    top: 0;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    gap: 8px;
    align-items: baseline;
    padding: 3px 8px;
    border-radius: 999px;
    background: var(--surface-hi, var(--surface));
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.14), inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator));
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
    pointer-events: none;
  }
  .readout span { color: var(--text-dim); }
</style>
