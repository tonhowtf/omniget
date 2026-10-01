<script lang="ts">
  import { t } from "$lib/i18n";
  import type { CostBucket } from "$lib/stores/llm-telemetry-store.svelte";

  interface Props {
    days: CostBucket[];
  }

  let { days }: Props = $props();

  let max = $derived(Math.max(0, ...days.map((d) => d.cost_usd)));
  let axisTop = $derived.by(() => {
    if (max === 0) return 0.01;
    const exp = Math.floor(Math.log10(max));
    const base = Math.pow(10, exp);
    for (const m of [1, 2, 5, 10]) if (max <= m * base) return m * base;
    return 10 * base;
  });
  let total = $derived(days.reduce((s, d) => s + d.cost_usd, 0));
  let peakIdx = $derived(days.reduce((best, d, i) => (d.cost_usd > (days[best]?.cost_usd ?? -1) ? i : best), 0));

  let hover = $state(-1);

  function usd(v: number): string {
    return `$${v.toFixed(v === 0 ? 2 : Math.min(8, Math.max(2, 1 - Math.floor(Math.log10(Math.abs(v))))))}`;
  }

  function dayLabel(day: string): string {
    return day.slice(5).replace("-", "/");
  }
</script>

<div class="chart">
  <div class="axis">
    <span>{usd(axisTop)}</span>
    <span>{usd(axisTop / 2)}</span>
    <span>$0</span>
  </div>
  <div class="plot">
    <div class="grid" aria-hidden="true"><i></i><i></i><i></i></div>
    <div class="cols">
      {#each days as d, i (d.day)}
        <div
          class="col"
          class:peak={i === peakIdx}
          role="img"
          aria-label={`${d.day}: ${usd(d.cost_usd)}, ${d.calls} ${$t("llm.observatory.calls")}`}
          onpointerenter={() => (hover = i)}
          onpointerleave={() => (hover = -1)}
        >
          {#if i === peakIdx && d.cost_usd > 0}
            <span class="cap-label">{usd(d.cost_usd)}</span>
          {/if}
          <div class="fill" style:height="{Math.max(0, (d.cost_usd / axisTop) * 100)}%"></div>
          {#if hover === i}
            <span class="tip">{dayLabel(d.day)} · {usd(d.cost_usd)} · {d.calls} {$t("llm.observatory.calls")}</span>
          {/if}
        </div>
      {/each}
    </div>
  </div>
</div>
<div class="ticks">
  {#each days as d, i (d.day)}
    <span class="tick">{i % 2 === 0 ? dayLabel(d.day) : ""}</span>
  {/each}
</div>
<div class="total">{$t("llm.observatory.cost_total")} <strong>{usd(total)}</strong></div>

<details class="values">
  <summary>{$t("llm.observatory.table")}</summary>
  <table><caption>{$t("llm.observatory.cost_by_day")}</caption><tbody>
    {#each days as day (day.day)}<tr><th scope="row">{day.day}</th><td>{usd(day.cost_usd)}</td><td>{day.calls} {$t("llm.observatory.calls")}</td></tr>{/each}
  </tbody></table>
</details>
<style>
  .values { margin-top: var(--space-3); font-size: var(--text-sm); }
  summary { cursor: pointer; }
  table { width: 100%; text-align: left; border-collapse: collapse; }
  th, td { padding: var(--space-2); border-bottom: 1px solid var(--border); font-variant-numeric: tabular-nums; }

  .chart {
    display: flex;
    gap: var(--space-2);
    height: 132px;
  }
  .axis {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    font-size: var(--text-caption);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    text-align: right;
    min-width: 34px;
    margin-block: -6px -7px;
  }
  .plot {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  .grid {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    pointer-events: none;
  }
  .grid i {
    display: block;
    height: 1px;
    background: var(--border);
    opacity: 0.6;
  }
  .cols {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 2px;
  }
  .col {
    position: relative;
    flex: 1;
    min-width: 0;
    max-width: 24px;
    height: 100%;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
  }
  .fill {
    background: var(--accent);
    border-radius: var(--radius-xs) var(--radius-xs) 0 0;
  }
  .col.peak .fill {
    background: var(--accent);
  }
  .cap-label {
    font-size: var(--text-caption);
    color: var(--text-muted);
    text-align: center;
    font-variant-numeric: tabular-nums;
    margin-bottom: 2px;
    white-space: nowrap;
  }
  .tip {
    position: absolute;
    bottom: calc(100% + 4px);
    left: 50%;
    transform: translateX(-50%);
    padding: 2px 6px;
    border-radius: var(--radius-xs);
    background: var(--surface-hi);
    border: 1px solid var(--border);
    font-size: var(--text-caption);
    color: var(--text);
    white-space: nowrap;
    pointer-events: none;
    z-index: 2;
  }
  .ticks {
    display: flex;
    justify-content: space-between;
    gap: 2px;
    margin-left: calc(34px + var(--space-2));
    margin-top: var(--space-1);
  }
  .tick {
    flex: 1;
    min-width: 0;
    max-width: 24px;
    font-size: 9px;
    color: var(--text-dim);
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .total {
    margin-top: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
</style>
