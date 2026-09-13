<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";
  import StatCard from "$lib/study-components/StatCard.svelte";
  import SegmentedControl from "$lib/study-components/SegmentedControl.svelte";

  type HeatmapEntry = { date: string; count: number };
  type BucketEntry = { bucket: string; count: number };
  type FutureDueEntry = { days_ahead: number; count: number };
  type RetentionEntry = { date: string; retention: number; sample: number };
  type NotetypeStat = {
    notetype_id: number;
    name: string;
    cards_count: number;
    avg_interval: number;
  };

  type GraphsResponse = {
    period: string;
    heatmap: HeatmapEntry[];
    interval_distribution: BucketEntry[];
    ease_distribution: BucketEntry[];
    future_due: FutureDueEntry[];
    daily_load_avg: number;
    retention_curve: RetentionEntry[];
    avg_secs_per_review: number;
    total_secs_studied: number;
    reviews_count: number;
    notetype_stats: NotetypeStat[];
  };

  let period = $state<string>("1m");
  let loading = $state(true);
  let error = $state("");
  let data = $state<GraphsResponse | null>(null);

  const periodOptions = $derived([
    { value: "1m", label: $t("study.anki.stats.p_1m") },
    { value: "3m", label: $t("study.anki.stats.p_3m") },
    { value: "6m", label: $t("study.anki.stats.p_6m") },
    { value: "1y", label: $t("study.anki.stats.p_1y") },
    { value: "all", label: $t("study.anki.stats.p_all") },
  ]);

  async function load() {
    loading = true;
    error = "";
    try {
      await pluginInvoke("study", "study:anki:storage:open");
      data = await pluginInvoke<GraphsResponse>("study", "study:anki:stats:graphs", {
        period,
      });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void period;
    load();
  });

  onMount(load);

  function fmtHours(secs: number): string {
    if (secs < 60) return `${secs}s`;
    const mins = Math.floor(secs / 60);
    if (mins < 60) return `${mins}min`;
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return m === 0 ? `${h}h` : `${h}h${m}m`;
  }

  function fmtNumber(n: number, digits = 0): string {
    return n.toLocaleString("pt-BR", { maximumFractionDigits: digits });
  }

  function heatmapWeeks(entries: HeatmapEntry[]): {
    weeks: { date: string; count: number; level: number }[][];
  } {
    if (entries.length === 0) return { weeks: [] };
    const map = new Map<string, number>();
    for (const e of entries) map.set(e.date, e.count);
    const max = Math.max(...entries.map((e) => e.count), 1);
    const todayIso = new Date().toISOString().slice(0, 10);
    const today = new Date(todayIso);
    const totalDays = 365;
    const start = new Date(today);
    start.setDate(start.getDate() - (totalDays - 1));
    const startDow = start.getDay();
    start.setDate(start.getDate() - startDow);
    const cells: { date: string; count: number; level: number }[] = [];
    for (let i = 0; i < 53 * 7; i++) {
      const d = new Date(start);
      d.setDate(start.getDate() + i);
      if (d > today) break;
      const iso = d.toISOString().slice(0, 10);
      const count = map.get(iso) ?? 0;
      const level =
        count === 0
          ? 0
          : count >= max
            ? 4
            : count >= Math.ceil((max * 3) / 4)
              ? 3
              : count >= Math.ceil(max / 2)
                ? 2
                : 1;
      cells.push({ date: iso, count, level });
    }
    const weeks: { date: string; count: number; level: number }[][] = [];
    for (let i = 0; i < cells.length; i += 7) {
      weeks.push(cells.slice(i, i + 7));
    }
    return { weeks };
  }

  const heatmap = $derived(data ? heatmapWeeks(data.heatmap) : { weeks: [] });

  const totalReviews = $derived(data?.reviews_count ?? 0);
  const totalHours = $derived(data?.total_secs_studied ?? 0);
  const avgSecs = $derived(data?.avg_secs_per_review ?? 0);
  const dailyLoad = $derived(data?.daily_load_avg ?? 0);

  function maxCount(rows: { count: number }[]): number {
    return rows.reduce((m, r) => (r.count > m ? r.count : m), 0);
  }
</script>

<section class="study-page">
  <PageHero title={$t("study.anki.stats.title")} />

  <div class="period-row">
    <SegmentedControl
      options={periodOptions}
      bind:value={period}
      ariaLabel={$t("study.anki.stats.period_aria")}
    />
    <a href="/study/anki/stats/revlog" class="advanced-link">{$t("study.anki.stats.revlog_link")}</a>
  </div>

  {#if loading}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if data}
    <div class="kpi-grid">
      <StatCard label={$t("study.anki.stats.reviews")} value={fmtNumber(totalReviews)} hint={$t("study.anki.stats.in_period")} />
      <StatCard label={$t("study.anki.stats.total_time")} value={fmtHours(totalHours)} hint={$t("study.anki.stats.studying")} />
      <StatCard
        label={$t("study.anki.stats.avg_time")}
        value={`${avgSecs.toFixed(1)}s`}
        hint={$t("study.anki.stats.per_review")}
      />
      <StatCard
        label={$t("study.anki.stats.daily_load")}
        value={dailyLoad.toFixed(1)}
        hint={$t("study.anki.stats.next_30")}
      />
    </div>

    <section class="card heatmap-card">
      <header class="card-head">
        <h2>{$t("study.anki.stats.activity_12m")}</h2>
        <div class="legend">
          <span>{$t("study.anki.stats.less")}</span>
          <span class="cell level-0" aria-hidden="true"></span>
          <span class="cell level-1" aria-hidden="true"></span>
          <span class="cell level-2" aria-hidden="true"></span>
          <span class="cell level-3" aria-hidden="true"></span>
          <span class="cell level-4" aria-hidden="true"></span>
          <span>{$t("study.anki.stats.more")}</span>
        </div>
      </header>
      {#if heatmap.weeks.length === 0}
        <p class="muted small">{$t("study.anki.stats.no_reviews")}</p>
      {:else}
        <div class="heatmap" role="img" aria-label={$t("study.anki.stats.heatmap_aria")}>
          {#each heatmap.weeks as week, i (i)}
            <div class="week">
              {#each week as day (day.date)}
                <span
                  class="cell level-{day.level}"
                  title="{day.date}: {day.count} {day.count === 1 ? $t('study.anki.stats.review_one') : $t('study.anki.stats.review_many')}"
                ></span>
              {/each}
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <div class="grid-2">
      <section class="card">
        <header class="card-head">
          <h2>{$t("study.anki.stats.intervals_dist")}</h2>
          <small class="muted">{$t("study.anki.stats.cards_in_review")}</small>
        </header>
        {@render barChart(data.interval_distribution)}
      </section>

      <section class="card">
        <header class="card-head">
          <h2>{$t("study.anki.stats.ease_dist")}</h2>
          <small class="muted">{$t("study.anki.stats.cards_in_review")}</small>
        </header>
        {@render barChart(data.ease_distribution)}
      </section>

      <section class="card">
        <header class="card-head">
          <h2>{$t("study.anki.stats.future_load")}</h2>
          <small class="muted">{$t("study.anki.stats.next_30")}</small>
        </header>
        {@render futureChart(data.future_due)}
      </section>

      <section class="card">
        <header class="card-head">
          <h2>{$t("study.anki.stats.retention_curve")}</h2>
          <small class="muted">{$t("study.anki.stats.acc_per_day")}</small>
        </header>
        {@render retentionChart(data.retention_curve)}
      </section>
    </div>

    {#if data.notetype_stats.length > 0}
      <section class="card">
        <header class="card-head">
          <h2>{$t("study.anki.stats.by_notetype")}</h2>
        </header>
        <table class="nt-table">
          <thead>
            <tr>
              <th>{$t("study.anki.stats.th_notetype")}</th>
              <th class="num">{$t("study.anki.stats.th_cards")}</th>
              <th class="num">{$t("study.anki.stats.th_avg_interval")}</th>
            </tr>
          </thead>
          <tbody>
            {#each data.notetype_stats as nt (nt.notetype_id)}
              <tr>
                <td>{nt.name}</td>
                <td class="num mono">{fmtNumber(nt.cards_count)}</td>
                <td class="num mono">{nt.avg_interval.toFixed(1)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </section>
    {/if}
  {/if}
</section>

{#snippet barChart(rows: BucketEntry[])}
  {@const max = maxCount(rows)}
  {#if rows.length === 0 || max === 0}
    <p class="muted small">Sem dados</p>
  {:else}
    <ul class="bar-list">
      {#each rows as r (r.bucket)}
        {@const pct = (r.count / max) * 100}
        <li>
          <span class="bar-label">{r.bucket}</span>
          <span class="bar-track" aria-hidden="true">
            <span class="bar-fill" style:width="{pct}%"></span>
          </span>
          <span class="bar-count">{fmtNumber(r.count)}</span>
        </li>
      {/each}
    </ul>
  {/if}
{/snippet}

{#snippet futureChart(rows: FutureDueEntry[])}
  {@const filled = (() => {
    const m = new Map(rows.map((r) => [r.days_ahead, r.count] as const));
    const out: { day: number; count: number }[] = [];
    for (let i = 0; i < 30; i++) out.push({ day: i, count: m.get(i) ?? 0 });
    return out;
  })()}
  {@const max = filled.reduce((m, r) => (r.count > m ? r.count : m), 0)}
  {#if max === 0}
    <p class="muted small">Nenhum card programado.</p>
  {:else}
    <div class="vbar" role="img" aria-label={$t("study.anki.stats.future_aria")}>
      {#each filled as f (f.day)}
        {@const h = max === 0 ? 0 : (f.count / max) * 100}
        <span
          class="vbar-col"
          style:--h="{h}%"
          title="Dia +{f.day}: {f.count} cards"
        >
          <span class="vbar-fill"></span>
        </span>
      {/each}
    </div>
    <div class="vbar-axis">
      <span>hoje</span>
      <span>+15d</span>
      <span>+30d</span>
    </div>
  {/if}
{/snippet}

{#snippet retentionChart(rows: RetentionEntry[])}
  {#if rows.length < 2}
    <p class="muted small">{$t("study.anki.stats.min_days")}</p>
  {:else}
    {@const w = 360}
    {@const h = 120}
    {@const xs = rows.map((_, i) => (i / (rows.length - 1)) * w)}
    {@const ys = rows.map((r) => h - r.retention * h)}
    {@const points = xs.map((x, i) => `${x.toFixed(1)},${ys[i].toFixed(1)}`).join(" ")}
    <svg viewBox="0 0 {w} {h}" class="retention-svg" preserveAspectRatio="none">
      <line
        x1="0"
        y1={h * 0.1}
        x2={w}
        y2={h * 0.1}
        class="grid-line"
      />
      <line
        x1="0"
        y1={h * 0.5}
        x2={w}
        y2={h * 0.5}
        class="grid-line"
      />
      <line
        x1="0"
        y1={h * 0.9}
        x2={w}
        y2={h * 0.9}
        class="grid-line"
      />
      <polyline {points} class="retention-line" fill="none" />
    </svg>
    <div class="retention-axis">
      <span>{rows[0].date}</span>
      <span>{rows[rows.length - 1].date}</span>
    </div>
  {/if}
{/snippet}

<style>
  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
    width: 100%;
    max-width: 1100px;
    margin-inline: auto;
  }
  .muted {
    color: var(--tertiary);
    font-size: 13px;
  }
  .muted.small {
    font-size: 12px;
    text-align: center;
    padding: calc(var(--padding) * 2);
  }
  .error {
    color: var(--error);
    font-size: 13px;
  }

  .period-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }
  .advanced-link {
    color: var(--accent);
    font-size: 12px;
    text-decoration: none;
    padding: 4px 10px;
    border-radius: var(--border-radius);
  }
  .advanced-link:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }

  .kpi-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--padding);
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: calc(var(--padding) * 2);
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
  }
  .card-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--padding);
  }
  .card-head h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 500;
    color: var(--secondary);
  }

  .grid-2 {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--padding);
  }

  .heatmap-card .legend {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--tertiary);
  }
  .heatmap {
    display: flex;
    gap: 3px;
    overflow-x: auto;
    padding: 4px 0;
  }
  .heatmap .week {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .cell {
    width: 11px;
    height: 11px;
    border-radius: 2px;
    display: inline-block;
  }
  .cell.level-0 {
    background: color-mix(in oklab, var(--input-border) 40%, transparent);
  }
  .cell.level-1 {
    background: color-mix(in oklab, var(--accent) 25%, transparent);
  }
  .cell.level-2 {
    background: color-mix(in oklab, var(--accent) 50%, transparent);
  }
  .cell.level-3 {
    background: color-mix(in oklab, var(--accent) 75%, transparent);
  }
  .cell.level-4 {
    background: var(--accent);
  }

  .bar-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .bar-list li {
    display: grid;
    grid-template-columns: 80px 1fr 50px;
    gap: 8px;
    align-items: center;
  }
  .bar-label {
    font-size: 11px;
    color: var(--tertiary);
    font-family: var(--font-mono, monospace);
  }
  .bar-track {
    height: 14px;
    background: color-mix(in oklab, var(--input-border) 50%, transparent);
    border-radius: 3px;
    overflow: hidden;
  }
  .bar-fill {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 400ms ease-out;
  }
  .bar-count {
    text-align: right;
    font-family: var(--font-mono, monospace);
    font-variant-numeric: tabular-nums;
    font-size: 12px;
    color: var(--secondary);
  }

  .vbar {
    display: grid;
    grid-template-columns: repeat(30, 1fr);
    gap: 2px;
    align-items: end;
    height: 120px;
  }
  .vbar-col {
    height: 100%;
    display: flex;
    align-items: end;
  }
  .vbar-fill {
    display: block;
    width: 100%;
    height: var(--h);
    background: var(--accent);
    border-radius: 2px 2px 0 0;
    transition: height 400ms ease-out;
  }
  .vbar-axis {
    display: flex;
    justify-content: space-between;
    font-size: 10px;
    color: var(--tertiary);
    margin-top: 4px;
  }

  .retention-svg {
    width: 100%;
    height: 120px;
    display: block;
  }
  .grid-line {
    stroke: color-mix(in oklab, var(--input-border) 60%, transparent);
    stroke-width: 1;
    stroke-dasharray: 2 4;
  }
  .retention-line {
    stroke: var(--accent);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .retention-axis {
    display: flex;
    justify-content: space-between;
    font-size: 10px;
    color: var(--tertiary);
    margin-top: 4px;
    font-family: var(--font-mono, monospace);
  }

  .nt-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .nt-table th,
  .nt-table td {
    padding: 8px 10px;
    border-bottom: none;
    text-align: left;
  }
  .nt-table th {
    color: var(--tertiary);
    font-weight: 500;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .nt-table td {
    color: var(--secondary);
  }
  .nt-table .num {
    text-align: right;
  }
  .nt-table .mono {
    font-family: var(--font-mono, monospace);
    font-variant-numeric: tabular-nums;
  }

  @media (max-width: 720px) {
    .kpi-grid {
      grid-template-columns: repeat(2, 1fr);
    }
    .grid-2 {
      grid-template-columns: 1fr;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .bar-fill,
    .vbar-fill {
      transition: none;
    }
  }
</style>
