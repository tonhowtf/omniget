<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { open as pickFile } from "@tauri-apps/plugin-dialog";
  import { t, locale } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SocialIcon from "./SocialIcon.svelte";
  import AudienceFilters from "./AudienceFilters.svelte";
  import AudienceInterests from "./AudienceInterests.svelte";
  import NotifySettings from "./NotifySettings.svelte";
  import { hasExtras } from "./second";
  import { ago, compact, errKey, NET, social, TITLE_KEY, tool, type Platform, type Profile } from "./api";
  import {
    EMPTY_FILTERS,
    LIST_VIEWS,
    SORTS,
    STAR_VIEWS,
    fromRepoSummary,
    fromRepoTrend,
    repoName,
    activeFilters,
    changeBars,
    errorCode,
    filterArgs,
    gaps,
    lastSync,
    linePath,
    pendingSync,
    progress,
    type Filters,
    type ImportResult,
    type ListResult,
    type Person,
    type Summary,
    type SyncResult,
    type Trend,
    type Tri,
    type ViewId,
  } from "./audience";

  let { platform, me, openProfile }: { platform: Platform; me: Profile; openProfile: (handle: string) => void } = $props();

  const PAGE_CAP: Partial<Record<Platform, number>> = { [NET.shortpost]: 40, [NET.photofeed]: 40, [NET.clipfeed]: 67 };
  const REPO_KEY = "omniget.social.audience.repo";
  const PAGE = 50;
  const CHART_W = 560;
  const LINE_H = 110;
  const BAR_H = 28;
  const OWN_ERRORS: Record<string, string> = {
    BUSY: "busy",
    RESUME_FAILED: "resume_failed",
    NOTHING_TO_EXPORT: "nothing_to_export",
    FILE_EXISTS: "file",
    WRITE_FAILED: "file",
    STORE_UNAVAILABLE: "store",
    STORE_FAILED: "store",
    EXPORT_NOT_FOUND: "export_invalid",
    EXPORT_INVALID: "export_invalid",
    EXPORT_TOO_LARGE: "export_invalid",
    ACCOUNT_UNKNOWN: "export_invalid",
  };
  const FLAG_FILTERS = ["verified", "private", "has_avatar", "has_bio", "has_link"] as const;

  let label = $derived($t(TITLE_KEY[platform]) as string);
  let summary = $state<Summary | null>(null);
  let view = $state<ViewId | "trend">("not_following_back");
  let list = $state<ListResult | null>(null);
  let items = $state<Person[]>([]);
  let trend = $state<Trend | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let filters = $state<Filters>({ ...EMPTY_FILTERS });
  let showFilters = $state(false);
  let named = $state<string | null>(null);
  let syncing = $state(false);
  let syncResult = $state<SyncResult | null>(null);
  let syncError = $state<string | null>(null);
  let acting = $state<Record<string, boolean>>({});
  let exporting = $state(false);
  let importing = $state(false);
  let checking = $state(false);
  let seq = 0;
  let poll: ReturnType<typeof setInterval> | undefined;
  let typing: ReturnType<typeof setTimeout> | undefined;
  let alive = true;

  let repo = $state<string | null>(null);
  let repoDraft = $state("");
  let repoInvalid = $state(false);
  let chips = $derived(repo ? STAR_VIEWS : LIST_VIEWS);
  let scope = $derived(repo ? { repo } : {});

  let lists = $derived(summary?.lists ?? []);
  let synced = $derived(lastSync(summary?.synced_at));
  let resume = $derived(pendingSync(lists));
  let missing = $derived(repo ? [] : gaps(summary?.completeness));
  let followersFromExport = $derived(!repo && summary?.completeness?.followers?.source === "export");
  let reading = $derived(progress(lists.filter((l) => l.state === "partial" || l.state === "pending" || syncing)));
  let filterCount = $derived(activeFilters(filters));
  let userKey = $derived(platform === NET.shortpost ? "handle" : "username");
  let followersLine = $derived(trend ? linePath(trend.days.map((d) => d.followers), CHART_W, LINE_H) : "");
  let followingLine = $derived(trend ? linePath(trend.days.map((d) => d.following), CHART_W, LINE_H) : "");
  let bars = $derived(trend ? changeBars(trend.days, CHART_W, BAR_H) : []);
  let retentionPeak = $derived(trend ? Math.max(1, trend.retention.since_before_history, ...trend.retention.buckets.map((b) => b.count)) : 1);

  function message(e: unknown): string {
    const own = OWN_ERRORS[errorCode(e)];
    return own ? $t(`social_audience.errors.${own}`) : $t(`social.errors.${errKey(e)}`, { platform: label });
  }

  function audience<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
    return social.call<T>(tool(platform, name), { account: me.handle, ...scope, ...args });
  }

  function when(ts: number | null | undefined): string {
    if (!ts) return "";
    try {
      return new Intl.DateTimeFormat($locale ?? undefined, { dateStyle: "medium", timeStyle: "short" }).format(ts * 1000);
    } catch {
      return new Date(ts * 1000).toLocaleString();
    }
  }

  function dayLabel(day: string): string {
    try {
      return new Intl.DateTimeFormat($locale ?? undefined, { day: "numeric", month: "short", timeZone: "UTC" }).format(new Date(`${day}T00:00:00Z`));
    } catch {
      return day;
    }
  }

  async function loadSummary() {
    try {
      const asked = repo;
      const s = await audience<Summary>("audience", { view: "summary" });
      if (alive && asked === repo) summary = asked ? fromRepoSummary(s) : s;
    } catch (e) {
      if (alive && !summary) error = message(e);
    }
  }

  async function loadList(more = false, check = 0) {
    if (view === "trend") return loadTrend();
    const mine = ++seq;
    const asked = view;
    if (!check) loading = true;
    if (!more && !check) error = null;
    try {
      const offset = more ? items.length : 0;
      const limit = check ? Math.max(items.length, PAGE) : PAGE;
      const r = await audience<ListResult>("audience", { view: asked, ...filterArgs(filters), ...(named ? { filter: named } : {}), limit: Math.min(limit, 200), offset, check_limit: check });
      if (mine !== seq || !alive) return;
      list = r;
      items = more ? [...items, ...r.items] : r.items;
      const unchecked = asked === "unfollowers" ? r.items.some((p) => p.checked === false && !p.current) : asked === "inactive" && (r.unknown ?? 0) > 0;
      if (!check && !more && unchecked) void verify(5);
    } catch (e) {
      if (mine === seq && alive && !check) error = message(e);
    } finally {
      if (mine === seq) loading = false;
    }
  }

  async function verify(count: number) {
    checking = true;
    try {
      await loadList(false, count);
      if (view === "inactive") void loadSummary();
    } finally {
      checking = false;
    }
  }

  async function loadTrend() {
    const mine = ++seq;
    loading = true;
    error = null;
    try {
      const r = await audience<Trend>("audience", { view: "trend" });
      if (mine === seq && alive) trend = repo ? fromRepoTrend(r) : r;
    } catch (e) {
      if (mine === seq && alive) error = message(e);
    } finally {
      if (mine === seq) loading = false;
    }
  }

  function setScope(next: string | null) {
    repo = next;
    view = next ? "stargazers" : "not_following_back";
    summary = null;
    trend = null;
    syncResult = null;
    syncError = null;
    items = [];
    list = null;
    void loadSummary();
    void loadList();
  }

  function openRepo() {
    const name = repoName(repoDraft);
    repoInvalid = !name;
    if (!name) return;
    try {
      localStorage.setItem(REPO_KEY, name);
    } catch {
      repoInvalid = false;
    }
    setScope(name);
  }

  function pick(next: ViewId | "trend") {
    view = next;
    items = [];
    list = null;
    void loadList();
  }

  function onSearch() {
    clearTimeout(typing);
    typing = setTimeout(() => void loadList(), 300);
  }

  function setFlag(key: (typeof FLAG_FILTERS)[number], value: string) {
    filters[key] = value as Tri;
    void loadList();
  }

  function clearFilters() {
    filters = { ...EMPTY_FILTERS, q: filters.q };
    void loadList();
  }

  async function sync(extra: Record<string, unknown> = {}) {
    if (syncing) return;
    syncing = true;
    syncError = null;
    syncResult = null;
    poll = setInterval(() => void loadSummary(), 3000);
    let left = PAGE_CAP[platform] ?? 40;
    let args = extra;
    try {
      for (;;) {
        const r = await social.call<SyncResult>(tool(platform, "audience_sync"), { max_pages: left, ...scope, ...args });
        syncResult = r;
        left -= r.pages;
        args = {};
        if (r.stopped !== "time" || !alive) break;
        if (left <= 0) {
          syncResult = { ...r, stopped: "page_cap" };
          break;
        }
      }
    } catch (e) {
      syncError = message(e);
    } finally {
      clearInterval(poll);
      syncing = false;
      if (alive) {
        await loadSummary();
        void loadList();
      }
    }
  }

  async function follow(p: Person) {
    const undo = !!p.followed_by_me;
    if (undo && p.kept) return;
    acting[p.id] = true;
    try {
      await social.call(tool(platform, "follow"), { [userKey]: p.handle, undo });
      items = items.map((x) => (x.id === p.id ? { ...x, followed_by_me: !undo } : x));
    } catch (e) {
      showToast("error", message(e));
    } finally {
      acting[p.id] = false;
    }
  }

  async function mark(p: Person, which: "keep" | "hidden") {
    const remove = which === "keep" ? p.kept : p.hidden;
    acting[p.id] = true;
    try {
      await audience("audience_mark", { list: which, handles: [p.handle], remove });
      if (which === "keep") {
        items = view === "kept" && remove ? items.filter((x) => x.id !== p.id) : items.map((x) => (x.id === p.id ? { ...x, kept: !remove } : x));
      } else {
        items = items.filter((x) => x.id !== p.id);
        if (list) list = { ...list, total: Math.max(0, list.total - 1) };
      }
      void loadSummary();
    } catch (e) {
      showToast("error", message(e));
    } finally {
      acting[p.id] = false;
    }
  }

  async function exportCsv() {
    if (view === "trend" || exporting) return;
    exporting = true;
    try {
      const r = await audience<{ path: string; rows: number }>("audience_export", { view, ...filterArgs(filters) });
      showToast("success", $t("social_audience.export.done", { rows: r.rows, path: r.path }));
    } catch (e) {
      showToast("error", message(e));
    } finally {
      exporting = false;
    }
  }

  async function importExport() {
    if (importing || syncing) return;
    let path: string | null = null;
    try {
      const picked = await pickFile({ multiple: false, filters: [{ name: "ZIP", extensions: ["zip"] }] });
      path = typeof picked === "string" ? picked : null;
    } catch {
      path = null;
    }
    if (!path) return;
    importing = true;
    try {
      const r = await audience<ImportResult>("audience_import", { path });
      const counts = { following: r.imported.following?.accounts ?? 0, followers: r.imported.followers?.accounts ?? 0, count: r.not_following_back ?? 0 };
      showToast("success", $t(r.not_following_back == null ? "social_audience.import.done_lists" : "social_audience.import.done", counts));
      syncResult = null;
      await loadSummary();
      void loadList();
    } catch (e) {
      showToast("error", message(e));
    } finally {
      importing = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && showFilters) showFilters = false;
  }

  onMount(() => {
    try {
      repoDraft = platform === NET.codehub ? (localStorage.getItem(REPO_KEY) ?? "") : "";
    } catch {
      repoDraft = "";
    }
    void loadSummary();
    void loadList();
  });

  onDestroy(() => {
    alive = false;
    clearInterval(poll);
    clearTimeout(typing);
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="au">
  <div class="sync">
    <div class="sync-text" aria-live="polite">
      {#if syncing}
        <span class="spinner"></span>
        <span>{reading.total != null ? $t("social_audience.sync.running", { done: compact(reading.done, $locale ?? undefined), total: compact(reading.total, $locale ?? undefined) }) : $t("social_audience.sync.running_unknown", { done: compact(reading.done, $locale ?? undefined) })}</span>
      {:else if synced}
        <span title={when(synced)}>{$t("social_audience.sync.last", { time: ago(synced, $locale ?? undefined) })}</span>
      {:else}
        <span>{$t("social_audience.sync.never")}</span>
      {/if}
    </div>
    <div class="sync-actions">
      {#if view !== "trend"}
        <button class="btn btn-ghost btn-sm" disabled={exporting || !items.length} onclick={() => void exportCsv()}>
          <SocialIcon name="download" /> {$t("social_audience.export.button")}
        </button>
      {/if}
      {#if platform === NET.photofeed}
        <button class="btn btn-ghost btn-sm" disabled={importing || syncing} title={$t("social_audience.import.where")} onclick={() => void importExport()}>
          {#if importing}<span class="spinner"></span>{:else}<SocialIcon name="download" />{/if} {importing ? $t("social_audience.import.busy") : $t("social_audience.import.button")}
        </button>
      {/if}
      {#if resume && !syncing}
        <button class="btn btn-ghost btn-sm" onclick={() => void sync({ restart: true })}>{$t("social_audience.sync.restart")}</button>
      {/if}
      <button class="btn btn-primary btn-sm" disabled={syncing} onclick={() => void sync()}>
        <SocialIcon name="refresh" />
        {syncing ? $t("social_audience.sync.busy") : resume ? $t("social_audience.sync.continue") : $t("social_audience.sync.button")}
      </button>
    </div>
  </div>

  {#if platform === NET.codehub}
    <form class="repo-bar" onsubmit={(e) => { e.preventDefault(); openRepo(); }}>
      <label class="repo-field">
        <span>{$t("social_audience.stars.label")}</span>
        <input class="input" bind:value={repoDraft} placeholder={$t("social_audience.stars.placeholder")} autocomplete="off" spellcheck="false" aria-invalid={repoInvalid} disabled={syncing} />
      </label>
      <button class="btn btn-secondary btn-sm" disabled={syncing || !repoDraft.trim()}><SocialIcon name="star" /> {$t("social_audience.stars.open")}</button>
      {#if repo}
        <button type="button" class="btn btn-ghost btn-sm" disabled={syncing} onclick={() => setScope(null)}>{$t("social_audience.stars.close")}</button>
      {/if}
      {#if repoInvalid}
        <span class="repo-note" role="alert">{$t("social_audience.stars.invalid")}</span>
      {:else if repo}
        <span class="repo-note">{$t("social_audience.stars.viewing", { repo })}</span>
      {/if}
    </form>
  {/if}

  {#if syncError}
    <div class="notice notice-warning" role="alert"><div class="notice-body"><div class="notice-text">{syncError}</div></div></div>
  {:else if syncResult?.stopped === "rate_limited"}
    <div class="notice notice-warning" role="status"><div class="notice-body"><div class="notice-text">{$t("social_audience.notice.rate_limited", { platform: label, minutes: Math.ceil((syncResult.retry_after_seconds ?? 900) / 60) })}</div></div></div>
  {:else if syncResult?.stopped === "sweep_wait"}
    <div class="notice notice-info" role="status"><div class="notice-body"><div class="notice-text">{$t("social_audience.notice.sweep_wait", { minutes: Math.max(1, Math.ceil((syncResult.retry_after_seconds ?? 300) / 60)) })}</div></div></div>
  {:else if syncResult?.stopped === "short" || (!syncing && lists.some((l) => l.state === "short"))}
    <div class="notice notice-warning" role="status">
      <div class="notice-body"><div class="notice-text">{$t("social_audience.notice.short", { platform: label })}</div></div>
      <button class="btn btn-ghost btn-sm" onclick={() => void sync({ accept_short: true })}>{$t("social_audience.notice.short_accept")}</button>
    </div>
  {:else if syncResult?.stopped === "page_cap" || (resume && !syncing)}
    <div class="notice notice-info" role="status"><div class="notice-body"><div class="notice-text">{$t("social_audience.notice.page_cap")}</div></div></div>
  {:else if syncResult?.complete && syncResult.changes && !syncResult.first_snapshot}
    <div class="notice notice-success" role="status"><div class="notice-body"><div class="notice-text">{$t(repo ? "social_audience.stars.changes" : "social_audience.notice.changes", { gained: syncResult.changes.gained, lost: syncResult.changes.lost })}</div></div></div>
  {:else if summary && summary.history.snapshots === 0 && !syncing}
    <div class="notice notice-info">
      <div class="notice-body">
        <div class="notice-title">{$t("social_audience.notice.first_title")}</div>
        <div class="notice-text">{$t(repo ? "social_audience.stars.first_body" : "social_audience.notice.first_body", { platform: label })}</div>
      </div>
    </div>
  {:else if summary?.history.too_short && summary.history.started}
    <div class="notice notice-info"><div class="notice-body"><div class="notice-text">{$t("social_audience.notice.history_short", { date: when(summary.history.started) })}</div></div></div>
  {/if}

  {#if (missing.length || followersFromExport) && !syncing}
    <div class="notice notice-warning" role="status">
      <div class="notice-body">
        {#each missing as gap (gap.kind)}
          <div class="notice-text">{$t(`social_audience.notice.incomplete_${gap.kind}`, { fetched: gap.fetched, expected: gap.expected })}</div>
        {/each}
        {#if followersFromExport}
          <div class="notice-text">{$t("social_audience.notice.followers_from_export")}</div>
        {/if}
        {#if platform === NET.photofeed && missing.length}
          <div class="notice-text">{$t("social_audience.import.where")}</div>
        {/if}
      </div>
    </div>
  {/if}

  <NotifySettings {platform} scope="audience" />

  <div class="chips" role="tablist" aria-label={$t("social_audience.views.label")}>
    {#each chips as v (v)}
      <button class="chip" role="tab" aria-selected={view === v} onclick={() => pick(v)}>
        {$t(`social_audience.views.${v}`)}
        {#if summary}<span class="chip-n">{compact(summary.counts[v] ?? 0, $locale ?? undefined)}</span>{/if}
      </button>
    {/each}
    <button class="chip" role="tab" aria-selected={view === "trend"} onclick={() => pick("trend")}><SocialIcon name="chart" size={13} /> {$t("social_audience.views.trend")}</button>
  </div>

  {#if view === "trend"}
    {#if loading && !trend}
      <div class="boot"><span class="spinner"></span></div>
    {:else if error}
      <div class="notice notice-warning" role="alert">
        <div class="notice-body"><div class="notice-text">{error}</div></div>
        <button class="btn btn-ghost btn-sm" onclick={() => void loadTrend()}>{$t("social.common.retry")}</button>
      </div>
    {:else if trend}
      {#if trend.days.length < 2}
        <div class="empty">{$t("social_audience.trend.empty")}</div>
      {:else}
        <figure class="chart">
          <figcaption>
            <span class="key key-followers">{$t(repo ? "social_audience.views.stargazers" : "social_audience.views.followers")}</span>
            {#if !repo}<span class="key key-following">{$t("social_audience.views.following")}</span>{/if}
            <span class="key key-gained">{$t("social_audience.trend.gained")}</span>
            <span class="key key-lost">{$t("social_audience.trend.lost")}</span>
          </figcaption>
          <svg viewBox="0 0 {CHART_W} {LINE_H + BAR_H * 2 + 8}" role="img" aria-label={$t("social_audience.trend.title")}>
            <path class="line-following" d={followingLine} />
            <path class="line-followers" d={followersLine} />
            <line class="axis" x1="0" x2={CHART_W} y1={LINE_H + BAR_H + 4} y2={LINE_H + BAR_H + 4} />
            {#each bars as b, i (i)}
              <rect class="bar-gained" x={b.x} width={b.width} y={LINE_H + BAR_H + 4 - b.up} height={b.up}><title>{dayLabel(trend.days[i].day)}: +{trend.days[i].gained}</title></rect>
              <rect class="bar-lost" x={b.x} width={b.width} y={LINE_H + BAR_H + 4} height={b.down}><title>{dayLabel(trend.days[i].day)}: -{trend.days[i].lost}</title></rect>
            {/each}
          </svg>
          <div class="chart-ends"><span>{dayLabel(trend.days[0].day)}</span><span>{dayLabel(trend.days[trend.days.length - 1].day)}</span></div>
        </figure>
        <table class="days">
          <thead>
            <tr><th></th><th>{$t(repo ? "social_audience.views.stargazers" : "social_audience.views.followers")}</th><th>{$t("social_audience.views.following")}</th><th>{$t("social_audience.trend.ratio")}</th><th>{$t("social_audience.trend.gained")}</th><th>{$t("social_audience.trend.lost")}</th></tr>
          </thead>
          <tbody>
            {#each [...trend.days].reverse().slice(0, 14) as d (d.day)}
              <tr><th>{dayLabel(d.day)}</th><td>{d.followers ?? "–"}</td><td>{d.following ?? "–"}</td><td>{d.ratio ?? "–"}</td><td class="up">{d.gained ? `+${d.gained}` : "–"}</td><td class="down">{d.lost ? `-${d.lost}` : "–"}</td></tr>
            {/each}
          </tbody>
        </table>
      {/if}
      {#if summary && (summary.counts.followers ?? 0) > 0}
        <h3 class="section">{$t("social_audience.trend.retention")}</h3>
        <div class="retention">
          {#each trend.retention.buckets as b (b.bucket)}
            <div class="ret-row"><span>{$t(`social_audience.trend.buckets.${b.bucket}`)}</span><span class="ret-bar"><i style="width: {(b.count / retentionPeak) * 100}%"></i></span><span class="ret-n">{b.count}</span></div>
          {/each}
          <div class="ret-row"><span>{$t("social_audience.trend.before_history", { date: when(trend.retention.history_started) })}</span><span class="ret-bar"><i class="muted" style="width: {(trend.retention.since_before_history / retentionPeak) * 100}%"></i></span><span class="ret-n">{trend.retention.since_before_history}</span></div>
        </div>
      {/if}
      {#if hasExtras(platform)}
        <AudienceInterests {platform} account={me.handle} />
      {/if}
    {/if}
  {:else}
    <p class="hint">{$t(`social_audience.hints.${view}`, { days: 90 })}</p>
    {#if hasExtras(platform)}
      <AudienceFilters {platform} current={filterArgs(filters)} active={named} onpick={(name) => { named = name; void loadList(); }} />
    {/if}
    <div class="tools">
      <div class="search">
        <SocialIcon name="search" size={14} />
        <input class="input" type="search" bind:value={filters.q} oninput={onSearch} placeholder={$t("social_audience.search")} aria-label={$t("social_audience.search")} />
      </div>
      <div class="pop-wrap">
        <button class="btn btn-ghost btn-sm" aria-expanded={showFilters} aria-controls="audience-filters" onclick={() => (showFilters = !showFilters)}>
          <SocialIcon name="filter" /> {$t("social_audience.filters.button")}{#if filterCount}<span class="chip-n">{filterCount}</span>{/if}
        </button>
        {#if showFilters}
          <button class="scrim" aria-label={$t("social_audience.filters.close")} onclick={() => (showFilters = false)}></button>
          <div class="pop" id="audience-filters" role="dialog" aria-label={$t("social_audience.filters.button")}>
            <label class="f-row">
              <span>{$t("social_audience.filters.sort")}</span>
              <select class="input" bind:value={filters.sort} onchange={() => void loadList()}>
                {#each SORTS as s (s)}<option value={s}>{$t(`social_audience.sort.${s}`)}</option>{/each}
              </select>
            </label>
            {#each FLAG_FILTERS as k (k)}
              <label class="f-row">
                <span>{$t(`social_audience.filters.${k}`)}</span>
                <select class="input" value={filters[k]} onchange={(e) => setFlag(k, e.currentTarget.value)}>
                  <option value="">{$t("social_audience.filters.any")}</option>
                  <option value="yes">{$t("social_audience.filters.yes")}</option>
                  <option value="no">{$t("social_audience.filters.no")}</option>
                </select>
              </label>
            {/each}
            <div class="f-row">
              <span>{$t("social_audience.filters.followers")}</span>
              <span class="f-pair">
                <input class="input" inputmode="numeric" bind:value={filters.min_followers} onchange={() => void loadList()} placeholder={$t("social_audience.filters.min")} aria-label="{$t('social_audience.filters.followers')} {$t('social_audience.filters.min')}" />
                <input class="input" inputmode="numeric" bind:value={filters.max_followers} onchange={() => void loadList()} placeholder={$t("social_audience.filters.max")} aria-label="{$t('social_audience.filters.followers')} {$t('social_audience.filters.max')}" />
              </span>
            </div>
            <div class="f-row">
              <span>{$t("social_audience.filters.ratio")}</span>
              <span class="f-pair">
                <input class="input" inputmode="decimal" bind:value={filters.min_ratio} onchange={() => void loadList()} placeholder={$t("social_audience.filters.min")} aria-label="{$t('social_audience.filters.ratio')} {$t('social_audience.filters.min')}" />
                <input class="input" inputmode="decimal" bind:value={filters.max_ratio} onchange={() => void loadList()} placeholder={$t("social_audience.filters.max")} aria-label="{$t('social_audience.filters.ratio')} {$t('social_audience.filters.max')}" />
              </span>
            </div>
            {#if list?.unavailable_fields.length}
              <p class="f-note">{$t("social_audience.filters.unavailable", { platform: label, fields: list.unavailable_fields.map((f) => $t(`social_audience.fields.${f}`)).join(", ") })}</p>
            {/if}
            <button class="btn btn-ghost btn-sm" disabled={!filterCount} onclick={clearFilters}>{$t("social_audience.filters.clear")}</button>
          </div>
        {/if}
      </div>
      {#if list && list.total > 0}<span class="total">{$t("social_audience.list.count", { shown: items.length, total: list.total })}</span>{/if}
    </div>

    {#if view === "inactive" && list && (list.unknown ?? 0) > 0}
      <div class="notice notice-info" role="status">
        <div class="notice-body"><div class="notice-text">{$t("social_audience.list.inactive_unknown", { count: list.unknown })}</div></div>
        <button class="btn btn-ghost btn-sm" disabled={checking} onclick={() => void verify(10)}>
          {#if checking}<span class="spinner"></span>{/if}{$t("social_audience.list.check_more")}
        </button>
      </div>
    {/if}

    {#if error}
      <div class="notice notice-warning" role="alert">
        <div class="notice-body"><div class="notice-text">{error}</div></div>
        <button class="btn btn-ghost btn-sm" onclick={() => void loadList()}>{$t("social.common.retry")}</button>
      </div>
    {/if}

    <ul class="people">
      {#each items as p (p.id + (p.between?.[1] ?? ""))}
        <li class="person" class:gone={p.status === "gone"}>
          <button class="who" onclick={() => openProfile(p.handle)} title={$t("social_audience.row.open")}>
            {#if p.avatar}<img src={p.avatar} alt="" width="40" height="40" referrerpolicy="no-referrer" loading="lazy" />{:else}<span class="avatar-empty">{p.handle.slice(0, 1).toUpperCase()}</span>{/if}
            <span class="who-text">
              <strong>
                {p.name || p.handle}
                {#if p.verified}<span class="verified" title={$t("social_audience.row.verified")}>✓</span>{/if}
              </strong>
              <span class="meta">
                @{p.handle}{#if p.followers != null} · {$t("social.profile.followers", { count: compact(p.followers, $locale ?? undefined) })}{/if}{#if p.following != null} · {$t("social.profile.following", { count: compact(p.following, $locale ?? undefined) })}{/if}
              </span>
              {#if p.between}
                <span class="meta">{$t("social_audience.row.between", { from: when(p.between[0]), to: when(p.between[1]) })}</span>
              {:else if p.activity === "inactive"}
                <span class="meta">{$t("social_audience.row.last_post", { time: ago(p.last_post, $locale ?? undefined) })}</span>
              {:else if p.activity === "no_posts"}
                <span class="meta">{$t("social_audience.row.no_posts")}</span>
              {/if}
            </span>
          </button>
          <span class="badges">
            {#if p.status === "gone"}<span class="tag tag-danger">{$t("social_audience.row.gone")}</span>{/if}
            {#if p.event === "lost" && p.current}<span class="tag tag-success">{$t(repo ? "social_audience.row.starred_again" : "social_audience.row.refollowed")}</span>{/if}
            {#if p.event === "gained" && p.current === false}<span class="tag tag-warning">{$t(repo ? "social_audience.row.unstarred_since" : "social_audience.row.left")}</span>{/if}
            {#if p.private}<span class="tag">{$t("social.profile.private")}</span>{/if}
            {#if p.kept}<span class="tag tag-accent">{$t("social_audience.row.kept")}</span>{/if}
            {#if p.follows_me && view !== "followers" && view !== "mutuals" && view !== "fans"}<span class="tag">{$t("social.profile.follows_you")}</span>{/if}
          </span>
          <span class="actions">
            {#if p.status !== "gone" && p.followed_by_me != null}
              <button
                class="btn btn-sm {p.followed_by_me ? 'btn-secondary' : 'btn-primary'}"
                disabled={!!acting[p.id] || (p.followed_by_me && p.kept)}
                title={p.followed_by_me && p.kept ? $t("social_audience.row.kept_hint") : undefined}
                onclick={() => void follow(p)}
              >
                {p.followed_by_me ? $t("social.profile.unfollow") : $t("social.profile.follow")}
              </button>
            {/if}
            <button class="btn btn-icon btn-sm" aria-pressed={p.kept} disabled={!!acting[p.id]} title={p.kept ? $t("social_audience.row.unkeep") : $t("social_audience.row.keep")} aria-label={p.kept ? $t("social_audience.row.unkeep") : $t("social_audience.row.keep")} onclick={() => void mark(p, "keep")}><SocialIcon name="star" /></button>
            <button class="btn btn-icon btn-sm" aria-pressed={p.hidden} disabled={!!acting[p.id]} title={p.hidden ? $t("social_audience.row.unhide") : $t("social_audience.row.hide")} aria-label={p.hidden ? $t("social_audience.row.unhide") : $t("social_audience.row.hide")} onclick={() => void mark(p, "hidden")}><SocialIcon name="eyeoff" /></button>
          </span>
        </li>
      {/each}
    </ul>

    {#if loading}
      <div class="boot"><span class="spinner"></span></div>
    {:else if !items.length && !error}
      <div class="empty">
        {#if list?.empty_reason === "not_synced"}
          {$t("social_audience.list.not_synced")}
        {:else if list?.empty_reason === "history_too_short"}
          {$t("social_audience.list.history_too_short")}
        {:else}
          {$t("social_audience.list.empty")}
        {/if}
      </div>
    {:else if list?.truncated || (list && items.length < list.total)}
      <button class="btn btn-ghost more" onclick={() => void loadList(true)}>{$t("social.common.more")}</button>
    {/if}
  {/if}
</div>

<style>
  .au {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .sync,
  .tools,
  .sync-actions,
  .sync-text {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .sync {
    justify-content: space-between;
    flex-wrap: wrap;
  }

  .sync-text {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .repo-bar {
    display: flex;
    align-items: flex-end;
    flex-wrap: wrap;
    gap: 8px;
  }

  .repo-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1 1 220px;
    max-width: 360px;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .repo-note {
    flex-basis: 100%;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .notice {
    align-items: center;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    height: var(--control-h);
    padding: 0 var(--space-3);
    border: 0;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-dim);
    font-size: var(--text-xs);
    cursor: pointer;
  }

  .chip[aria-selected="true"] {
    background: var(--accent);
    color: var(--on-accent);
  }

  .chip:focus-visible,
  .who:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 2px;
  }

  .chip-n {
    font-variant-numeric: tabular-nums;
    opacity: 0.75;
  }

  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .search {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    color: var(--text-muted);
  }

  .search :global(svg) {
    position: absolute;
    left: var(--space-2);
    pointer-events: none;
  }

  .search .input {
    width: 100%;
    padding-left: var(--space-6);
  }

  .total {
    color: var(--text-muted);
    font-size: var(--text-xs);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .pop-wrap {
    position: relative;
  }

  .scrim {
    position: fixed;
    inset: 0;
    border: 0;
    background: transparent;
    z-index: 20;
    cursor: default;
  }

  .pop {
    position: absolute;
    right: 0;
    top: calc(100% + var(--space-1));
    z-index: 21;
    width: 280px;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--popup-bg);
    box-shadow:
      inset 0 0 0 var(--hairline) var(--content-border),
      0 12px 32px rgb(from var(--shadow-ink) r g b / var(--elev-alpha-2));
  }

  .f-row {
    display: grid;
    grid-template-columns: 1fr 140px;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .f-row .input {
    width: 100%;
    min-width: 0;
  }

  .f-pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-1);
  }

  .f-note {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-caption);
  }

  .people {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .person {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .person.gone .who {
    opacity: 0.6;
  }

  .who {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    border-radius: var(--radius-sm);
  }

  .who img,
  .avatar-empty {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
    background: var(--fill-2);
    display: grid;
    place-items: center;
  }

  .who-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-size: var(--text-sm);
  }

  .who-text strong,
  .meta {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .meta {
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .verified {
    color: var(--accent);
    margin-left: 2px;
  }

  .badges,
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-shrink: 0;
  }

  .actions :global(.btn-icon[aria-pressed="true"]) {
    color: var(--accent);
  }

  .boot,
  .empty {
    display: flex;
    justify-content: center;
    padding: var(--space-6) 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
    text-align: center;
  }

  .more {
    align-self: center;
  }

  .chart {
    margin: 0;
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .chart svg {
    display: block;
    width: 100%;
    height: auto;
  }

  .chart figcaption,
  .chart-ends {
    display: flex;
    gap: var(--space-3);
    color: var(--text-dim);
    font-size: var(--text-caption);
  }

  .chart-ends {
    justify-content: space-between;
  }

  .key::before {
    content: "";
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 4px;
    border-radius: 2px;
    background: currentColor;
  }

  .key-followers::before {
    background: var(--accent);
  }

  .key-following::before {
    background: var(--text-muted);
  }

  .key-gained::before,
  .bar-gained {
    background: var(--success);
    fill: var(--success);
  }

  .key-lost::before,
  .bar-lost {
    background: var(--danger);
    fill: var(--danger);
  }

  .line-followers,
  .line-following {
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }

  .line-followers {
    stroke: var(--accent);
  }

  .line-following {
    stroke: var(--text-muted);
  }

  .axis {
    stroke: var(--content-border);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }

  .days {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }

  .days th,
  .days td {
    padding: var(--space-1) var(--space-2);
    text-align: right;
    color: var(--text-dim);
    font-weight: 400;
    border-bottom: var(--hairline) solid var(--content-border);
  }

  .days th:first-child {
    text-align: left;
  }

  .days td {
    color: var(--text);
  }

  .days .up {
    color: var(--success);
  }

  .days .down {
    color: var(--danger);
  }

  .section {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-weight: 600;
  }

  .retention {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .ret-row {
    display: grid;
    grid-template-columns: minmax(120px, 220px) 1fr 48px;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .ret-bar {
    height: 8px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    overflow: hidden;
  }

  .ret-bar i {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .ret-bar i.muted {
    background: var(--text-muted);
  }

  .ret-n {
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }

  @media (max-width: 760px) {
    .badges {
      display: none;
    }
  }
</style>
