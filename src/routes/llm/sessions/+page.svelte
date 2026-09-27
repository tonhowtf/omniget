<script lang="ts">
  /**
   * Sessões — lista ao vivo de todas as ferramentas, agrupada por projeto.
   *
   * O estado "trabalhando / esperando você / parada" vem de `sessions_active`
   * e só é consultado enquanto esta tela está visível (a aba escondida ou o
   * app em segundo plano param o relógio). A busca tem dois níveis: digitar
   * filtra títulos na hora; Enter procura no conteúdo de todas as sessões.
   */
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import StateBadge from "$components/central/sessions/StateBadge.svelte";
  import ImportDialog from "$components/central/sessions/ImportDialog.svelte";
  import Snippet from "$components/central/sessions/Snippet.svelte";
  import {
    compact,
    errText,
    prefGet,
    prefSet,
    projectName,
    relTime,
    sessionHref,
    sessionsActive,
    sessionsList,
    sessionsSearch,
    sessionsSources,
    toMs,
    toolName,
    totalTokens,
    usd,
    type ActiveSession,
    type Hit,
    type SearchResult,
    type SessionMeta,
  } from "$lib/central/sessions";

  const PAGE = 200;
  const ACTIVE_MS = 5000;
  const LIST_MS = 30000;

  const qp = page.url.searchParams;
  let tool = $state(qp.get("tool") ?? "");
  let project = $state(qp.get("project") ?? "");
  let from = $state(qp.get("from") ?? "");
  let to = $state(qp.get("to") ?? "");
  let text = $state(qp.get("q") ?? "");
  let includeSubagents = $state(prefGet("subagents", "0") === "1");
  let sort = $state<"recent" | "tokens" | "cost">("recent");

  let sessions = $state<SessionMeta[]>([]);
  let total = $state(0);
  let active = $state<Map<string, ActiveSession>>(new Map());
  let tools = $state<string[]>([]);
  let loading = $state(false);
  let error = $state("");
  let collapsed = $state<Set<string>>(new Set());
  let showImport = $state(false);

  let search = $state<SearchResult | null>(null);
  let searching = $state(false);
  let lastSearch = $state("");
  let now = $state(Date.now());
  let seq = 0;

  const key = (m: { tool: string; id: string }) => `${m.tool}/${m.id}`;

  async function load(append = false) {
    const my = ++seq;
    loading = true;
    error = "";
    try {
      const res = await sessionsList({
        tool,
        project,
        from,
        to,
        include_subagents: includeSubagents,
        sort,
        limit: PAGE,
        offset: append ? sessions.length : 0,
      });
      if (my !== seq) return;
      sessions = append ? [...sessions, ...res.sessions] : res.sessions;
      total = res.total;
      now = Date.now();
    } catch (e) {
      if (my === seq) error = errText(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  async function loadActive() {
    try {
      const list = await sessionsActive(600);
      active = new Map(list.map((a) => [key(a.meta), a]));
      now = Date.now();
    } catch {
      /* estado ao vivo é extra: a lista continua */
    }
  }

  async function runSearch() {
    const q = text.trim();
    if (q.length < 2) {
      search = null;
      lastSearch = "";
      return;
    }
    searching = true;
    try {
      search = await sessionsSearch({ text: q, tool, project, from, to, include_subagents: includeSubagents, limit: 200, per_session: 3 });
      lastSearch = q;
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      searching = false;
    }
  }

  onMount(() => {
    sessionsSources()
      .then((s) => (tools = s.filter((x) => x.sessions > 0).map((x) => x.tool)))
      .catch(() => {});
    if (text.trim().length >= 2) void runSearch();

    let activeTimer: ReturnType<typeof setInterval> | null = null;
    let listTimer: ReturnType<typeof setInterval> | null = null;
    const start = () => {
      if (activeTimer) return;
      void loadActive();
      activeTimer = setInterval(loadActive, ACTIVE_MS);
      listTimer = setInterval(() => {
        if (!loading && sessions.length <= PAGE) void load();
      }, LIST_MS);
    };
    const stop = () => {
      if (activeTimer) clearInterval(activeTimer);
      if (listTimer) clearInterval(listTimer);
      activeTimer = listTimer = null;
    };
    const onVis = () => (document.visibilityState === "visible" ? start() : stop());
    document.addEventListener("visibilitychange", onVis);
    onVis();
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVis);
    };
  });

  $effect(() => {
    void [tool, project, from, to, includeSubagents, sort];
    void load();
  });

  $effect(() => {
    prefSet("subagents", includeSubagents ? "1" : "0");
  });

  let quick = $derived(text.trim().toLowerCase());
  let liveCount = $derived([...active.values()].filter((a) => a.state !== "idle" && (includeSubagents || !a.meta.parent_session)).length);

  let visible = $derived(
    quick && !search
      ? sessions.filter((s) =>
          `${s.title ?? ""} ${s.id} ${s.project_path ?? ""} ${s.git_branch ?? ""}`.toLowerCase().includes(quick),
        )
      : sessions,
  );

  type Group = { path: string; name: string; items: SessionMeta[]; last: number; live: number };

  let groups = $derived.by<Group[]>(() => {
    const map = new Map<string, Group>();
    for (const s of visible) {
      const p = s.project_path ?? "";
      let g = map.get(p);
      if (!g) {
        g = { path: p, name: projectName(p) || $t("llm.central.sessions.no_project"), items: [], last: 0, live: 0 };
        map.set(p, g);
      }
      g.items.push(s);
      g.last = Math.max(g.last, s.mtime_ms || toMs(s.ended));
      const a = active.get(key(s));
      if (a && a.state !== "idle") g.live++;
    }
    const out = [...map.values()];
    if (sort === "recent") out.sort((a, b) => b.live - a.live || b.last - a.last);
    return out;
  });

  /** Resultados agrupados por sessão, na ordem em que chegaram. */
  let hitGroups = $derived.by(() => {
    const map = new Map<string, { tool: string; id: string; title: string; project: string; ended: string | null; hits: Hit[] }>();
    for (const h of search?.hits ?? []) {
      const k = `${h.tool}/${h.session_id}`;
      let g = map.get(k);
      if (!g) {
        g = { tool: h.tool, id: h.session_id, title: h.title ?? h.session_id, project: h.project_path ?? "", ended: h.ended, hits: [] };
        map.set(k, g);
      }
      g.hits.push(h);
    }
    return [...map.values()];
  });

  function toggle(path: string) {
    const next = new Set(collapsed);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    collapsed = next;
  }

  function onSearchKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      void runSearch();
    } else if (e.key === "Escape") {
      text = "";
      search = null;
    }
  }

  function clearAll() {
    tool = project = from = to = text = "";
    search = null;
  }

  function openHit(h: Hit) {
    const href = sessionHref(h.tool, h.session_id, h.turn);
    const q = lastSearch ? `${href.includes("?") ? "&" : "?"}q=${encodeURIComponent(lastSearch)}` : "";
    void goto(href + q);
  }
</script>

<div class="page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.central.sessions.list.title")}</h1>
      <p class="lede">{$t("llm.central.sessions.list.lede")}</p>
    </div>
    <div class="head-actions">
      <button type="button" class="btn btn-secondary" onclick={() => (showImport = true)}>{$t("llm.central.sessions.import.button")}</button>
    </div>
  </header>

  <div class="searchbar">
    <input
      class="input input-search"
      type="search"
      bind:value={text}
      onkeydown={onSearchKey}
      placeholder={$t("llm.central.sessions.list.search_placeholder")}
      aria-label={$t("llm.central.sessions.list.search_placeholder")}
    />
    <button type="button" class="btn btn-primary" onclick={runSearch} disabled={searching || text.trim().length < 2}>
      {searching ? $t("llm.central.sessions.searching") : $t("llm.central.sessions.list.search_content")}
    </button>
  </div>

  <div class="filters" role="group" aria-label={$t("llm.central.sessions.filters")}>
    <select class="input" bind:value={tool} aria-label={$t("llm.central.sessions.tool")}>
      <option value="">{$t("llm.central.sessions.all_tools")}</option>
      {#each tools.includes(tool) || !tool ? tools : [tool, ...tools] as tl (tl)}<option value={tl}>{toolName(tl)}</option>{/each}
    </select>
    <input class="input" type="text" bind:value={project} placeholder={$t("llm.central.sessions.project")} aria-label={$t("llm.central.sessions.project")} />
    <label class="date"><span>{$t("llm.central.sessions.from")}</span><input class="input" type="date" bind:value={from} /></label>
    <label class="date"><span>{$t("llm.central.sessions.to")}</span><input class="input" type="date" bind:value={to} /></label>
    <select class="input" bind:value={sort} aria-label={$t("llm.central.sessions.sort")}>
      <option value="recent">{$t("llm.central.sessions.sort_recent")}</option>
      <option value="tokens">{$t("llm.central.sessions.sort_tokens")}</option>
      <option value="cost">{$t("llm.central.sessions.sort_cost")}</option>
    </select>
    <label class="check"><input type="checkbox" bind:checked={includeSubagents} /> {$t("llm.central.sessions.include_subagents")}</label>
    {#if tool || project || from || to || text}
      <button type="button" class="btn btn-ghost btn-sm" onclick={clearAll}>{$t("llm.central.sessions.clear_filters")}</button>
    {/if}
  </div>

  {#if error}
    <div class="notice notice-danger"><div class="notice-body"><div class="notice-text">{error}</div></div></div>
  {/if}

  {#if search}
    <section class="results" aria-live="polite">
      <div class="results-head">
        <strong>{search.sessions_matched}</strong> {$t("llm.central.sessions.list.sessions_matched")} · {search.hits.length} {$t("llm.central.sessions.list.hits")}
        · {search.sessions_scanned} {$t("llm.central.sessions.list.scanned")}
        {#if search.truncated}<span class="tag">{$t("llm.central.sessions.list.truncated")}</span>{/if}
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => (search = null)}>{$t("llm.central.sessions.list.back_to_list")}</button>
      </div>
      {#if hitGroups.length === 0}
        <p class="empty">{$t("llm.central.sessions.list.no_hits")}</p>
      {/if}
      <ul class="hit-groups">
        {#each hitGroups as g (g.tool + g.id)}
          <li class="surface-card hit-card">
            <a class="hit-title" href={sessionHref(g.tool, g.id)}>{g.title}</a>
            <span class="meta">{toolName(g.tool)} · {projectName(g.project)} · {relTime(toMs(g.ended))}</span>
            <ul class="hits">
              {#each g.hits as h, i (i)}
                <li>
                  <button type="button" class="hit" onclick={() => openHit(h)}>
                    <span class="field">{$t(`llm.central.sessions.field.${h.field}`)}{h.match_count > 1 ? ` ×${h.match_count}` : ""}</span>
                    <Snippet text={h.snippet} start={h.match_start} len={h.match_len} />
                  </button>
                </li>
              {/each}
            </ul>
          </li>
        {/each}
      </ul>
    </section>
  {:else}
    <p class="count" aria-live="polite">
      {visible.length === sessions.length ? `${sessions.length} / ${total}` : `${visible.length} / ${sessions.length}`}
      {$t("llm.central.sessions.sessions_unit")}
      {#if liveCount}· <strong>{liveCount}</strong> {$t("llm.central.sessions.list.live_now")}{/if}
    </p>
    {#if !loading && groups.length === 0}
      <div class="empty-state">
        <div class="empty-state-title">{$t("llm.central.sessions.list.empty_title")}</div>
        <div class="empty-state-hint">{$t("llm.central.sessions.list.empty_hint")}</div>
      </div>
    {/if}
    <div class="groups" class:stale={loading && sessions.length > 0}>
      {#each groups as g (g.path)}
        {@const open = !collapsed.has(g.path)}
        <section class="group-block">
          <button type="button" class="group-head" aria-expanded={open} onclick={() => toggle(g.path)} title={g.path}>
            <span class="chev" class:open aria-hidden="true">›</span>
            <span class="g-name">{g.name}</span>
            {#if g.live}<span class="live-dot" aria-hidden="true"></span>{/if}
            <span class="g-count">{g.items.length}</span>
            <span class="g-path">{g.path}</span>
          </button>
          {#if open}
            <ul class="rows">
              {#each g.items as s (key(s))}
                {@const a = active.get(key(s))}
                <li>
                  <a class="row" href={sessionHref(s.tool, s.id)}>
                    <StateBadge state={a?.state ?? null} pending={a?.pending_tool ?? false} />
                    <span class="r-main">
                      <span class="r-title">{s.title || s.id}</span>
                      <span class="r-sub">
                        {toolName(s.tool)}{s.account ? ` · ${s.account}` : ""}{s.git_branch ? ` · ${s.git_branch}` : ""}{s.parent_session ? ` · ${$t("llm.central.sessions.subagent")}` : ""}
                        · {s.turn_count} {$t("llm.central.sessions.turns_unit")}
                        {#if s.models.length}· {s.models[s.models.length - 1]}{/if}
                      </span>
                    </span>
                    <span class="r-num">
                      <span>{compact(totalTokens(s.usage))}</span>
                      <span class="r-cost">{usd(s.cost_usd)}</span>
                    </span>
                    <span class="r-when">{relTime(s.mtime_ms || toMs(s.ended), now)}</span>
                  </a>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      {/each}
    </div>
    {#if sessions.length < total}
      <button type="button" class="btn btn-secondary more" onclick={() => load(true)} disabled={loading}>
        {$t("llm.central.sessions.load_more")} ({total - sessions.length})
      </button>
    {/if}
  {/if}
</div>

{#if showImport}
  <ImportDialog onclose={() => (showImport = false)} ondone={() => load()} />
{/if}

<style>
  .page {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    box-sizing: border-box;
  }
  .page > :global(*) {
    flex-shrink: 0;
    min-width: 0;
  }
  .page-head {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .page-title {
    margin: 0;
  }
  .lede {
    margin: 4px 0 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    max-width: 62ch;
  }
  .searchbar {
    display: flex;
    gap: 8px;
  }
  .searchbar input {
    flex: 1;
    min-width: 0;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .filters .input {
    height: 28px;
    font-size: var(--text-sm);
    padding-block: 0;
    max-width: 200px;
  }
  .date {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .count {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .groups {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    transition: opacity var(--duration-base) var(--ease-out);
  }
  .groups.stale {
    opacity: 0.7;
  }
  .group-head {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 4px;
    background: none;
    border: none;
    font: inherit;
    color: var(--text);
    cursor: pointer;
    text-align: left;
    border-radius: var(--radius-sm);
  }
  .group-head:focus-visible {
    outline: var(--focus-ring);
  }
  .chev {
    display: inline-block;
    width: 12px;
    color: var(--text-dim);
    transition: transform var(--duration-fast) var(--ease-out);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .g-name {
    font-weight: 600;
    font-size: var(--text-base);
  }
  .g-count {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .g-path {
    font-size: var(--text-caption);
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1;
  }
  .live-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
  }
  .rows {
    list-style: none;
    margin: 0 0 var(--space-2);
    padding: 0;
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 1px var(--content-border);
    overflow: hidden;
  }
  .rows li + li {
    border-top: 1px solid var(--separator);
  }
  .row {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto 72px;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    color: inherit;
    text-decoration: none;
  }
  @media (hover: hover) {
    .row:hover {
      background: var(--fill-1);
    }
  }
  .row:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }
  .r-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .r-title {
    font-size: var(--text-base);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .r-sub {
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .r-num {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .r-cost {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .r-when {
    font-size: var(--text-xs);
    color: var(--text-dim);
    text-align: right;
  }
  .more {
    align-self: center;
  }
  .results {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .results-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .tag {
    font-size: var(--text-caption);
    background: var(--fill-2);
    border-radius: var(--radius-xs);
    padding: 0 4px;
  }
  .hit-groups {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .hit-card {
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .hit-title {
    font-weight: 600;
    color: var(--text);
    text-decoration: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hit-title:hover {
    text-decoration: underline;
  }
  .meta {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .hits {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .hit {
    width: 100%;
    display: grid;
    grid-template-columns: 90px 1fr;
    gap: 8px;
    text-align: left;
    background: none;
    border: none;
    padding: 4px 6px;
    border-radius: var(--radius-sm);
    font: inherit;
    font-size: var(--text-sm);
    color: var(--text-muted);
    cursor: pointer;
  }
  @media (hover: hover) {
    .hit:hover {
      background: var(--fill-1);
    }
  }
  .hit:focus-visible {
    outline: var(--focus-ring);
  }
  .field {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .empty {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  @media (max-width: 640px) {
    .page {
      padding: var(--space-3);
    }
    .row {
      grid-template-columns: auto minmax(0, 1fr) auto;
    }
    .r-when {
      display: none;
    }
    .g-path {
      display: none;
    }
    .hit {
      grid-template-columns: 1fr;
      gap: 0;
    }
  }
</style>
