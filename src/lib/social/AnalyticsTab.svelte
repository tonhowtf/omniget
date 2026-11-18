<script lang="ts">
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open as shellOpen } from "@tauri-apps/plugin-shell";
  import { t, locale } from "$lib/i18n";
  import SocialIcon from "./SocialIcon.svelte";
  import InsightsDigest from "./InsightsDigest.svelte";
  import InsightsPanels from "./InsightsPanels.svelte";
  import { hasExtras } from "./second";
  import { ago, compact, errKey, NET, social, TITLE_KEY, tool, type Platform, type Profile } from "./api";
  import {
    REPO_SORTS,
    SAMPLES,
    cleanHandle,
    hasReplies,
    heatGrid,
    heatLevel,
    reportsFor,
    sortRows,
    toolError,
    weekdayName,
    whoKey,
    type Compare,
    type Envelope,
    type HashtagRow,
    type Listing,
    type MediaRow,
    type Outliers,
    type PostRow,
    type RepoRow,
    type RepoTotals,
    type Report,
    type Summary,
    type Times,
    type ToolError,
    type TrendRow,
    type WordRow,
  } from "./insights";

  let { platform, me, openProfile }: { platform: Platform; me: Profile; openProfile: (handle: string) => void } = $props();

  const METRICS = ["likes", "replies", "reposts", "saves", "views"] as const;
  const POST_COLUMNS = ["created_at", "likes", "replies", "reposts", "saves", "views", "engagement", "lift"] as const;
  const HOURS = Array.from({ length: 24 }, (_, i) => i);

  let draft = $state("");
  let handle = $state("");
  let sample = $state(50);
  let replies = $state(false);
  let view = $state<Report>("summary");
  let loading = $state(false);
  let error = $state<ToolError | null>(null);
  let env = $state<Envelope<unknown> | null>(null);
  let sort = $state<string>("created_at");
  let asc = $state(false);
  let unit = $state<"day" | "week">("day");
  let tableSort = $state<{ key: string; asc: boolean }>({ key: "", asc: false });
  let rivals = $state<string[]>([]);
  let rivalDraft = $state("");
  let exporting = $state(false);
  let exported = $state<{ path: string; rows: number } | null>(null);
  let seq = 0;

  let who = $derived(handle || me.handle);
  let shown = $derived(env && env.report === view ? env : null);
  let summary = $derived(shown?.report === "summary" ? (shown.result as Summary) : null);
  let posts = $derived(shown?.report === "posts" ? (shown.result as Listing<PostRow>) : null);
  let times = $derived(shown?.report === "times" ? (shown.result as Times) : null);
  let grid = $derived(times ? heatGrid(platform === NET.codehub ? times.cells.map((c) => ({ ...c, mean: c.posts })) : times.cells) : null);
  let reports = $derived(reportsFor(platform));
  let repos = $derived(shown?.report === "repos" ? (shown.result as Listing<RepoRow> & { totals: RepoTotals }) : null);
  let repoSort = $state<string>("stars");
  let repoAsc = $state(false);
  let media = $derived(shown?.report === "media" ? (shown.result as Listing<MediaRow>) : null);
  let hashtags = $derived(shown?.report === "hashtags" ? (shown.result as Listing<HashtagRow>) : null);
  let words = $derived(shown?.report === "words" ? (shown.result as Listing<WordRow>) : null);
  let trend = $derived(shown?.report === "trend" ? (shown.result as Listing<TrendRow>) : null);
  let trendMax = $derived(Math.max(1, ...(trend?.items ?? []).map((r) => r.engagement)));
  let outliers = $derived(shown?.report === "outliers" ? (shown.result as Outliers) : null);
  let compare = $derived(shown?.report === "compare" ? (shown.result as Compare) : null);

  function rivalsKey() {
    return `omniget.social.rivals.${platform}`;
  }

  function readRivals(): string[] {
    try {
      const raw = JSON.parse(localStorage.getItem(rivalsKey()) ?? "[]");
      return Array.isArray(raw) ? raw.filter((x) => typeof x === "string").slice(0, 4) : [];
    } catch {
      return [];
    }
  }

  function saveRivals() {
    try {
      localStorage.setItem(rivalsKey(), JSON.stringify(rivals));
    } catch {
      return;
    }
  }

  async function load(refresh = false, append = false) {
    if (view === "compare" && rivals.length === 0) {
      seq += 1;
      env = null;
      error = null;
      loading = false;
      return;
    }
    const mine = ++seq;
    loading = true;
    error = null;
    const args: Record<string, unknown> = { report: view, sample, limit: 100 };
    if (handle) args[whoKey(platform)] = handle;
    if (hasReplies(platform) && replies) args.include_replies = true;
    if (refresh) args.refresh = true;
    if (view === "repos") {
      args.sort = repoSort;
      args.order = repoAsc ? "asc" : "desc";
      if (append && repos) args.offset = repos.items.length;
    }
    if (view === "posts") {
      args.sort = sort;
      args.order = asc ? "asc" : "desc";
      if (append && posts) args.offset = posts.items.length;
    }
    if (view === "trend") args.unit = unit;
    if (view === "compare") args.handles = [who, ...rivals];
    try {
      const r = await social.call<Envelope<unknown>>(tool(platform, "analytics"), args);
      if (mine !== seq) return;
      if (append && posts) {
        const next = r.result as Listing<PostRow>;
        env = { ...r, result: { ...next, items: [...posts.items, ...next.items] } };
      } else if (append && repos) {
        const next = r.result as Listing<RepoRow>;
        env = { ...r, result: { ...next, items: [...repos.items, ...next.items] } };
      } else {
        env = r;
      }
    } catch (e) {
      if (mine === seq) error = toolError(e);
    } finally {
      if (mine === seq) loading = false;
    }
  }

  $effect(() => {
    void platform;
    untrack(() => {
      handle = "";
      draft = "";
      if (!reportsFor(platform).includes(view)) view = "summary";
      env = null;
      exported = null;
      rivals = readRivals();
      void load();
    });
  });

  function analyze() {
    handle = cleanHandle(draft);
    if (handle.toLowerCase() === me.handle.toLowerCase()) handle = "";
    exported = null;
    env = null;
    void load();
  }

  function pick(r: Report) {
    if (view === r) return;
    view = r;
    tableSort = { key: "", asc: false };
    void load();
  }

  function sortPosts(key: string) {
    if (sort === key) asc = !asc;
    else {
      sort = key;
      asc = false;
    }
    void load();
  }

  function sortRepos(key: string) {
    if (repoSort === key) repoAsc = !repoAsc;
    else {
      repoSort = key;
      repoAsc = false;
    }
    void load();
  }

  function typeLabel(kind: string): string {
    return platform === NET.codehub ? kind.replaceAll("_", " ") : ($t(`social_analytics.type.${kind}`) as string);
  }

  function sortTable(key: string) {
    tableSort = tableSort.key === key ? { key, asc: !tableSort.asc } : { key, asc: false };
  }

  function sorted<T extends Record<string, unknown>>(rows: T[]): T[] {
    return tableSort.key ? sortRows(rows, tableSort.key, tableSort.asc) : rows;
  }

  function ariaSort(active: boolean, ascending: boolean): "ascending" | "descending" | "none" {
    if (!active) return "none";
    return ascending ? "ascending" : "descending";
  }

  function addRival() {
    const h = cleanHandle(rivalDraft);
    rivalDraft = "";
    if (!h || rivals.length >= 4) return;
    if ([who, ...rivals].some((x) => x.toLowerCase() === h.toLowerCase())) return;
    rivals = [...rivals, h];
    saveRivals();
    void load();
  }

  function removeRival(h: string) {
    rivals = rivals.filter((x) => x !== h);
    saveRivals();
    void load();
  }

  async function exportCsv() {
    if (exporting) return;
    exporting = true;
    error = null;
    const args: Record<string, unknown> = { sample };
    if (handle) args[whoKey(platform)] = handle;
    if (hasReplies(platform) && replies) args.include_replies = true;
    try {
      exported = await social.call<{ path: string; rows: number }>(tool(platform, "analytics_export"), args);
    } catch (e) {
      error = toolError(e);
    } finally {
      exporting = false;
    }
  }

  function reveal(path: string) {
    void invoke("reveal_file", { path }).catch(() => {});
  }

  function external(url: string) {
    if (url) void shellOpen(url).catch(() => {});
  }

  function num(n: number | null | undefined): string {
    return n == null ? "–" : compact(n, $locale);
  }

  function date(ts: number | null | undefined): string {
    if (!ts) return "–";
    try {
      return new Date(ts * 1000).toLocaleDateString($locale, { day: "2-digit", month: "short", year: "numeric" });
    } catch {
      return new Date(ts * 1000).toISOString().slice(0, 10);
    }
  }

  function errorLine(e: ToolError): string {
    const key = errKey(`${e.code}: x`);
    if (key !== "unknown" && key !== "invalid_arguments") return $t(`social.errors.${key}`, { platform: $t(TITLE_KEY[platform]) }) as string;
    return e.message;
  }
</script>

<div class="an">
  {#if hasExtras(platform)}
    <InsightsDigest {platform} ready={!loading && (env !== null || error !== null)} {openProfile} />
  {/if}
  <form class="an-bar" onsubmit={(e) => { e.preventDefault(); analyze(); }}>
    <label class="field an-grow">
      <span class="field-label">{$t("social_analytics.handle_label")}</span>
      <input class="input" bind:value={draft} placeholder={$t("social_analytics.handle_placeholder", { handle: me.handle })} autocomplete="off" spellcheck="false" />
    </label>
    <label class="field">
      <span class="field-label">{$t("social_analytics.sample_label")}</span>
      <select class="input" bind:value={sample} onchange={() => void load()}>
        {#each SAMPLES as n (n)}<option value={n}>{n}</option>{/each}
      </select>
    </label>
    {#if hasReplies(platform)}
      <label class="an-check"><input type="checkbox" class="checkbox" bind:checked={replies} onchange={() => void load()} /> {$t(platform === NET.forums ? "social_analytics.include_comments" : "social_analytics.include_replies")}</label>
    {/if}
    <button class="btn btn-primary btn-sm" disabled={loading}>{$t("social_analytics.analyze")}</button>
    <button type="button" class="btn btn-ghost btn-sm" disabled={loading} onclick={() => void load(true)} title={$t("social_analytics.refresh")} aria-label={$t("social_analytics.refresh")}><SocialIcon name="refresh" /></button>
    <button type="button" class="btn btn-secondary btn-sm" disabled={exporting || loading} onclick={() => void exportCsv()}>
      {#if exporting}<span class="spinner"></span>{:else}<SocialIcon name="download" />{/if} {$t("social_analytics.export")}
    </button>
  </form>

  <div class="segmented an-tabs" role="tablist" aria-label={$t("social.tabs.analytics")}>
    {#each reports as r (r)}
      <button class="segmented-btn" role="tab" aria-selected={view === r} onclick={() => pick(r)}>{$t(`social_analytics.reports.${r}`)}</button>
    {/each}
  </div>

  {#if exported}
    <div class="notice notice-success" role="status">
      <div class="notice-body"><div class="notice-text">{$t("social_analytics.exported", { rows: exported.rows, path: exported.path })}</div></div>
      <button class="btn btn-ghost btn-sm" onclick={() => exported && reveal(exported.path)}>{$t("social_analytics.reveal")}</button>
    </div>
  {/if}

  {#if error}
    <div class="notice notice-warning" role="alert">
      <div class="notice-body">
        <div class="notice-text">{errorLine(error)}</div>
        {#if error.next_step}<div class="notice-text an-dim">{error.next_step}</div>{/if}
      </div>
      <button class="btn btn-ghost btn-sm" onclick={() => void load()}>{$t("social.common.retry")}</button>
    </div>
  {/if}

  {#if shown && shown.handle}
    <p class="an-meta">
      <button class="an-link" onclick={() => openProfile(shown?.handle ?? "")}>@{shown.handle}</button>
      · {$t("social_analytics.meta", { count: shown.sample ?? 0 })}
      {#if shown.fetched_at} · {ago(Math.floor(shown.fetched_at / 1000), $locale)}{/if}
      {#if shown.cached} · {$t("social_analytics.cached")}{/if}
    </p>
  {/if}
  {#if shown?.partial_reason}
    <div class="notice notice-info" role="status"><div class="notice-body"><span class="notice-title">{$t("social_analytics.partial")}</span><div class="notice-text">{shown.partial_reason}</div></div></div>
  {/if}

  {#if view === "compare"}
    <div class="an-compare-bar">
      <p class="an-hint">{$t("social_analytics.compare_hint")}</p>
      <form class="an-rival" onsubmit={(e) => { e.preventDefault(); addRival(); }}>
        <input class="input" bind:value={rivalDraft} placeholder={$t("social_analytics.compare_add_placeholder")} aria-label={$t("social_analytics.compare_add_placeholder")} autocomplete="off" spellcheck="false" disabled={rivals.length >= 4} />
        <button class="btn btn-secondary btn-sm" disabled={!rivalDraft.trim() || rivals.length >= 4}><SocialIcon name="plus" /> {$t("social_analytics.compare_add")}</button>
      </form>
      <div class="an-chips">
        <span class="tag tag-accent">@{who}</span>
        {#each rivals as h (h)}
          <span class="tag">@{h}<button class="an-x" onclick={() => removeRival(h)} aria-label={$t("social_analytics.compare_remove", { handle: h })}>×</button></span>
        {/each}
      </div>
    </div>
  {/if}

  {#if loading && !shown}
    <div class="an-state"><span class="spinner"></span><span>{$t("social_analytics.loading")}</span></div>
  {:else if view === "compare" && rivals.length === 0}
    <div class="an-state">{$t("social_analytics.compare_need")}</div>
  {:else if shown && shown.sample === 0}
    <div class="an-state">{$t("social_analytics.empty")}</div>
  {:else if summary}
    <div class="an-cards">
      <div class="an-card">
        <span class="an-card-label">{$t("social_analytics.cards.posts")}</span>
        <strong>{summary.analyzed}</strong>
        <span class="an-dim">{date(summary.from)} → {date(summary.to)}</span>
        <span class="tag {summary.confidence === 'high' ? 'tag-success' : 'tag-warning'}">{$t(`social_analytics.confidence.${summary.confidence}`)}</span>
      </div>
      <div class="an-card">
        <span class="an-card-label">{$t("social_analytics.cards.engagement")}</span>
        <strong>{num(summary.engagement.mean)}</strong>
        <span class="an-dim">{$t("social_analytics.stat.median")} {num(summary.engagement.median)} · {$t("social_analytics.stat.total")} {num(summary.engagement.total)}</span>
        <span class="an-dim">{$t("social_analytics.engagement_formula")}</span>
      </div>
      {#each METRICS as m (m)}
        {@const v = summary.metrics[m]}
        <div class="an-card" class:an-off={!v?.available}>
          <span class="an-card-label">{$t(`social_analytics.metric.${m}`)}</span>
          {#if v?.available}
            <strong>{num(v.mean)}</strong>
            <span class="an-dim">{$t("social_analytics.stat.median")} {num(v.median)} · {$t("social_analytics.stat.total")} {num(v.total)}</span>
          {:else}
            <strong>–</strong>
            <span class="an-dim">{$t("social_analytics.not_shown")}</span>
          {/if}
        </div>
      {/each}
    </div>
    <h3 class="an-h">{$t("social_analytics.types_title")}</h3>
    <div class="an-chips">
      {#each Object.entries(summary.types) as [kind, count] (kind)}
        <span class="tag">{typeLabel(kind)} · {count}</span>
      {/each}
    </div>
    <h3 class="an-h">{$t("social_analytics.unavailable_title")}</h3>
    <p class="an-hint">{$t("social_analytics.unavailable_hint")}</p>
    <div class="an-chips">
      {#each summary.unavailable as u (u.metric)}
        <span class="tag" title={u.reason}>{$t(`social_analytics.unavailable.${u.metric}`)}</span>
      {/each}
    </div>
  {:else if posts}
    <div class="an-scroll">
      <table class="an-table">
        <thead>
          <tr>
            <th scope="col">{$t("social_analytics.col.post")}</th>
            <th scope="col">{$t("social_analytics.col.type")}</th>
            {#each POST_COLUMNS as c (c)}
              <th scope="col" class="an-num" aria-sort={ariaSort(sort === c, asc)}>
                <button class="an-th" onclick={() => sortPosts(c)} title={$t("social_analytics.sort_by", { column: $t(`social_analytics.metric.${c}`) })}>
                  {$t(`social_analytics.metric.${c}`)}{#if sort === c}<span aria-hidden="true">{asc ? " ↑" : " ↓"}</span>{/if}
                </button>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each posts.items as r (r.id)}
            <tr>
              <td class="an-text">
                <button class="an-link" onclick={() => external(r.url)} title={$t("social_analytics.open_post")}>{r.text || r.url}</button>
                {#if r.outlier}<span class="tag an-out an-out-{r.outlier}">{$t(`social_analytics.outlier.${r.outlier}`)}</span>{/if}
              </td>
              <td>{typeLabel(r.type)} · {$t(`social_analytics.media.${r.media}`)}</td>
              <td class="an-num">{date(r.created_at)}</td>
              <td class="an-num">{num(r.likes)}</td>
              <td class="an-num">{num(r.replies)}</td>
              <td class="an-num">{num(r.reposts)}</td>
              <td class="an-num">{num(r.saves)}</td>
              <td class="an-num">{num(r.views)}</td>
              <td class="an-num">{num(r.engagement)}</td>
              <td class="an-num">{r.lift == null ? "–" : `${r.lift}x`}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    {#if posts.truncated}
      <button class="btn btn-ghost an-more" disabled={loading} onclick={() => void load(false, true)}>{$t("social.common.more")}</button>
    {/if}
  {:else if times && grid}
    <p class="an-hint">{$t(platform === NET.codehub ? "social_analytics.times_hint_activity" : "social_analytics.times_hint", { zone: times.timezone, min: times.thin_below })}</p>
    {#if times.best}
      <p class="an-best"><SocialIcon name="star" size={14} /> {$t("social_analytics.times_best", { day: weekdayName(times.best.weekday, $locale), hour: times.best.hour })}</p>
    {/if}
    <div class="an-scroll">
      <div class="an-heat" role="table" aria-label={$t("social_analytics.reports.times")}>
        <div class="an-heat-row" role="row">
          <span class="an-heat-day" role="columnheader"></span>
          {#each HOURS as h (h)}<span class="an-heat-hour" role="columnheader">{h}</span>{/each}
        </div>
        {#each grid.rows as row, d (d)}
          <div class="an-heat-row" role="row">
            <span class="an-heat-day" role="rowheader">{weekdayName(d, $locale)}</span>
            {#each row as c, h (h)}
              {#if c}
                {@const label = $t("social_analytics.cell", { day: weekdayName(d, $locale), hour: h, posts: c.posts, mean: num(c.mean) })}
                <span class="an-cell an-l{heatLevel(c.mean, grid.max)}" class:an-thin={c.thin} role="cell" tabindex="0" title={label} aria-label={label}>{c.posts}</span>
              {:else}
                <span class="an-cell" role="cell"></span>
              {/if}
            {/each}
          </div>
        {/each}
      </div>
    </div>
  {:else if media}
    <div class="an-scroll">
      <table class="an-table">
        <thead>
          <tr>
            {#each [["media", "col.media"], ["posts", "col.posts"], ["engagement_mean", "col.mean"], ["engagement_median", "col.median"], ["views_mean", "col.views_mean"]] as [key, label], i (key)}
              <th scope="col" class:an-num={i > 0} aria-sort={ariaSort(tableSort.key === key, tableSort.asc)}>
                <button class="an-th" onclick={() => sortTable(key)}>{$t(`social_analytics.${label}`)}{#if tableSort.key === key}<span aria-hidden="true">{tableSort.asc ? " ↑" : " ↓"}</span>{/if}</button>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each sorted(media.items) as r (r.media)}
            <tr><td>{$t(`social_analytics.media.${r.media}`)}</td><td class="an-num">{r.posts}</td><td class="an-num">{num(r.engagement_mean)}</td><td class="an-num">{num(r.engagement_median)}</td><td class="an-num">{num(r.views_mean)}</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if hashtags}
    {#if hashtags.total === 0}
      <div class="an-state">{$t("social_analytics.hashtags_empty")}</div>
    {:else}
      <div class="an-scroll">
        <table class="an-table">
          <thead>
            <tr>
              {#each [["hashtag", "col.hashtag"], ["uses", "col.uses"], ["engagement_mean", "col.mean"], ["engagement_median", "col.median"]] as [key, label], i (key)}
                <th scope="col" class:an-num={i > 0} aria-sort={ariaSort(tableSort.key === key, tableSort.asc)}>
                  <button class="an-th" onclick={() => sortTable(key)}>{$t(`social_analytics.${label}`)}{#if tableSort.key === key}<span aria-hidden="true">{tableSort.asc ? " ↑" : " ↓"}</span>{/if}</button>
                </th>
              {/each}
            </tr>
          </thead>
          <tbody>
            {#each sorted(hashtags.items) as r (r.hashtag)}
              <tr><td>#{r.hashtag}</td><td class="an-num">{r.uses}</td><td class="an-num">{num(r.engagement_mean)}</td><td class="an-num">{num(r.engagement_median)}</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {:else if words}
    <p class="an-hint">{$t("social_analytics.words_note")}</p>
    {#if words.total === 0}
      <div class="an-state">{$t("social_analytics.words_empty")}</div>
    {:else}
      <div class="an-scroll">
        <table class="an-table">
          <thead>
            <tr>
              {#each [["word", "col.word"], ["uses", "col.uses"], ["posts", "col.posts"]] as [key, label], i (key)}
                <th scope="col" class:an-num={i > 0} aria-sort={ariaSort(tableSort.key === key, tableSort.asc)}>
                  <button class="an-th" onclick={() => sortTable(key)}>{$t(`social_analytics.${label}`)}{#if tableSort.key === key}<span aria-hidden="true">{tableSort.asc ? " ↑" : " ↓"}</span>{/if}</button>
                </th>
              {/each}
            </tr>
          </thead>
          <tbody>
            {#each sorted(words.items) as r (r.word)}
              <tr><td>{r.word}</td><td class="an-num">{r.uses}</td><td class="an-num">{r.posts}</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {:else if repos}
    <p class="an-hint">{$t("social_analytics.repos_totals", { repos: repos.totals.repos, stars: num(repos.totals.stars), forks: num(repos.totals.forks), issues: num(repos.totals.open_issues) })} {$t("social_analytics.repos_note")}</p>
    <div class="an-scroll">
      <table class="an-table">
        <thead>
          <tr>
            <th scope="col">{$t("social_analytics.col.repo")}</th>
            {#each REPO_SORTS as c (c)}
              <th scope="col" class="an-num" aria-sort={ariaSort(repoSort === c, repoAsc)}>
                <button class="an-th" onclick={() => sortRepos(c)} title={$t("social_analytics.sort_by", { column: $t(`social_analytics.col.${c}`) })}>
                  {$t(`social_analytics.col.${c}`)}{#if repoSort === c}<span aria-hidden="true">{repoAsc ? " ↑" : " ↓"}</span>{/if}
                </button>
              </th>
            {/each}
            <th scope="col">{$t("social_analytics.col.language")}</th>
          </tr>
        </thead>
        <tbody>
          {#each repos.items as r (r.repo)}
            <tr>
              <td class="an-text">
                <button class="an-link" onclick={() => external(r.url)} title={r.description || r.repo}>{r.repo}</button>
                {#if r.fork}<span class="tag">{$t("social_analytics.repo_fork")}</span>{/if}
                {#if r.archived}<span class="tag tag-warning">{$t("social_analytics.repo_archived")}</span>{/if}
              </td>
              <td class="an-num">{num(r.stars)}</td>
              <td class="an-num">{num(r.forks)}</td>
              <td class="an-num">{num(r.open_issues)}</td>
              <td class="an-num">{date(r.pushed)}</td>
              <td>{r.language ?? "–"}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    {#if repos.truncated}
      <button class="btn btn-ghost an-more" disabled={loading} onclick={() => void load(false, true)}>{$t("social.common.more")}</button>
    {/if}
  {:else if trend}
    <div class="segmented an-unit" role="tablist">
      <button class="segmented-btn" role="tab" aria-selected={unit === "day"} onclick={() => { unit = "day"; void load(); }}>{$t("social_analytics.trend.day")}</button>
      <button class="segmented-btn" role="tab" aria-selected={unit === "week"} onclick={() => { unit = "week"; void load(); }}>{$t("social_analytics.trend.week")}</button>
    </div>
    <div class="an-scroll">
      <table class="an-table">
        <thead>
          <tr><th scope="col">{$t("social_analytics.col.period")}</th><th scope="col" class="an-num">{$t("social_analytics.col.posts")}</th><th scope="col" class="an-num">{$t("social_analytics.metric.engagement")}</th><th scope="col" class="an-num">{$t("social_analytics.col.mean")}</th><th scope="col" aria-hidden="true"></th></tr>
        </thead>
        <tbody>
          {#each trend.items as r (r.start)}
            <tr>
              <td>{r.start}</td><td class="an-num">{r.posts}</td><td class="an-num">{num(r.engagement)}</td><td class="an-num">{num(r.mean)}</td>
              <td class="an-barcell" aria-hidden="true"><span class="an-barfill" style:width={`${Math.max(2, (r.engagement / trendMax) * 100)}%`}></span></td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if outliers}
    <p class="an-hint">
      {$t("social_analytics.outliers_hint", { metric: $t(`social_analytics.metric.${outliers.metric}`), median: num(outliers.median) })}
      <span class="tag {outliers.confidence === 'high' ? 'tag-success' : 'tag-warning'}">{$t(`social_analytics.confidence.${outliers.confidence}`)}</span>
    </p>
    {#if outliers.total === 0}
      <div class="an-state">{$t("social_analytics.outliers_empty")}</div>
    {:else}
      <ul class="an-list">
        {#each outliers.items as r (r.id)}
          <li class="an-row">
            <span class="tag an-out an-out-{r.outlier}">{r.lift}x · {$t(`social_analytics.outlier.${r.outlier}`)}</span>
            <button class="an-link an-row-text" onclick={() => external(r.url)} title={$t("social_analytics.open_post")}>{r.text || r.url}</button>
            <span class="an-dim">{date(r.created_at)} · {$t(`social_analytics.metric.${outliers.metric}`)} {num(outliers.metric === "views" ? r.views : r.engagement)}</span>
          </li>
        {/each}
      </ul>
    {/if}
  {:else if compare}
    <p class="an-hint">{$t("social_analytics.compare_sample", { count: compare.sample_each })} <span class="tag {compare.confidence === 'high' ? 'tag-success' : 'tag-warning'}">{$t(`social_analytics.confidence.${compare.confidence}`)}</span></p>
    <div class="an-scroll">
      <table class="an-table">
        <thead>
          <tr>
            <th scope="col">{$t("social_analytics.col.metric")}</th>
            {#each compare.accounts as acc (acc.handle)}
              <th scope="col" class="an-num"><button class="an-link" onclick={() => openProfile(acc.handle)}>@{acc.handle}</button></th>
            {/each}
          </tr>
        </thead>
        <tbody>
          <tr>
            <th scope="row">{$t("social_analytics.cards.engagement")}</th>
            {#each compare.accounts as acc (acc.handle)}<td class="an-num">{num(acc.summary.engagement.mean)} <span class="an-dim">({num(acc.summary.engagement.median)})</span></td>{/each}
          </tr>
          {#each METRICS as m (m)}
            <tr>
              <th scope="row">{$t(`social_analytics.metric.${m}`)}</th>
              {#each compare.accounts as acc (acc.handle)}
                {@const v = acc.summary.metrics[m]}
                <td class="an-num">{#if v?.available}{num(v.mean)} <span class="an-dim">({num(v.median)})</span>{:else}–{/if}</td>
              {/each}
            </tr>
          {/each}
          <tr>
            <th scope="row">{$t("social_analytics.cards.period")}</th>
            {#each compare.accounts as acc (acc.handle)}<td class="an-num">{date(acc.summary.from)} → {date(acc.summary.to)}</td>{/each}
          </tr>
        </tbody>
      </table>
    </div>
    <p class="an-hint">{$t("social_analytics.compare_legend")}</p>
  {/if}
  {#if loading && shown}<div class="an-state"><span class="spinner"></span></div>{/if}
  {#if hasExtras(platform)}
    <InsightsPanels {platform} ready={!loading} {openProfile} />
  {/if}
</div>

<style>
  .an {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .an-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .an-grow {
    flex: 1;
    min-width: 180px;
  }

  .an-check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .an-tabs {
    align-self: flex-start;
    max-width: 100%;
    overflow-x: auto;
  }

  .an-unit {
    align-self: flex-start;
  }

  .an-meta,
  .an-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.5;
  }

  .an-dim {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .an-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .an-link:hover,
  .an-link:focus-visible {
    text-decoration: underline;
  }

  .an-state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-6) 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
    text-align: center;
  }

  .an-cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: var(--space-2);
  }

  .an-card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    min-width: 0;
  }

  .an-card strong {
    font-size: var(--text-xl);
    font-variant-numeric: tabular-nums;
  }

  .an-card .tag {
    align-self: flex-start;
    height: auto;
    min-height: 20px;
    white-space: normal;
  }

  .an-card-label {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-weight: 600;
  }

  .an-off {
    opacity: 0.6;
  }

  .an-h {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-weight: 600;
  }

  .an-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .an-x {
    margin-left: 2px;
    padding: 0 2px;
    border: 0;
    background: none;
    color: inherit;
    cursor: pointer;
  }

  .an-compare-bar {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .an-rival {
    display: flex;
    gap: var(--space-2);
    max-width: 420px;
  }

  .an-rival .input {
    flex: 1;
    min-width: 0;
  }

  .an-scroll {
    overflow-x: auto;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .an-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }

  .an-table th,
  .an-table td {
    padding: 8px 10px;
    text-align: left;
    border-bottom: var(--hairline) solid var(--content-border);
    vertical-align: top;
  }

  .an-table tbody tr:last-child td,
  .an-table tbody tr:last-child th {
    border-bottom: 0;
  }

  .an-table th {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-dim);
    white-space: nowrap;
  }

  .an-table .an-num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .an-th {
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .an-th:hover,
  .an-th:focus-visible {
    color: var(--text);
  }

  .an-text {
    min-width: 220px;
    max-width: 380px;
  }

  .an-text .an-link {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  .an-out {
    margin-top: 4px;
  }

  .an-out-mild {
    color: var(--info);
    background: color-mix(in srgb, var(--info) 14%, transparent);
  }

  .an-out-strong {
    color: var(--warning);
    background: color-mix(in srgb, var(--warning) 14%, transparent);
  }

  .an-out-huge {
    color: var(--success);
    background: color-mix(in srgb, var(--success) 16%, transparent);
  }

  .an-more {
    align-self: center;
  }

  .an-best {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--text-sm);
  }

  .an-heat {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-3);
    min-width: 640px;
  }

  .an-heat-row {
    display: grid;
    grid-template-columns: 44px repeat(24, minmax(18px, 1fr));
    gap: 2px;
    align-items: center;
  }

  .an-heat-day,
  .an-heat-hour {
    font-size: 10px;
    color: var(--text-muted);
    text-align: center;
  }

  .an-heat-day {
    text-align: left;
  }

  .an-cell {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 24px;
    border-radius: 4px;
    background: var(--fill-1);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }

  .an-cell:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .an-l1 {
    background: color-mix(in srgb, var(--accent) 22%, var(--fill-1));
  }

  .an-l2 {
    background: color-mix(in srgb, var(--accent) 45%, var(--fill-1));
  }

  .an-l3 {
    background: color-mix(in srgb, var(--accent) 70%, var(--fill-1));
    color: var(--on-accent);
  }

  .an-l4 {
    background: var(--accent);
    color: var(--on-accent);
  }

  .an-thin {
    outline: 1px dashed var(--text-muted);
    outline-offset: -1px;
    opacity: 0.75;
  }

  .an-barcell {
    width: 30%;
    min-width: 80px;
    vertical-align: middle;
  }

  .an-barfill {
    display: block;
    height: 8px;
    border-radius: 4px;
    background: var(--accent);
  }

  .an-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .an-row {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .an-row-text {
    overflow-wrap: anywhere;
  }
</style>
