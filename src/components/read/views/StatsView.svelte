<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { fmtDuration, go, tr } from "$lib/read/ui-store.svelte";
  import type { Stats } from "$lib/read/types";
  import ReadIcon from "../ReadIcon.svelte";

  let s = $state<Stats | null>(null);
  let goalBooks = $state(12);
  let goalMin = $state(20);

  async function load() {
    s = await invoke<Stats>("read_stats").catch(() => null);
    if (s?.goal) {
      goalBooks = s.goal.books;
      goalMin = s.goal.minutes_per_day;
    }
  }
  onMount(load);

  async function saveGoal() {
    await invoke("read_set_goal", { goal: { year: new Date().getFullYear(), books: goalBooks, minutes_per_day: goalMin } });
    showToast("success", tr("read.stats.goal_saved"));
    await load();
  }

  let heat = $derived.by(() => {
    if (!s) return [];
    const days: { key: string; ms: number; future: boolean }[] = [];
    const today = new Date();
    const start = new Date(today);
    start.setDate(today.getDate() - 52 * 7 - today.getDay());
    for (let d = new Date(start); days.length < 53 * 7; d.setDate(d.getDate() + 1)) {
      const key = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
      days.push({ key, ms: s.days[key] ?? 0, future: d > today });
    }
    return days;
  });
  const level = (ms: number) => (ms <= 0 ? 0 : ms < 10 * 60_000 ? 1 : ms < 30 * 60_000 ? 2 : ms < 60 * 60_000 ? 3 : 4);
  let maxHour = $derived(s ? Math.max(1, ...s.hours) : 1);
  let months = $derived(s ? Object.entries(s.finished_by_month).slice(-12) : []);
  let maxMonth = $derived(Math.max(1, ...months.map(([, n]) => n)));
  let goalPct = $derived(s?.goal?.books ? Math.min(100, Math.round((s.finished_this_year / s.goal.books) * 100)) : 0);
  let todayPct = $derived(s?.goal?.minutes_per_day ? Math.min(100, Math.round((s.today_ms / (s.goal.minutes_per_day * 60_000)) * 100)) : 0);
</script>

<div class="r-h"><h1>{$t("read.nav.stats")}</h1></div>

{#if !s}
  <div class="r-empty"><ReadIcon name="refresh" size={22} spin /></div>
{:else}
  <div class="kpis">
    <div class="k"><b>{fmtDuration(s.today_ms)}</b><span>{$t("read.stats.today")}</span></div>
    <div class="k"><b>{fmtDuration(s.week_ms)}</b><span>{$t("read.stats.week")}</span></div>
    <div class="k"><b>{fmtDuration(s.month_ms)}</b><span>{$t("read.stats.month")}</span></div>
    <div class="k"><b>{fmtDuration(s.total_ms)}</b><span>{$t("read.stats.total")}</span></div>
    <div class="k"><b>{s.streak_days}</b><span>{$t("read.stats.streak")}</span></div>
    <div class="k"><b>{s.longest_streak}</b><span>{$t("read.stats.longest")}</span></div>
    <div class="k"><b>{s.finished}</b><span>{$t("read.stats.finished_all")}</span></div>
    <div class="k"><b>{s.chapters_read}</b><span>{$t("read.stats.chapters")}</span></div>
    <div class="k"><b>{s.annotations}</b><span>{$t("read.stats.annotations")}</span></div>
    <div class="k"><b>{fmtDuration(s.avg_session_ms)}</b><span>{$t("read.stats.avg_session")}</span></div>
  </div>

  <h2 class="r-sub">{$t("read.stats.goal", { year: new Date().getFullYear() })}</h2>
  <div class="r-panel goal">
    <div class="gr">
      <span>{$t("read.stats.books_goal", { done: s.finished_this_year, goal: s.goal?.books ?? "—" })}</span>
      <div class="r-bar big"><i style:width="{goalPct}%"></i></div>
    </div>
    <div class="gr">
      <span>{$t("read.stats.today_goal", { done: Math.round(s.today_ms / 60000), goal: s.goal?.minutes_per_day ?? "—" })}</span>
      <div class="r-bar big"><i style:width="{todayPct}%"></i></div>
    </div>
    <form class="r-line" onsubmit={(e) => { e.preventDefault(); void saveGoal(); }}>
      <label class="r-field">{$t("read.stats.books_per_year")}<input class="input" type="number" min="0" bind:value={goalBooks} /></label>
      <label class="r-field">{$t("read.stats.minutes_per_day")}<input class="input" type="number" min="0" bind:value={goalMin} /></label>
      <button class="btn btn-primary" type="submit">{$t("read.save")}</button>
    </form>
  </div>

  <h2 class="r-sub">{$t("read.stats.activity")}</h2>
  <div class="heat" role="img" aria-label={$t("read.stats.activity") as string}>
    {#each heat as d (d.key)}
      <i class="l{level(d.ms)}" class:fut={d.future} title="{d.key}: {fmtDuration(d.ms)}"></i>
    {/each}
  </div>

  <div class="two">
    <div>
      <h2 class="r-sub">{$t("read.stats.by_hour")}</h2>
      <div class="bars">
        {#each s.hours as h, i (i)}
          <div class="bar" title="{i}h: {fmtDuration(h)}"><i style:height="{Math.round((h / maxHour) * 100)}%"></i><span>{i % 6 === 0 ? i : ""}</span></div>
        {/each}
      </div>
    </div>
    <div>
      <h2 class="r-sub">{$t("read.stats.finished_months")}</h2>
      <div class="bars">
        {#each months as [m, n] (m)}
          <div class="bar" title="{m}: {n}"><i style:height="{Math.round((n / maxMonth) * 100)}%"></i><span>{m.slice(5)}</span></div>
        {:else}
          <p class="r-muted">{$t("read.stats.none_yet")}</p>
        {/each}
      </div>
    </div>
  </div>

  <div class="two">
    <div>
      <h2 class="r-sub">{$t("read.stats.top_books")}</h2>
      <ol class="top">
        {#each s.top_books as [id, title, ms] (id)}
          <li><button onclick={() => go({ kind: "book", id })}>{title}</button><span>{fmtDuration(ms)}</span></li>
        {/each}
      </ol>
    </div>
    <div>
      <h2 class="r-sub">{$t("read.stats.top_authors")}</h2>
      <ol class="top">
        {#each s.top_authors as [name, ms] (name)}
          <li><span class="n">{name}</span><span>{fmtDuration(ms)}</span></li>
        {/each}
      </ol>
      <h2 class="r-sub">{$t("read.stats.formats")}</h2>
      <div class="r-line">{#each s.formats as [f, n] (f)}<span class="r-chip">{f.toUpperCase()} · {n}</span>{/each}</div>
    </div>
  </div>
{/if}

<style>
  .kpis {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: var(--space-2);
  }
  .k {
    display: flex;
    flex-direction: column;
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .k b {
    font-size: var(--text-xl);
  }
  .k span {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .goal {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .gr {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--text-sm);
  }
  .big {
    height: 8px;
    border-radius: 4px;
  }
  .heat {
    display: grid;
    grid-template-rows: repeat(7, 11px);
    grid-auto-flow: column;
    grid-auto-columns: 11px;
    gap: 3px;
    overflow-x: auto;
    padding-bottom: 6px;
  }
  .heat i {
    border-radius: 2px;
    background: var(--fill-2);
  }
  .heat i.l1 {
    background: color-mix(in srgb, var(--accent) 30%, var(--fill-2));
  }
  .heat i.l2 {
    background: color-mix(in srgb, var(--accent) 55%, var(--fill-2));
  }
  .heat i.l3 {
    background: color-mix(in srgb, var(--accent) 80%, var(--fill-2));
  }
  .heat i.l4 {
    background: var(--accent);
  }
  .heat i.fut {
    opacity: 0;
  }
  .two {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: var(--space-5);
  }
  .bars {
    display: flex;
    align-items: flex-end;
    gap: 3px;
    height: 120px;
  }
  .bar {
    flex: 1;
    height: 100%;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    align-items: center;
    gap: 2px;
  }
  .bar i {
    width: 100%;
    min-height: 2px;
    border-radius: 3px 3px 0 0;
    background: var(--accent);
  }
  .bar span {
    font-size: 10px;
    color: var(--text-muted);
    height: 12px;
  }
  .top {
    margin: 0;
    padding-left: 20px;
  }
  .top li {
    padding: 3px 0;
  }
  .top li button {
    all: unset;
    cursor: pointer;
    font-weight: 600;
  }
  .top li span {
    color: var(--text-muted);
    font-size: var(--text-sm);
    margin-left: 8px;
  }
  .top li .n {
    color: var(--text);
    font-weight: 600;
    margin: 0;
    font-size: var(--text-base);
  }
</style>
