<script lang="ts">
  import { locale, t } from "$lib/i18n";
  import { cellLevel, gridMax, gridTotal } from "$lib/stores/llm-accounts-store.svelte";

  interface Props {
    grid: number[][];
  }

  let { grid }: Props = $props();

  let max = $derived(gridMax(grid));
  let total = $derived(gridTotal(grid));

  let days = $derived.by(() => {
    const fmt = new Intl.DateTimeFormat($locale || "en", { weekday: "short" });
    return Array.from({ length: 7 }, (_, i) => fmt.format(new Date(Date.UTC(2024, 0, 1 + i))));
  });

  let peak = $derived.by(() => {
    let best = { day: -1, hour: -1, value: 0 };
    grid.forEach((row, day) =>
      row.forEach((v, hour) => {
        if (v > best.value) best = { day, hour, value: v };
      }),
    );
    return best;
  });

  function hourLabel(h: number): string {
    return `${String(h).padStart(2, "0")}h`;
  }
</script>

<div class="heatmap">
  <table>
    <caption class="sr-only">
      {$t("llm.accounts.heatmap_caption")}
    </caption>
    <thead>
      <tr>
        <th scope="col"><span class="sr-only">{$t("llm.accounts.heatmap_weekday")}</span></th>
        {#each Array.from({ length: 24 }, (_, h) => h) as h (h)}
          <th scope="col" class="hour" class:shown={h % 6 === 0}>
            {h % 6 === 0 ? hourLabel(h) : ""}
            <span class="sr-only">{hourLabel(h)}</span>
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each grid as row, day (day)}
        <tr>
          <th scope="row" class="day">{days[day]}</th>
          {#each row as value, hour (hour)}
            <td
              class="cell level-{cellLevel(value, max)}"
              class:peak={day === peak.day && hour === peak.hour && value > 0}
              title="{days[day]} {hourLabel(hour)} · {value}"
              aria-label="{days[day]} {hourLabel(hour)}: {value}"
            >
              <span class="sr-only">{value}</span>
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>

  <div class="legend">
    <span class="total">{total} {$t("llm.accounts.tool_calls")}</span>
    <span class="scale">
      <span class="scale-label">{$t("llm.accounts.heatmap_less")}</span>
      <i class="swatch level-0"></i>
      <i class="swatch level-1"></i>
      <i class="swatch level-2"></i>
      <i class="swatch level-3"></i>
      <i class="swatch level-4"></i>
      <span class="scale-label">{$t("llm.accounts.heatmap_more")}</span>
      <span class="peak-label">{$t("llm.accounts.heatmap_peak")} {max}</span>
    </span>
  </div>
</div>

<style>
  .heatmap {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    overflow-x: auto;
  }
  .legend {
    align-self: stretch;
  }
  table {
    border-collapse: separate;
    border-spacing: 2px;
    table-layout: fixed;
  }
  .hour {
    font-size: 9px;
    font-weight: 400;
    color: var(--text-dim);
    text-align: center;
    padding: 0;
    min-width: 14px;
    font-variant-numeric: tabular-nums;
  }
  .day {
    font-size: var(--text-caption);
    font-weight: 400;
    color: var(--text-dim);
    text-align: right;
    padding-right: var(--space-2);
    white-space: nowrap;
  }
  .cell {
    width: 14px;
    height: 14px;
    border-radius: 3px;
    background: var(--step);
    --step: var(--surface-hi);
  }
  .level-0 {
    --step: color-mix(in srgb, var(--accent) 6%, var(--surface-hi));
  }
  .level-1 {
    --step: color-mix(in srgb, var(--accent) 26%, var(--surface-hi));
  }
  .level-2 {
    --step: color-mix(in srgb, var(--accent) 50%, var(--surface-hi));
  }
  .level-3 {
    --step: color-mix(in srgb, var(--accent) 74%, var(--surface-hi));
  }
  .level-4 {
    --step: var(--accent);
  }
  .cell.peak {
    box-shadow: 0 0 0 1px var(--text-dim);
  }
  .legend {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .scale {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .scale-label {
    margin-inline: 4px;
  }
  .peak-label {
    margin-left: var(--space-3);
    font-variant-numeric: tabular-nums;
  }
  .swatch {
    width: 12px;
    height: 12px;
    border-radius: 3px;
    background: var(--step);
    display: inline-block;
  }
  .total {
    font-variant-numeric: tabular-nums;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }
</style>
