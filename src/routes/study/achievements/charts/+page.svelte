<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";
  import SegmentedControl from "$lib/study-components/SegmentedControl.svelte";

  type XpEntry = {
    source: string;
    amount: number;
    awarded_at: number;
    metadata: Record<string, unknown>;
  };

  type GamificationState = {
    xp: number;
    level: number;
    xp_to_next: number;
    level_progress_pct: number;
  };

  type Achievement = {
    code: string;
    tier: string;
    unlocked_at: number;
  };

  let loading = $state(true);
  let error = $state("");
  let entries = $state<XpEntry[]>([]);
  let achievements = $state<Achievement[]>([]);
  let xpState = $state<GamificationState | null>(null);
  let period = $state("30");

  const periodOptions = $derived([
    { value: "7", label: "7d" },
    { value: "30", label: "30d" },
    { value: "90", label: "90d" },
    { value: "365", label: $t("study.achv.charts.p_1y") },
    { value: "all", label: $t("study.anki.stats.p_all") },
  ]);

  function periodCutoff(p: string): number | null {
    if (p === "all") return null;
    const days = Number(p);
    if (!Number.isFinite(days)) return null;
    return Math.floor(Date.now() / 1000) - days * 86400;
  }

  async function load() {
    loading = true;
    error = "";
    try {
      const [hist, achs, st] = await Promise.all([
        pluginInvoke<XpEntry[]>("study", "study:gamification:history", {
          limit: 5000,
        }),
        pluginInvoke<Achievement[]>("study", "study:gamification:achievements"),
        pluginInvoke<GamificationState>("study", "study:gamification:state"),
      ]);
      entries = Array.isArray(hist) ? hist : [];
      achievements = Array.isArray(achs) ? achs : [];
      xpState = st;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function dayKey(secs: number): string {
    const d = new Date(secs * 1000);
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    return `${y}-${m}-${day}`;
  }

  function dayKeyToDate(key: string): Date {
    const [y, m, d] = key.split("-").map(Number);
    return new Date(y, m - 1, d);
  }

  const cutoffSecs = $derived(periodCutoff(period));

  const filteredEntries = $derived(
    cutoffSecs == null
      ? entries
      : entries.filter((e) => e.awarded_at >= cutoffSecs),
  );

  const filteredAchievements = $derived(
    cutoffSecs == null
      ? achievements
      : achievements.filter((a) => a.unlocked_at >= cutoffSecs),
  );

  const xpByDay = $derived.by(() => {
    const map = new Map<string, number>();
    for (const e of filteredEntries) {
      const k = dayKey(e.awarded_at);
      map.set(k, (map.get(k) ?? 0) + e.amount);
    }
    return Array.from(map.entries())
      .map(([day, amount]) => ({ day, amount }))
      .sort((a, b) => a.day.localeCompare(b.day));
  });

  const unlocksByDay = $derived.by(() => {
    const map = new Map<string, number>();
    for (const a of filteredAchievements) {
      const k = dayKey(a.unlocked_at);
      map.set(k, (map.get(k) ?? 0) + 1);
    }
    return Array.from(map.entries())
      .map(([day, count]) => ({ day, count }))
      .sort((a, b) => a.day.localeCompare(b.day));
  });

  function xpForLevel(L: number): number {
    return 50 * (L - 1) * (L - 1);
  }

  function levelForXp(xp: number): number {
    let level = 1;
    while (xpForLevel(level + 1) <= xp && level < 200) level++;
    return level;
  }

  const levelHistory = $derived.by(() => {
    const sorted = [...filteredEntries].sort(
      (a, b) => a.awarded_at - b.awarded_at,
    );
    let total = 0;
    let lastLevel = 1;
    if (sorted.length > 0 && xpState) {
      const totalNow = xpState.xp;
      const sumPeriod = sorted.reduce((s, e) => s + e.amount, 0);
      total = Math.max(0, totalNow - sumPeriod);
      lastLevel = levelForXp(total);
    }
    const map = new Map<string, number>();
    for (const e of sorted) {
      total += e.amount;
      const L = levelForXp(total);
      if (L > lastLevel) {
        const k = dayKey(e.awarded_at);
        if (!map.has(k) || (map.get(k) ?? 0) < L) {
          map.set(k, L);
        }
        lastLevel = L;
      }
    }
    return Array.from(map.entries())
      .map(([day, level]) => ({ day, level }))
      .sort((a, b) => a.day.localeCompare(b.day));
  });

  const streakHistory = $derived.by(() => {
    if (filteredEntries.length === 0) return [];
    const dayHasXp = new Set<string>();
    for (const e of filteredEntries) {
      dayHasXp.add(dayKey(e.awarded_at));
    }
    const allDays = Array.from(dayHasXp).sort();
    if (allDays.length === 0) return [];
    const start = dayKeyToDate(allDays[0]);
    const end = dayKeyToDate(allDays[allDays.length - 1]);
    const out: { day: string; length: number }[] = [];
    let streak = 0;
    const cursor = new Date(start);
    while (cursor <= end) {
      const k = dayKey(Math.floor(cursor.getTime() / 1000));
      if (dayHasXp.has(k)) {
        streak += 1;
      } else {
        streak = 0;
      }
      out.push({ day: k, length: streak });
      cursor.setDate(cursor.getDate() + 1);
    }
    return out;
  });

  function maxOf(values: number[], floor: number = 1): number {
    return values.length === 0 ? floor : Math.max(floor, ...values);
  }

  function chartArea(
    points: { x: number; y: number }[],
    width: number,
    height: number,
    pad: number,
  ): string {
    if (points.length === 0) return "";
    const path = points
      .map((p, i) => `${i === 0 ? "M" : "L"} ${p.x} ${p.y}`)
      .join(" ");
    return `${path} L ${width - pad} ${height - pad} L ${pad} ${
      height - pad
    } Z`;
  }

  const CHART_W = 360;
  const CHART_H = 140;
  const CHART_PAD = 24;

  function plotPoints(
    items: { day: string; value: number }[],
    maxVal: number,
  ): { x: number; y: number; day: string; value: number }[] {
    if (items.length === 0) return [];
    const innerW = CHART_W - CHART_PAD * 2;
    const innerH = CHART_H - CHART_PAD * 2;
    const denom = items.length === 1 ? 1 : items.length - 1;
    return items.map((it, i) => ({
      x: CHART_PAD + (i / denom) * innerW,
      y: CHART_PAD + innerH - (it.value / maxVal) * innerH,
      day: it.day,
      value: it.value,
    }));
  }

  const xpPlot = $derived.by(() => {
    const items = xpByDay.map((e) => ({ day: e.day, value: e.amount }));
    const maxVal = maxOf(items.map((e) => e.value));
    return { points: plotPoints(items, maxVal), maxVal, count: items.length };
  });

  const levelPlot = $derived.by(() => {
    const items = levelHistory.map((e) => ({ day: e.day, value: e.level }));
    const maxVal = maxOf(items.map((e) => e.value));
    return { points: plotPoints(items, maxVal), maxVal, count: items.length };
  });

  const streakPlot = $derived.by(() => {
    const items = streakHistory.map((e) => ({ day: e.day, value: e.length }));
    const maxVal = maxOf(items.map((e) => e.value));
    return { points: plotPoints(items, maxVal), maxVal, count: items.length };
  });

  const unlocksPlot = $derived.by(() => {
    const items = unlocksByDay.map((e) => ({ day: e.day, value: e.count }));
    const maxVal = maxOf(items.map((e) => e.value));
    return { points: plotPoints(items, maxVal), maxVal, count: items.length };
  });

  let hoveredChart = $state<string | null>(null);
  let hoveredIdx = $state<number | null>(null);

  function fmtDate(day: string): string {
    return dayKeyToDate(day).toLocaleDateString();
  }

  onMount(load);
</script>

<section class="charts-page">
  <PageHero
    title={$t("study.achv.charts.title")}
    subtitle={$t("study.achv.charts.subtitle")}
  />

  <div class="period-bar">
    <SegmentedControl
      options={periodOptions}
      bind:value={period}
      ariaLabel={$t("study.anki.stats.period_aria")}
    />
    <button class="back-btn" onclick={() => history.back()}>
      {$t("study.common.go_home")}
    </button>
  </div>

  {#if loading}
    <div class="state">Carregando…</div>
  {:else if error}
    <div class="state err">{error}</div>
  {:else if entries.length === 0}
    <div class="empty">
      <p>Sem histórico de XP no período selecionado.</p>
      <p class="muted">Estude um pouco e volte aqui pra ver charts.</p>
    </div>
  {:else}
    <div class="grid">
      <article class="card">
        <header class="card-head">
          <h3>XP por dia</h3>
          <span class="meta">
            {xpPlot.count} dia{xpPlot.count === 1 ? "" : "s"} ·
            pico {xpPlot.maxVal} XP
          </span>
        </header>
        {#if xpPlot.points.length > 0}
          <svg
            viewBox="0 0 {CHART_W} {CHART_H}"
            class="chart"
            role="img"
            aria-label="XP por dia"
            onmousemove={(e) => {
              const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
              const x = ((e.clientX - rect.left) / rect.width) * CHART_W;
              const denom = xpPlot.points.length === 1 ? 1 : xpPlot.points.length - 1;
              const innerW = CHART_W - CHART_PAD * 2;
              const idx = Math.round(((x - CHART_PAD) / innerW) * denom);
              hoveredChart = "xp";
              hoveredIdx = Math.max(0, Math.min(xpPlot.points.length - 1, idx));
            }}
            onmouseleave={() => {
              hoveredChart = null;
              hoveredIdx = null;
            }}
          >
            <path
              class="area"
              d={chartArea(xpPlot.points, CHART_W, CHART_H, CHART_PAD)}
            />
            <polyline
              class="line"
              points={xpPlot.points.map((p) => `${p.x},${p.y}`).join(" ")}
            />
            {#each xpPlot.points as p, i (i)}
              <circle
                cx={p.x}
                cy={p.y}
                r={hoveredChart === "xp" && hoveredIdx === i ? 4 : 2.5}
                class="dot"
                class:hovered={hoveredChart === "xp" && hoveredIdx === i}
              />
            {/each}
          </svg>
          {#if hoveredChart === "xp" && hoveredIdx !== null}
            <p class="tooltip mono">
              {fmtDate(xpPlot.points[hoveredIdx].day)} · {xpPlot.points[hoveredIdx].value} XP
            </p>
          {/if}
        {/if}
      </article>

      <article class="card">
        <header class="card-head">
          <h3>Level ao longo do tempo</h3>
          <span class="meta">
            {levelPlot.count} level-up{levelPlot.count === 1 ? "" : "s"} no período
          </span>
        </header>
        {#if levelPlot.points.length > 0}
          <svg
            viewBox="0 0 {CHART_W} {CHART_H}"
            class="chart"
            role="img"
            aria-label="Level ao longo do tempo"
            onmousemove={(e) => {
              const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
              const x = ((e.clientX - rect.left) / rect.width) * CHART_W;
              const denom = levelPlot.points.length === 1 ? 1 : levelPlot.points.length - 1;
              const innerW = CHART_W - CHART_PAD * 2;
              const idx = Math.round(((x - CHART_PAD) / innerW) * denom);
              hoveredChart = "level";
              hoveredIdx = Math.max(0, Math.min(levelPlot.points.length - 1, idx));
            }}
            onmouseleave={() => {
              hoveredChart = null;
              hoveredIdx = null;
            }}
          >
            <polyline
              class="line step"
              points={levelPlot.points.map((p) => `${p.x},${p.y}`).join(" ")}
            />
            {#each levelPlot.points as p, i (i)}
              <circle
                cx={p.x}
                cy={p.y}
                r={hoveredChart === "level" && hoveredIdx === i ? 4 : 3}
                class="dot accent"
                class:hovered={hoveredChart === "level" && hoveredIdx === i}
              />
            {/each}
          </svg>
          {#if hoveredChart === "level" && hoveredIdx !== null}
            <p class="tooltip mono">
              {fmtDate(levelPlot.points[hoveredIdx].day)} · L{levelPlot.points[hoveredIdx].value}
            </p>
          {/if}
        {:else}
          <p class="muted small">Nenhum level-up no período</p>
        {/if}
      </article>

      <article class="card">
        <header class="card-head">
          <h3>Streak (dias consecutivos)</h3>
          <span class="meta">
            pico {streakPlot.maxVal} dia{streakPlot.maxVal === 1 ? "" : "s"}
          </span>
        </header>
        {#if streakPlot.points.length > 0}
          <svg
            viewBox="0 0 {CHART_W} {CHART_H}"
            class="chart"
            role="img"
            aria-label="Streak ao longo do tempo"
            onmousemove={(e) => {
              const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
              const x = ((e.clientX - rect.left) / rect.width) * CHART_W;
              const denom = streakPlot.points.length === 1 ? 1 : streakPlot.points.length - 1;
              const innerW = CHART_W - CHART_PAD * 2;
              const idx = Math.round(((x - CHART_PAD) / innerW) * denom);
              hoveredChart = "streak";
              hoveredIdx = Math.max(0, Math.min(streakPlot.points.length - 1, idx));
            }}
            onmouseleave={() => {
              hoveredChart = null;
              hoveredIdx = null;
            }}
          >
            <polyline
              class="line warning"
              points={streakPlot.points.map((p) => `${p.x},${p.y}`).join(" ")}
            />
          </svg>
          {#if hoveredChart === "streak" && hoveredIdx !== null}
            <p class="tooltip mono">
              {fmtDate(streakPlot.points[hoveredIdx].day)} ·
              {streakPlot.points[hoveredIdx].value} dia{streakPlot.points[hoveredIdx].value === 1 ? "" : "s"}
            </p>
          {/if}
        {/if}
      </article>

      <article class="card">
        <header class="card-head">
          <h3>Unlocks por dia</h3>
          <span class="meta">
            {filteredAchievements.length} unlock{filteredAchievements.length === 1 ? "" : "s"} no período
          </span>
        </header>
        {#if unlocksPlot.points.length > 0}
          <svg
            viewBox="0 0 {CHART_W} {CHART_H}"
            class="chart"
            role="img"
            aria-label="Unlocks por dia"
            onmousemove={(e) => {
              const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
              const x = ((e.clientX - rect.left) / rect.width) * CHART_W;
              const denom = unlocksPlot.points.length === 1 ? 1 : unlocksPlot.points.length - 1;
              const innerW = CHART_W - CHART_PAD * 2;
              const idx = Math.round(((x - CHART_PAD) / innerW) * denom);
              hoveredChart = "unlocks";
              hoveredIdx = Math.max(0, Math.min(unlocksPlot.points.length - 1, idx));
            }}
            onmouseleave={() => {
              hoveredChart = null;
              hoveredIdx = null;
            }}
          >
            {#each unlocksPlot.points as p, i (i)}
              {@const innerH = CHART_H - CHART_PAD * 2}
              {@const barH = innerH - (p.y - CHART_PAD)}
              {@const barW = Math.min(
                18,
                (CHART_W - CHART_PAD * 2) / Math.max(unlocksPlot.points.length, 1) - 2,
              )}
              <rect
                x={p.x - barW / 2}
                y={p.y}
                width={barW}
                height={barH}
                class="bar success"
                class:hovered={hoveredChart === "unlocks" && hoveredIdx === i}
              />
            {/each}
          </svg>
          {#if hoveredChart === "unlocks" && hoveredIdx !== null}
            <p class="tooltip mono">
              {fmtDate(unlocksPlot.points[hoveredIdx].day)} ·
              {unlocksPlot.points[hoveredIdx].value} unlock{unlocksPlot.points[hoveredIdx].value === 1 ? "" : "s"}
            </p>
          {/if}
        {:else}
          <p class="muted small">Nenhum unlock no período</p>
        {/if}
      </article>
    </div>
  {/if}
</section>

<style>
  .charts-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.25);
    width: 100%;
    max-width: 920px;
    margin-inline: auto;
  }
  .period-bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .back-btn {
    background: transparent;
    border: none;
    color: var(--text);
    padding: 6px 12px;
    border-radius: var(--border-radius);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .back-btn:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }

  .state {
    padding: calc(var(--padding) * 2);
    text-align: center;
    color: var(--secondary);
    font-size: 14px;
  }
  .state.err {
    color: var(--error, var(--accent));
  }
  .empty {
    padding: calc(var(--padding) * 3) var(--padding);
    text-align: center;
    border: none;
    border-radius: var(--border-radius);
    background: color-mix(in oklab, var(--accent) 3%, transparent);
  }
  .empty p {
    margin: 4px 0;
  }
  .muted {
    color: var(--tertiary);
  }
  .small {
    font-size: 12px;
  }
  .mono {
    font-family: var(--font-mono, ui-monospace, monospace);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--padding);
  }
  @media (max-width: 760px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }

  .card {
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    padding: calc(var(--padding) * 0.9);
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 200px;
  }
  .card-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
  }
  .card-head h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }
  .card-head .meta {
    font-size: 11px;
    color: var(--tertiary);
  }

  .chart {
    width: 100%;
    height: auto;
    display: block;
  }
  .area {
    fill: color-mix(in oklab, var(--accent) 18%, transparent);
    stroke: none;
  }
  .line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .line.step {
    stroke-linejoin: miter;
  }
  .line.warning {
    stroke: var(--warning, var(--accent));
  }
  .dot {
    fill: var(--accent);
    transition: r 100ms ease;
  }
  .dot.accent {
    fill: var(--accent);
  }
  .dot.hovered {
    fill: var(--surface);
    stroke: var(--accent);
    stroke-width: 2;
  }
  .bar {
    fill: var(--accent);
    transition: opacity 100ms ease;
  }
  .bar.success {
    fill: var(--success, var(--accent));
  }
  .bar.hovered {
    opacity: 0.7;
  }
  .tooltip {
    margin: 0;
    font-size: 11px;
    color: var(--secondary);
    padding: 0 4px;
  }

  @media (prefers-reduced-motion: reduce) {
    .dot,
    .bar {
      transition: none;
    }
  }
</style>
