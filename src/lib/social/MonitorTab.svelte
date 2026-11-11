<script lang="ts">
  import { untrack } from "svelte";
  import { open as shellOpen } from "@tauri-apps/plugin-shell";
  import { t, locale } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import PostCard from "./PostCard.svelte";
  import SocialIcon from "./SocialIcon.svelte";
  import NotifySettings from "./NotifySettings.svelte";
  import { ago, errKey, social, tool, type Platform, type Post, type Profile } from "./api";
  import {
    REDDIT_SORTS,
    REDDIT_TIMES,
    buildOptions,
    monitorQueryKey,
    redditOptions,
    sparkPoints,
    toolError,
    type CheckResult,
    type Monitor,
    type MonitorDetail,
    type MonitorList,
    type ToolError,
  } from "./insights";

  let { platform, me, openProfile }: { platform: Platform; me: Profile; openProfile: (handle: string) => void } = $props();

  const LABEL: Record<string, string> = { x: "X", instagram: "Instagram", tiktok: "TikTok", reddit: "Reddit", pinterest: "Pinterest", github: "GitHub" };
  const RD_BLANK = { sort: "new", time: "", subreddit: "" };
  const INTERVALS = [0, 5, 15, 30, 60];
  const BLANK = { phrase: "", any: "", exclude: "", hashtags: "", from: "", to: "", mentions: "", lang: "", minLikes: "", minReposts: "", excludeReplies: false, excludeReposts: false };

  let list = $state<MonitorList | null>(null);
  let loading = $state(false);
  let error = $state<ToolError | null>(null);
  let selected = $state<number | null>(null);
  let detail = $state<MonitorDetail | null>(null);
  let detailLoading = $state(false);
  let unreadOnly = $state(false);
  let checking = $state(false);
  let lastCheck = $state<CheckResult | null>(null);
  let interval = $state(0);
  let acting = $state<Record<string, boolean>>({});

  let formOpen = $state(false);
  let editing = $state<Monitor | null>(null);
  let name = $state("");
  let kind = $state("search");
  let query = $state("");
  let advanced = $state(false);
  let filters = $state({ ...BLANK });
  let rd = $state({ ...RD_BLANK });
  let canAct = $derived(platform === "x" || platform === "instagram" || platform === "tiktok");
  let redditSearch = $derived(platform === "reddit" && kind === "search");
  let preview = $state<string | null>(null);
  let previewError = $state<string | null>(null);
  let saving = $state(false);
  let removing = $state<{ id: number; hits: number } | null>(null);
  let previewSeq = 0;
  let previewTimer: ReturnType<typeof setTimeout> | undefined;

  let spark = $derived(detail ? sparkPoints(detail.days.map((d) => d.count), 240, 40) : "");

  function intervalKey() {
    return `omniget.social.monitor.interval.${platform}`;
  }

  function readInterval(): number {
    try {
      const n = Number(localStorage.getItem(intervalKey()));
      return INTERVALS.includes(n) ? n : 0;
    } catch {
      return 0;
    }
  }

  function setIntervalMinutes(n: number) {
    interval = n;
    try {
      localStorage.setItem(intervalKey(), String(n));
    } catch {
      return;
    }
  }

  function errorLine(e: ToolError): string {
    const key = errKey(`${e.code}: x`);
    if (key !== "unknown" && key !== "invalid_arguments") return $t(`social.errors.${key}`, { platform: LABEL[platform] }) as string;
    return e.message;
  }

  async function loadList() {
    loading = true;
    try {
      list = await social.call<MonitorList>("social_monitors", { platform });
      error = null;
    } catch (e) {
      error = toolError(e);
    } finally {
      loading = false;
    }
  }

  async function loadDetail(id: number) {
    detailLoading = true;
    try {
      const r = await social.call<MonitorDetail>("social_monitors", { platform, id, unread_only: unreadOnly, limit: 50, days: 30 });
      if (selected === id) detail = r;
      error = null;
    } catch (e) {
      error = toolError(e);
    } finally {
      detailLoading = false;
    }
  }

  function openMonitor(id: number) {
    selected = id;
    detail = null;
    lastCheck = null;
    void loadDetail(id);
  }

  function backToList() {
    selected = null;
    detail = null;
    void loadList();
  }

  async function check(id?: number) {
    if (checking) return;
    checking = true;
    error = null;
    try {
      const args: Record<string, unknown> = { platform };
      if (id != null) args.id = id;
      lastCheck = await social.call<CheckResult>("social_monitor_check", args);
      await loadList();
      if (selected != null) await loadDetail(selected);
    } catch (e) {
      error = toolError(e);
    } finally {
      checking = false;
    }
  }

  $effect(() => {
    void platform;
    untrack(() => {
      selected = null;
      detail = null;
      lastCheck = null;
      formOpen = false;
      interval = readInterval();
      void loadList();
    });
  });

  $effect(() => {
    if (interval <= 0) return;
    const timer = setInterval(() => void check(), interval * 60_000);
    return () => clearInterval(timer);
  });

  $effect(() => () => clearTimeout(previewTimer));

  function startCreate() {
    editing = null;
    name = "";
    query = "";
    kind = list?.kinds[0] ?? "search";
    filters = { ...BLANK };
    rd = { ...RD_BLANK };
    advanced = false;
    preview = null;
    previewError = null;
    formOpen = true;
  }

  function startEdit(m: Monitor) {
    const o = m.input.options ?? {};
    editing = m;
    name = m.name;
    kind = m.kind;
    query = m.input.query ?? "";
    filters = {
      phrase: o.phrase ?? "",
      any: (o.any ?? []).join(", "),
      exclude: (o.exclude ?? []).join(", "),
      hashtags: (o.hashtags ?? []).join(", "),
      from: (o.from ?? []).join(", "),
      to: (o.to ?? []).join(", "),
      mentions: (o.mentions ?? []).join(", "),
      lang: o.lang ?? "",
      minLikes: o.min_likes ? String(o.min_likes) : "",
      minReposts: o.min_reposts ? String(o.min_reposts) : "",
      excludeReplies: !!o.exclude_replies,
      excludeReposts: !!o.exclude_reposts,
    };
    rd = { sort: o.sort ?? "new", time: o.time ?? "", subreddit: o.subreddit ?? "" };
    advanced = platform === "x" && Object.keys(o).length > 0;
    formOpen = true;
    schedulePreview();
  }

  function formArgs(op: string): Record<string, unknown> {
    const args: Record<string, unknown> = { platform, op, kind, query };
    if (platform === "x") args.options = buildOptions(filters);
    if (platform === "reddit") args.options = redditSearch ? redditOptions(rd) : {};
    if (name.trim()) args.name = name.trim();
    if (editing) args.id = editing.id;
    return args;
  }

  function schedulePreview() {
    clearTimeout(previewTimer);
    previewTimer = setTimeout(() => void runPreview(), 300);
  }

  async function runPreview() {
    const mine = ++previewSeq;
    try {
      const r = await social.call<{ query: string }>("social_monitor_save", formArgs("preview"));
      if (mine !== previewSeq) return;
      preview = r.query;
      previewError = null;
    } catch (e) {
      if (mine !== previewSeq) return;
      preview = null;
      previewError = toolError(e).message;
    }
  }

  async function save() {
    if (saving) return;
    saving = true;
    try {
      const r = await social.call<{ monitor: Monitor }>("social_monitor_save", formArgs(editing ? "update" : "create"));
      formOpen = false;
      await loadList();
      openMonitor(r.monitor.id);
    } catch (e) {
      previewError = toolError(e).message;
    } finally {
      saving = false;
    }
  }

  async function askRemove(m: Monitor) {
    try {
      const r = await social.call<{ would_remove: { hits: number } }>("social_monitor_save", { platform, op: "remove", id: m.id });
      removing = { id: m.id, hits: r.would_remove.hits };
    } catch (e) {
      error = toolError(e);
    }
  }

  async function confirmRemove() {
    if (!removing) return;
    try {
      await social.call("social_monitor_save", { platform, op: "remove", id: removing.id, confirm: true });
      removing = null;
      backToList();
    } catch (e) {
      error = toolError(e);
    }
  }

  async function mark(postIds: string[] | null, read: boolean) {
    if (selected == null) return;
    try {
      const args: Record<string, unknown> = { platform, op: "mark_read", id: selected, read };
      if (postIds) args.post_ids = postIds;
      await social.call("social_monitor_save", args);
      await loadDetail(selected);
    } catch (e) {
      error = toolError(e);
    }
  }

  function patchHit(id: string, patch: Partial<Post>) {
    if (!detail) return;
    detail = { ...detail, hits: { ...detail.hits, items: detail.hits.items.map((h) => (h.post.id === id ? { ...h, post: { ...h.post, ...patch } } : h)) } };
  }

  async function act(p: Post, verb: "like" | "repost") {
    if (acting[p.id]) return;
    acting = { ...acting, [p.id]: true };
    const undo = verb === "like" ? !!p.liked : !!p.reposted;
    try {
      await social.call(tool(platform, verb), { post: p.url || p.id, undo });
      patchHit(p.id, verb === "like" ? { liked: !undo } : { reposted: !undo });
    } catch (e) {
      showToast("error", errorLine(toolError(e)));
    } finally {
      acting = { ...acting, [p.id]: false };
    }
  }

  function external(url: string) {
    if (url) void shellOpen(url).catch(() => {});
  }

  function when(ms: number | null): string {
    return ms ? ($t("social_monitor.last_checked", { when: ago(Math.floor(ms / 1000), $locale) }) as string) : ($t("social_monitor.never_checked") as string);
  }

  function title(m: Monitor): string {
    return m.builtin ? ($t(platform === "github" ? "social_monitor.notifications" : "social_monitor.mentions") as string) : m.name;
  }

  function queryLabel(): string {
    return $t(`social_monitor.form.${monitorQueryKey(platform, kind)}`) as string;
  }
</script>

<div class="mo" data-me={me.handle}>
  {#if error}
    <div class="notice notice-warning" role="alert">
      <div class="notice-body">
        <div class="notice-text">{errorLine(error)}</div>
        {#if error.next_step}<div class="notice-text mo-dim">{error.next_step}</div>{/if}
      </div>
      <button class="btn btn-ghost btn-sm" onclick={() => (selected != null ? void loadDetail(selected) : void loadList())}>{$t("social.common.retry")}</button>
    </div>
  {/if}

  {#if formOpen}
    <form class="mo-form" onsubmit={(e) => { e.preventDefault(); void save(); }} oninput={schedulePreview} onchange={schedulePreview}>
      <h3 class="mo-h">{$t(editing ? "social_monitor.form.title_edit" : "social_monitor.form.title_new")}</h3>
      <label class="field">
        <span class="field-label">{$t("social_monitor.form.name")}</span>
        <input class="input" bind:value={name} maxlength="80" placeholder={$t("social_monitor.form.name_placeholder")} />
      </label>
      {#if (list?.kinds.length ?? 0) > 1}
        <div class="segmented mo-kind" role="radiogroup" aria-label={$t("social_monitor.form.kind")}>
          {#each list?.kinds ?? [] as k (k)}
            <button type="button" class="segmented-btn" role="radio" aria-checked={kind === k} onclick={() => { kind = k; schedulePreview(); }}>{$t(`social_monitor.form.kind_${k}`)}</button>
          {/each}
        </div>
      {/if}
      <label class="field">
        <span class="field-label">{queryLabel()}</span>
        <input class="input" bind:value={query} maxlength="500" autocomplete="off" spellcheck="false" />
      </label>
      {#if redditSearch}
        <div class="mo-grid">
          <label class="field">
            <span class="field-label">{$t("social_monitor.form.rd_sort_label")}</span>
            <select class="input" bind:value={rd.sort}>
              {#each REDDIT_SORTS as s (s)}<option value={s}>{$t(`social_monitor.form.rd_sort.${s}`)}</option>{/each}
            </select>
          </label>
          <label class="field">
            <span class="field-label">{$t("social_monitor.form.rd_time_label")}</span>
            <select class="input" bind:value={rd.time}>
              <option value="">{$t("social_monitor.form.rd_time.any")}</option>
              {#each REDDIT_TIMES as s (s)}<option value={s}>{$t(`social_monitor.form.rd_time.${s}`)}</option>{/each}
            </select>
          </label>
          <label class="field"><span class="field-label">{$t("social_monitor.form.rd_subreddit")}</span><input class="input" bind:value={rd.subreddit} maxlength="200" placeholder="r/rust" autocomplete="off" spellcheck="false" /></label>
        </div>
      {/if}
      {#if platform === "x"}
        <button type="button" class="btn btn-ghost btn-sm mo-adv" aria-expanded={advanced} onclick={() => (advanced = !advanced)}><SocialIcon name="filter" /> {$t("social_monitor.form.advanced")}</button>
        {#if advanced}
          <div class="mo-grid">
            <label class="field"><span class="field-label">{$t("social_monitor.form.phrase")}</span><input class="input" bind:value={filters.phrase} /></label>
            <label class="field"><span class="field-label">{$t("social_monitor.form.any")}</span><input class="input" bind:value={filters.any} placeholder={$t("social_monitor.form.list_hint")} /></label>
            <label class="field"><span class="field-label">{$t("social_monitor.form.exclude")}</span><input class="input" bind:value={filters.exclude} placeholder={$t("social_monitor.form.list_hint")} /></label>
            <label class="field"><span class="field-label">{$t("social_monitor.form.hashtags")}</span><input class="input" bind:value={filters.hashtags} placeholder={$t("social_monitor.form.list_hint")} /></label>
            <label class="field"><span class="field-label">{$t("social_monitor.form.from")}</span><input class="input" bind:value={filters.from} placeholder={$t("social_monitor.form.list_hint")} /></label>
            <label class="field"><span class="field-label">{$t("social_monitor.form.to")}</span><input class="input" bind:value={filters.to} placeholder={$t("social_monitor.form.list_hint")} /></label>
            <label class="field"><span class="field-label">{$t("social_monitor.form.mentions")}</span><input class="input" bind:value={filters.mentions} placeholder={$t("social_monitor.form.list_hint")} /></label>
            <label class="field"><span class="field-label">{$t("social_monitor.form.lang")}</span><input class="input" bind:value={filters.lang} maxlength="3" placeholder="pt" /></label>
            <label class="field"><span class="field-label">{$t("social_monitor.form.min_likes")}</span><input class="input" type="number" min="0" bind:value={filters.minLikes} /></label>
            <label class="field"><span class="field-label">{$t("social_monitor.form.min_reposts")}</span><input class="input" type="number" min="0" bind:value={filters.minReposts} /></label>
            <label class="mo-check"><input type="checkbox" class="checkbox" bind:checked={filters.excludeReplies} /> {$t("social_monitor.form.exclude_replies")}</label>
            <label class="mo-check"><input type="checkbox" class="checkbox" bind:checked={filters.excludeReposts} /> {$t("social_monitor.form.exclude_reposts")}</label>
          </div>
        {/if}
      {/if}
      <div class="mo-final" aria-live="polite">
        <span class="field-label">{$t("social_monitor.form.final")}</span>
        {#if preview}
          <code>{preview}</code>
        {:else}
          <span class="mo-dim">{previewError ?? $t("social_monitor.form.final_empty")}</span>
        {/if}
      </div>
      <div class="mo-actions">
        <button class="btn btn-primary btn-sm" disabled={saving || !preview}>{$t("social_monitor.form.save")}</button>
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => (formOpen = false)}>{$t("social_monitor.form.cancel")}</button>
      </div>
    </form>
  {:else if selected == null}
    <div class="mo-bar">
      <button class="btn btn-primary btn-sm" onclick={startCreate}><SocialIcon name="plus" /> {$t("social_monitor.new")}</button>
      <button class="btn btn-secondary btn-sm" disabled={checking || !list?.monitors.length} onclick={() => void check()}>
        {#if checking}<span class="spinner"></span>{:else}<SocialIcon name="refresh" />{/if} {$t("social_monitor.check_all")}
      </button>
      <span class="mo-grow"></span>
      <label class="mo-interval">
        <span>{$t("social_monitor.interval_label")}</span>
        <select class="input" value={interval} onchange={(e) => setIntervalMinutes(Number(e.currentTarget.value))}>
          {#each INTERVALS as n (n)}<option value={n}>{n === 0 ? $t("social_monitor.interval_off") : $t("social_monitor.interval_minutes", { count: n })}</option>{/each}
        </select>
      </label>
    </div>
    <p class="mo-hint">{$t("social_monitor.no_background")}</p>
    <NotifySettings {platform} scope="monitor" monitors={(list?.monitors ?? []).map((m) => ({ id: m.id, name: title(m) }))} />
    {#if checking}<p class="mo-hint" role="status">{$t("social_monitor.checking")}</p>{/if}
    {#if lastCheck && !checking}
      <div class="notice {lastCheck.surging.length ? 'notice-warning' : 'notice-info'}" role="status">
        <div class="notice-body">
          <div class="notice-text">{lastCheck.new > 0 ? $t("social_monitor.result", { count: lastCheck.new }) : $t("social_monitor.result_none")}</div>
          {#if lastCheck.partial_reason}<div class="notice-text mo-dim">{lastCheck.failed?.[0]?.error ?? lastCheck.partial_reason}</div>{/if}
        </div>
      </div>
    {/if}
    {#if list && !list.mentions_inbox.available}
      <p class="mo-hint">{$t(platform === "github" ? "social_monitor.mentions_github" : platform === "reddit" ? "social_monitor.mentions_reddit" : "social_monitor.mentions_unavailable", { platform: LABEL[platform] })}</p>
    {/if}
    {#if loading && !list}
      <div class="mo-state"><span class="spinner"></span></div>
    {:else if list && list.monitors.length === 0}
      <div class="mo-state">{$t("social_monitor.empty_list")}</div>
    {:else if list}
      <ul class="mo-list">
        {#each list.monitors as m (m.id)}
          <li>
            <button class="mo-item" onclick={() => openMonitor(m.id)}>
              <span class="mo-icon"><SocialIcon name={m.builtin ? "at" : "radar"} /></span>
              <span class="mo-body">
                <strong>{title(m)}</strong>
                {#if m.query}<code class="mo-query">{m.query}</code>{/if}
                <span class="mo-dim">{when(m.last_checked)} · {$t("social_monitor.stored", { count: m.total_stored })}</span>
              </span>
              {#if lastCheck?.surging.includes(m.id)}<span class="tag tag-warning">{$t("social_monitor.surge_title")}</span>{/if}
              {#if m.unread > 0}<span class="badge" aria-label={$t("social_monitor.unread", { count: m.unread })}>{m.unread}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  {:else}
    <div class="mo-bar">
      <button class="btn btn-ghost btn-sm" onclick={backToList}><SocialIcon name="back" /> {$t("social_monitor.back")}</button>
      <span class="mo-grow"></span>
      <button class="btn btn-primary btn-sm" disabled={checking} onclick={() => selected != null && void check(selected)}>
        {#if checking}<span class="spinner"></span>{:else}<SocialIcon name="refresh" />{/if} {$t("social_monitor.check_now")}
      </button>
      {#if detail && !detail.monitor.builtin}
        <button class="btn btn-ghost btn-sm" onclick={() => detail && startEdit(detail.monitor)}>{$t("social_monitor.edit")}</button>
        <button class="btn btn-ghost btn-sm" onclick={() => detail && void askRemove(detail.monitor)} aria-label={$t("social_monitor.remove")} title={$t("social_monitor.remove")}><SocialIcon name="trash" /></button>
      {/if}
    </div>
    {#if removing}
      <div class="notice notice-danger" role="alertdialog" aria-label={$t("social_monitor.remove")}>
        <div class="notice-body"><div class="notice-text">{$t("social_monitor.remove_confirm", { count: removing.hits })}</div></div>
        <button class="btn btn-destructive btn-sm" onclick={() => void confirmRemove()}>{$t("social_monitor.remove")}</button>
        <button class="btn btn-ghost btn-sm" onclick={() => (removing = null)}>{$t("social_monitor.form.cancel")}</button>
      </div>
    {/if}
    {#if checking}<p class="mo-hint" role="status">{$t("social_monitor.checking")}</p>{/if}
    {#if !detail}
      <div class="mo-state"><span class="spinner"></span></div>
    {:else}
      <div class="mo-head">
        <div class="mo-body">
          <h3 class="mo-h">{title(detail.monitor)}</h3>
          {#if detail.monitor.query}<code class="mo-query">{detail.monitor.query}</code>{/if}
          <span class="mo-dim">{when(detail.monitor.last_checked)} · {$t("social_monitor.stored", { count: detail.monitor.total_stored })} · {$t("social_monitor.unread", { count: detail.monitor.unread })}</span>
        </div>
        <div class="mo-trend">
          <span class="mo-dim">{$t("social_monitor.trend_title")}</span>
          {#if detail.days.length > 1}
            <svg viewBox="0 0 240 40" width="240" height="40" preserveAspectRatio="none" role="img" aria-label={$t("social_monitor.trend_label", { days: detail.days.length, today: detail.surge.today })}>
              <polyline points={spark} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round" vector-effect="non-scaling-stroke" />
            </svg>
          {:else}
            <span class="mo-dim">{$t("social_monitor.trend_empty")}</span>
          {/if}
        </div>
      </div>
      {#if detail.surge.surge}
        <div class="notice notice-warning" role="status">
          <div class="notice-body">
            <span class="notice-title">{$t("social_monitor.surge_title")}</span>
            <div class="notice-text">{$t("social_monitor.surge_text", { today: detail.surge.today, average: detail.surge.trailing_average ?? 0, days: detail.surge.days_used, multiple: detail.surge.multiple })}</div>
          </div>
        </div>
      {/if}
      {#if lastCheck && !checking}
        <p class="mo-hint" role="status">{lastCheck.new > 0 ? $t("social_monitor.result", { count: lastCheck.new }) : $t("social_monitor.result_none")}</p>
      {/if}
      <div class="mo-bar">
        <h3 class="mo-h">{$t("social_monitor.hits_title")}</h3>
        <span class="mo-grow"></span>
        <label class="mo-check"><input type="checkbox" class="checkbox" bind:checked={unreadOnly} onchange={() => selected != null && void loadDetail(selected)} /> {$t("social_monitor.unread_only")}</label>
        <button class="btn btn-ghost btn-sm" disabled={detail.monitor.unread === 0} onclick={() => void mark(null, true)}><SocialIcon name="check" /> {$t("social_monitor.mark_all")}</button>
      </div>
      {#if detail.hits.items.length === 0}
        <div class="mo-state">
          {#if detail.monitor.last_checked == null}{$t("social_monitor.hits_empty_never")}{:else if unreadOnly}{$t("social_monitor.hits_empty_unread")}{:else}{$t("social_monitor.hits_empty")}{/if}
        </div>
      {:else}
        <div class="mo-hits">
          {#each detail.hits.items as h (h.post.id)}
            <div class="mo-hit" class:mo-read={h.read}>
              <PostCard post={h.post} {platform} busy={!!acting[h.post.id]} onOpen={(p) => external(p.url)} onAuthor={openProfile} onLike={canAct ? (p) => void act(p, "like") : undefined} onRepost={canAct ? (p) => void act(p, "repost") : undefined} onExternal={external} />
              <button class="btn btn-ghost btn-sm mo-mark" onclick={() => void mark([h.post.id], !h.read)}>
                {#if !h.read}<span class="mo-dot" aria-hidden="true"></span>{/if}{$t(h.read ? "social_monitor.mark_unread" : "social_monitor.mark_read")}
              </button>
            </div>
          {/each}
        </div>
        {#if detail.hits.truncated}<p class="mo-hint">{$t("social_monitor.hits_more", { shown: detail.hits.returned, total: detail.hits.total })}</p>{/if}
      {/if}
      {#if detailLoading}<div class="mo-state"><span class="spinner"></span></div>{/if}
    {/if}
  {/if}
</div>

<style>
  .mo {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .mo-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .mo-grow {
    flex: 1;
  }

  .mo-interval {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .mo-interval .input {
    width: auto;
  }

  .mo-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.5;
  }

  .mo-dim {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .mo-state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-6) 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
    text-align: center;
  }

  .mo-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .mo-item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-3);
    border: 0;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }

  .mo-item:hover,
  .mo-item:focus-visible {
    background: var(--fill-1);
  }

  .mo-icon {
    display: inline-flex;
    color: var(--text-dim);
  }

  .mo-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .mo-query,
  .mo-final code {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }

  .mo-h {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
  }

  .mo-head {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .mo-trend {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--accent);
  }

  .mo-trend svg {
    max-width: 100%;
  }

  .mo-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .mo-kind,
  .mo-adv {
    align-self: flex-start;
  }

  .mo-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--space-2) var(--space-3);
  }

  .mo-check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .mo-final {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }

  .mo-final code {
    color: var(--text);
    font-size: var(--text-sm);
  }

  .mo-actions {
    display: flex;
    gap: var(--space-2);
  }

  .mo-hits {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .mo-hit {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .mo-read {
    opacity: 0.65;
  }

  .mo-mark {
    align-self: flex-end;
  }

  .mo-dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    margin-right: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
</style>
