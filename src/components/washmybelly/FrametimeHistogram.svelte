<script lang="ts">
  import { t } from "$lib/i18n";
  import { histogramBars, type Bar, type Histogram } from "$lib/washmybelly/perf";

  /// How many frames took how long. Bars are tinted by the frame rate they
  /// stand for: 60 FPS and faster, down to 30, slower than 30. The last bar
  /// collects the rare frames slower than the 99.9th percentile.
  /// Drawing ported from the no-gap bar chart of bklitai (21st.dev).
  let { histogram, height = 140 }: { histogram: Histogram; height?: number } = $props();

  let width = $state(0);
  let hover = $state<Bar | null>(null);
  const PAD_B = 18;

  let bars = $derived(histogramBars(histogram, width, height - PAD_B, 1));
  let total = $derived(histogram.counts.reduce((a, b) => a + b, 0) + histogram.overflow);

  function tone(b: Bar): string {
    if (b.from >= 1000 / 30) return "slow";
    if (b.from >= 1000 / 60) return "mid";
    return "fast";
  }

  function range(b: Bar): string {
    return b.overflow ? `≥ ${b.from.toFixed(1)} ms` : `${b.from.toFixed(1)}–${b.to.toFixed(1)} ms`;
  }

  let lastTo = $derived(histogram.start_ms + histogram.counts.length * histogram.width_ms);
</script>

<div class="hist" bind:clientWidth={width} style:height="{height}px">
  {#if width > 0}
    <svg {width} {height} role="img" aria-label={$t("washmybelly.perf.histogram_title") as string}>
      <line class="base" x1="0" x2={width} y1={height - PAD_B + 0.5} y2={height - PAD_B + 0.5} />
      {#each bars as b, i (i)}
        <rect
          class="bar {tone(b)}"
          class:dim={hover && hover !== b}
          x={b.x}
          y={b.y}
          width={b.w}
          height={b.h}
          rx="1.5"
          role="presentation"
          onpointerenter={() => (hover = b)}
          onpointerleave={() => (hover = null)}
        />
      {/each}
      <text class="axis" x="0" y={height - 4}>{histogram.start_ms.toFixed(0)} ms</text>
      <text class="axis" x={width} y={height - 4} text-anchor="end">{lastTo.toFixed(0)} ms{histogram.overflow ? "+" : ""}</text>
    </svg>
  {/if}
  {#if hover}
    <div class="readout" role="status">
      <strong>{range(hover)}</strong>
      <span>{$t("washmybelly.perf.histogram_frames", { count: hover.count.toLocaleString(), share: total ? ((100 * hover.count) / total).toFixed(1) : "0" })}</span>
    </div>
  {/if}
</div>

<style>
  .hist { position: relative; width: 100%; }
  svg { display: block; overflow: visible; }
  .base { stroke: var(--content-border, var(--separator)); stroke-width: 1; }
  .bar { transition: opacity 120ms ease-out; }
  .bar.fast { fill: var(--success); }
  .bar.mid { fill: var(--warning); }
  .bar.slow { fill: var(--danger); }
  .bar.dim { opacity: 0.35; }
  .axis { font-size: 10px; fill: var(--text-dim); font-variant-numeric: tabular-nums; }
  .readout {
    position: absolute;
    top: 0;
    right: 0;
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
  @media (prefers-reduced-motion: reduce) { .bar { transition: none; } }
</style>
