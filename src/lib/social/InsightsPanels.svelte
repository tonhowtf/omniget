<script lang="ts">
  import { untrack } from "svelte";
  import { t, locale } from "$lib/i18n";
  import EngagementDraw from "./EngagementDraw.svelte";
  import { compact, social, tool, type Platform } from "./api";
  import { toolError, type Envelope, type Listing, type PostRow, type ToolError } from "./insights";
  import { PERIOD_ROWS, SERIES_KEYS, lastDays, periodArgs, seriesPath, signed, type Periods, type Series } from "./second";

  let { platform, ready, openProfile }: { platform: Platform; ready: boolean; openProfile: (handle: string) => void } = $props();

  type Panel = "periods" | "history" | "engagement";
  const W = 520;
  const H = 90;

  let panel = $state<Panel | null>(null);
  let panels = $derived<Panel[]>(platform === "x" ? ["periods", "history", "engagement"] : ["periods", "history"]);

  let mine = $state<PostRow[]>([]);
  let mineLoading = $state(false);
  let mineError = $state<ToolError | null>(null);

  let form = $state({ ...lastDays(new Date(), 7), auto: true, compareFrom: "", compareTo: "" });
  let periods = $state<Periods | null>(null);
  let periodsLoading = $state(false);
  let periodsError = $state<ToolError | null>(null);
  let invalid = $state(false);

  let picked = $state("");
  let series = $state<Series | null>(null);
  let seriesLoading = $state(false);
  let seriesError = $state<ToolError | null>(null);
  let lines = $derived(series ? SERIES_KEYS.map((k) => ({ key: k, path: seriesPath(series?.points ?? [], k, W, H), change: series?.change[k] ?? null })).filter((l) => l.path) : []);

  async function loadMine() {
    if (mine.length || mineLoading) return;
    mineLoading = true;
    mineError = null;
    try {
      const r = await social.call<Envelope<Listing<PostRow>>>(tool(platform, "analytics"), { report: "posts", limit: 40 });
      mine = r.result.items;
    } catch (e) {
      mineError = toolError(e);
    } finally {
      mineLoading = false;
    }
  }

  function show(p: Panel) {
    panel = panel === p ? null : p;
    if (panel === "history" || panel === "engagement") void loadMine();
  }

  async function comparePeriods() {
    const args = periodArgs(form);
    invalid = !args;
    if (!args || periodsLoading) return;
    periodsLoading = true;
    periodsError = null;
    try {
      const r = await social.call<Envelope<Periods>>(tool(platform, "analytics"), { report: "compare_periods", sample: 200, ...args });
      periods = r.result;
    } catch (e) {
      periodsError = toolError(e);
    } finally {
      periodsLoading = false;
    }
  }

  async function loadSeries() {
    if (!picked) {
      series = null;
      return;
    }
    seriesLoading = true;
    seriesError = null;
    try {
      const r = await social.call<{ result: Series }>(tool(platform, "analytics"), { report: "post_history", post: picked });
      series = r.result;
    } catch (e) {
      seriesError = toolError(e);
    } finally {
      seriesLoading = false;
    }
  }

  function rowLabel(key: string): string {
    if (key === "posts" || key.startsWith("engagement_")) return $t(`social_analytics.extras.periods.${key}`) as string;
    return $t("social_analytics.extras.periods.metric_mean", { metric: $t(`social_analytics.metric.${key.replace("_mean", "")}`) }) as string;
  }

  function num(n: number | null | undefined): string {
    return n == null ? "–" : compact(n, $locale ?? undefined);
  }

  function day(ts: number): string {
    try {
      return new Intl.DateTimeFormat($locale ?? undefined, { dateStyle: "medium" }).format(ts * 1000);
    } catch {
      return new Date(ts * 1000).toISOString().slice(0, 10);
    }
  }

  function moment(ms: number): string {
    try {
      return new Intl.DateTimeFormat($locale ?? undefined, { dateStyle: "medium", timeStyle: "short" }).format(ms);
    } catch {
      return new Date(ms).toISOString();
    }
  }

  function postLabel(p: PostRow): string {
    const text = (p.text || p.url).slice(0, 70);
    return p.created_at ? `${day(p.created_at)} · ${text}` : text;
  }

  $effect(() => {
    void platform;
    untrack(() => {
      panel = null;
      mine = [];
      periods = null;
      series = null;
      picked = "";
    });
  });
</script>

<section class="pn">
  <div class="segmented pn-tabs" role="group" aria-label={$t("social_analytics.extras.panels.label")}>
    {#each panels as p (p)}
      <button class="segmented-btn" aria-pressed={panel === p} aria-expanded={panel === p} disabled={!ready} onclick={() => show(p)}>{$t(`social_analytics.extras.panels.${p}`)}</button>
    {/each}
  </div>

  {#if panel === "periods"}
    <form class="pn-form" onsubmit={(e) => { e.preventDefault(); void comparePeriods(); }}>
      <fieldset class="pn-set">
        <legend>{$t("social_analytics.extras.periods.a")}</legend>
        <label class="field"><span class="field-label">{$t("social_analytics.extras.periods.from")}</span><input class="input" type="date" bind:value={form.from} required /></label>
        <label class="field"><span class="field-label">{$t("social_analytics.extras.periods.to")}</span><input class="input" type="date" bind:value={form.to} required /></label>
      </fieldset>
      <fieldset class="pn-set">
        <legend>{$t("social_analytics.extras.periods.b")}</legend>
        <label class="pn-check"><input type="checkbox" class="checkbox" bind:checked={form.auto} /> {$t("social_analytics.extras.periods.auto")}</label>
        {#if !form.auto}
          <label class="field"><span class="field-label">{$t("social_analytics.extras.periods.from")}</span><input class="input" type="date" bind:value={form.compareFrom} /></label>
          <label class="field"><span class="field-label">{$t("social_analytics.extras.periods.to")}</span><input class="input" type="date" bind:value={form.compareTo} /></label>
        {/if}
      </fieldset>
      <button class="btn btn-primary btn-sm" disabled={periodsLoading}>{#if periodsLoading}<span class="spinner"></span>{/if} {$t("social_analytics.extras.periods.run")}</button>
    </form>
    {#if invalid}
      <div class="notice notice-warning" role="alert"><div class="notice-body"><div class="notice-text">{$t("social_analytics.extras.periods.invalid")}</div></div></div>
    {:else if periodsError}
      <div class="notice notice-warning" role="alert">
        <div class="notice-body">
          <div class="notice-text">{periodsError.message}</div>
          {#if periodsError.next_step}<div class="notice-text pn-dim">{periodsError.next_step}</div>{/if}
        </div>
        <button class="btn btn-ghost btn-sm" onclick={() => void comparePeriods()}>{$t("social.common.retry")}</button>
      </div>
    {:else if periodsLoading && !periods}
      <div class="pn-state"><span class="spinner"></span><span>{$t("social_analytics.loading")}</span></div>
    {:else if periods}
      {#if periods.empty_reason}
        <div class="pn-state">{$t("social_analytics.extras.periods.empty")}</div>
      {:else}
        {#if !periods.coverage.covers_a || !periods.coverage.covers_b}
          <div class="notice notice-info" role="status"><div class="notice-body"><div class="notice-text">{$t("social_analytics.extras.periods.coverage")}</div></div></div>
        {/if}
        <div class="pn-scroll">
          <table class="pn-table">
            <thead>
              <tr>
                <th scope="col">{$t("social_analytics.extras.periods.stat")}</th>
                <th scope="col" class="pn-num">{day(periods.a.from)} → {day(periods.a.to - 1)}</th>
                <th scope="col" class="pn-num">{day(periods.b.from)} → {day(periods.b.to - 1)}</th>
                <th scope="col" class="pn-num">{$t("social_analytics.extras.periods.diff")}</th>
                <th scope="col" class="pn-num">%</th>
              </tr>
            </thead>
            <tbody>
              {#each PERIOD_ROWS as key (key)}
                {@const d = periods.difference[key]}
                {#if d && (d.a != null || d.b != null)}
                  <tr>
                    <th scope="row">{rowLabel(key)}</th>
                    <td class="pn-num">{num(d.a)}</td>
                    <td class="pn-num">{num(d.b)}</td>
                    <td class="pn-num" class:pn-up={(d.diff ?? 0) > 0} class:pn-down={(d.diff ?? 0) < 0}>{signed(d.diff)}</td>
                    <td class="pn-num" class:pn-up={(d.pct ?? 0) > 0} class:pn-down={(d.pct ?? 0) < 0}>{d.pct == null ? "–" : `${signed(d.pct, 1)}%`}</td>
                  </tr>
                {/if}
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/if}
  {:else if panel === "history"}
    <p class="pn-dim">{$t("social_analytics.extras.history.hint")}</p>
    {#if mineLoading}
      <div class="pn-state"><span class="spinner"></span><span>{$t("social_analytics.loading")}</span></div>
    {:else if mineError}
      <div class="notice notice-warning" role="alert">
        <div class="notice-body"><div class="notice-text">{mineError.message}</div></div>
        <button class="btn btn-ghost btn-sm" onclick={() => void loadMine()}>{$t("social.common.retry")}</button>
      </div>
    {:else if !mine.length}
      <div class="pn-state">{$t("social_analytics.extras.history.none")}</div>
    {:else}
      <label class="field">
        <span class="field-label">{$t("social_analytics.extras.history.pick")}</span>
        <select class="input" bind:value={picked} onchange={() => void loadSeries()}>
          <option value="">–</option>
          {#each mine as p (p.id)}<option value={p.id}>{postLabel(p)}</option>{/each}
        </select>
      </label>
      {#if seriesLoading}
        <div class="pn-state"><span class="spinner"></span></div>
      {:else if seriesError}
        <div class="notice notice-warning" role="alert">
          <div class="notice-body"><div class="notice-text">{seriesError.message}</div></div>
          <button class="btn btn-ghost btn-sm" onclick={() => void loadSeries()}>{$t("social.common.retry")}</button>
        </div>
      {:else if series && series.total === 0}
        <div class="pn-state">{$t("social_analytics.extras.history.none")}</div>
      {:else if series}
        <p class="pn-dim">{$t("social_analytics.extras.history.points", { count: series.total })}{#if series.first_at && series.last_at} · {moment(series.first_at)} → {moment(series.last_at)}{/if}</p>
        {#if series.total < 2}
          <div class="pn-state">{$t("social_analytics.extras.history.one_point")}</div>
        {:else}
          {#each lines as l (l.key)}
            <figure class="pn-line">
              <figcaption>{$t(`social_analytics.metric.${l.key}`)} <span class="pn-dim">{$t("social_analytics.extras.history.change")}: {signed(l.change, 0)}</span></figcaption>
              <svg viewBox="0 -4 {W} {H + 8}" role="img" aria-label={$t(`social_analytics.metric.${l.key}`)} preserveAspectRatio="none"><path d={l.path} /></svg>
            </figure>
          {/each}
        {/if}
        <div class="pn-scroll">
          <table class="pn-table">
            <thead>
              <tr>
                <th scope="col">{$t("social_analytics.extras.history.at")}</th>
                {#each SERIES_KEYS as k (k)}<th scope="col" class="pn-num">{$t(`social_analytics.metric.${k}`)}</th>{/each}
              </tr>
            </thead>
            <tbody>
              {#each [...series.points].reverse().slice(0, 30) as pt (pt.at)}
                <tr>
                  <th scope="row">{moment(pt.at)}</th>
                  {#each SERIES_KEYS as k (k)}<td class="pn-num">{num(pt[k])}</td>{/each}
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/if}
  {:else if panel === "engagement"}
    {#if mineLoading}
      <div class="pn-state"><span class="spinner"></span><span>{$t("social_analytics.loading")}</span></div>
    {:else if mineError}
      <div class="notice notice-warning" role="alert">
        <div class="notice-body"><div class="notice-text">{mineError.message}</div></div>
        <button class="btn btn-ghost btn-sm" onclick={() => void loadMine()}>{$t("social.common.retry")}</button>
      </div>
    {:else}
      <EngagementDraw posts={mine} {openProfile} label={postLabel} />
    {/if}
  {/if}
</section>

<style>
  .pn {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .pn-tabs {
    align-self: flex-start;
    max-width: 100%;
    overflow-x: auto;
  }

  .pn-form {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-3);
  }

  .pn-set {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    border: 0;
    min-width: 0;
  }

  .pn-set legend {
    padding: 0;
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .pn-check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .pn-dim {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .pn-state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-4) 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
    text-align: center;
  }

  .pn-scroll {
    overflow-x: auto;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pn-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }

  .pn-table th,
  .pn-table td {
    padding: 6px var(--space-3);
    text-align: left;
    white-space: nowrap;
  }

  .pn-table thead th {
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .pn-table tbody th {
    font-weight: 400;
  }

  .pn-table .pn-num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .pn-up {
    color: var(--success);
  }

  .pn-down {
    color: var(--danger);
  }

  .pn-line {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .pn-line svg {
    display: block;
    width: 100%;
    height: 70px;
  }

  .pn-line path {
    fill: none;
    stroke: var(--accent);
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
  }
</style>
