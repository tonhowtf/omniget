<script lang="ts">
  /**
   * Visualizador de uma sessão de qualquer ferramenta.
   *
   * Paginação reversa: abre nas mensagens mais novas e carrega as antigas ao
   * rolar para cima (a posição de leitura fica parada). Busca dentro da
   * sessão com anterior/próximo e salto (carrega páginas até o trecho). Tool
   * calls pareadas com o resultado, recolhíveis; "mostrar tools" fica salvo.
   * Enquanto a tela está visível e a sessão está viva, as mensagens novas
   * chegam sozinhas; em segundo plano nada roda.
   */
  import { onMount, tick } from "svelte";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import TurnView from "$components/central/sessions/TurnView.svelte";
  import StateBadge from "$components/central/sessions/StateBadge.svelte";
  import AnalysisModal from "$components/central/sessions/AnalysisModal.svelte";
  import ExportDialog from "$components/central/sessions/ExportDialog.svelte";
  import ResumeDialog from "$components/central/sessions/ResumeDialog.svelte";
  import TeamView from "$components/central/sessions/TeamView.svelte";
  import {
    compact,
    dateTime,
    duration,
    errText,
    prefGet,
    prefSet,
    projectName,
    sessionHref,
    sessionsActive,
    sessionsGet,
    sessionsList,
    sessionsSearchIn,
    toMs,
    toolName,
    totalTokens,
    usd,
    type ActiveSession,
    type Hit,
    type SessionMeta,
    type Turn,
  } from "$lib/central/sessions";

  const PAGE_SIZE = 60;
  const LIVE_MS = 5000;

  let tool = $derived(page.params.tool ?? "");
  let id = $derived(page.params.id ?? "");

  let meta = $state<SessionMeta | null>(null);
  let turns = $state<Map<number, Turn>>(new Map());
  let total = $state(0);
  let resumeCommand = $state<string | null>(null);
  let loading = $state(false);
  let error = $state("");
  let live = $state<ActiveSession | null>(null);
  let childCount = $state(0);

  let tab = $state<"chat" | "team">("chat");
  let showTools = $state(prefGet("showTools", "1") === "1");
  let dialog = $state<"" | "analysis" | "export" | "convert" | "resume">("");

  let query = $state("");
  let hits = $state<Hit[]>([]);
  let hitIdx = $state(-1);
  let searched = $state("");
  let searching = $state(false);
  let currentTurn = $state<number | null>(null);

  let scroller: HTMLDivElement | undefined = $state();
  let sentinel: HTMLDivElement | undefined = $state();

  let ordered = $derived([...turns.entries()].sort((a, b) => a[0] - b[0]));
  let oldest = $derived(ordered.length ? ordered[0][0] : total);
  let hasOlder = $derived(oldest > 0);

  function merge(first: number, list: Turn[]) {
    const next = new Map(turns);
    list.forEach((tn, i) => next.set(first + i, tn));
    turns = next;
  }

  async function loadNewest(): Promise<boolean> {
    const p = await sessionsGet(tool, id, 0, PAGE_SIZE);
    const grew = p.total !== total;
    meta = p.meta;
    total = p.total;
    resumeCommand = p.resume_command;
    merge(p.first_index, p.turns);
    return grew;
  }

  let olderJob: Promise<void> | null = null;

  /** Uma carga de página antiga por vez; chamadas concorrentes esperam a mesma. */
  function loadOlder(keepPosition = true): Promise<void> {
    if (olderJob) return olderJob;
    if (!hasOlder) return Promise.resolve();
    olderJob = (async () => {
      loading = true;
      try {
        const pageNo = Math.max(1, Math.floor((total - oldest) / PAGE_SIZE));
        const before = scroller ? scroller.scrollHeight - scroller.scrollTop : 0;
        const p = await sessionsGet(tool, id, pageNo, PAGE_SIZE);
        total = p.total;
        merge(p.first_index, p.turns);
        if (p.turns.length === 0) {
          // Página vazia: o índice encolheu; recomeça do mais novo.
          turns = new Map();
          await loadNewest();
        }
        await tick();
        if (keepPosition && scroller) scroller.scrollTop = scroller.scrollHeight - before;
      } catch (e) {
        error = errText(e);
      } finally {
        loading = false;
        olderJob = null;
      }
    })();
    return olderJob;
  }

  async function open() {
    turns = new Map();
    total = 0;
    meta = null;
    error = "";
    hits = [];
    hitIdx = -1;
    loading = true;
    try {
      await loadNewest();
    } catch (e) {
      error = errText(e);
    } finally {
      loading = false;
    }
    await tick();
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
    sessionsList({ parent: id, tool, include_subagents: true, limit: 1 })
      .then((r) => (childCount = r.total))
      .catch(() => (childCount = 0));
    const qs = page.url.searchParams;
    const q = qs.get("q");
    const tn = qs.get("turn");
    if (q) {
      query = q;
      await runSearch(tn != null ? Number(tn) : null);
    } else if (tn != null) {
      await jumpTo(Number(tn));
    }
  }

  async function jumpTo(turn: number) {
    if (!Number.isFinite(turn)) return;
    tab = "chat";
    let guard = 0;
    while (turn < oldest && hasOlder && !error && guard++ < 400) {
      await loadOlder(false);
    }
    await tick();
    currentTurn = turn;
    const el = scroller?.querySelector<HTMLElement>(`[data-turn="${turn}"]`);
    el?.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }

  async function runSearch(preferTurn: number | null = null) {
    const q = query.trim();
    if (q.length < 2) {
      hits = [];
      hitIdx = -1;
      searched = "";
      return;
    }
    if (q === searched && hits.length) {
      return step(1);
    }
    searching = true;
    try {
      hits = (await sessionsSearchIn(tool, id, q)).filter((h) => h.turn != null);
      searched = q;
      if (hits.length) {
        let i = preferTurn != null ? hits.findIndex((h) => h.turn === preferTurn) : -1;
        if (i < 0) i = hits.length - 1;
        hitIdx = i;
        await jumpTo(hits[i].turn!);
      } else hitIdx = -1;
    } catch (e) {
      error = errText(e);
    } finally {
      searching = false;
    }
  }

  async function step(dir: number) {
    if (!hits.length) return;
    hitIdx = (hitIdx + dir + hits.length) % hits.length;
    await jumpTo(hits[hitIdx].turn!);
  }

  function onSearchKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      if (query.trim() === searched && hits.length) void step(e.shiftKey ? -1 : 1);
      else void runSearch();
    } else if (e.key === "Escape") {
      query = "";
      hits = [];
      hitIdx = -1;
      searched = "";
      currentTurn = null;
    }
  }

  function nearBottom(): boolean {
    if (!scroller) return false;
    return scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 120;
  }

  async function poll() {
    try {
      const act = await sessionsActive(600);
      const mine = act.find((a) => a.meta.tool === tool && a.meta.id === id) ?? null;
      live = mine;
      if (mine && meta && mine.meta.mtime_ms !== meta.mtime_ms) {
        const stick = nearBottom();
        await loadNewest();
        await tick();
        if (stick && scroller) scroller.scrollTop = scroller.scrollHeight;
      }
    } catch {
      /* estado ao vivo é extra */
    }
  }

  // Reabre ao trocar de sessão (link de subagente, busca).
  let openedKey = "";
  $effect(() => {
    const k = `${tool}/${id}`;
    if (k !== openedKey) {
      openedKey = k;
      void open();
    }
  });

  $effect(() => {
    prefSet("showTools", showTools ? "1" : "0");
  });

  onMount(() => {
    let timer: ReturnType<typeof setInterval> | null = null;
    const start = () => {
      if (timer) return;
      void poll();
      timer = setInterval(poll, LIVE_MS);
    };
    const stop = () => {
      if (timer) clearInterval(timer);
      timer = null;
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
    if (!sentinel || !scroller) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting) && hasOlder && !olderJob && tab === "chat") void loadOlder(true);
      },
      { root: scroller, rootMargin: "300px 0px 0px 0px" },
    );
    io.observe(sentinel);
    return () => io.disconnect();
  });

  let dur = $derived(meta ? Math.max(0, toMs(meta.ended) - toMs(meta.started)) : 0);
</script>

<div class="viewer">
  <header class="head">
    <a class="back" href="/llm/sessions">‹ {$t("llm.central.sessions.list.title")}</a>
    <div class="title-row">
      <h1 class="title">{meta?.title || id}</h1>
      <StateBadge state={live?.state ?? null} pending={live?.pending_tool ?? false} />
    </div>
    {#if meta}
      <p class="meta">
        <span>{toolName(meta.tool)}{meta.account ? ` · ${meta.account}` : ""}</span>
        {#if meta.project_path}<span title={meta.project_path}>{projectName(meta.project_path)}</span>{/if}
        {#if meta.git_branch}<span>{meta.git_branch}</span>{/if}
        <span>{dateTime(meta.started)}{dur ? ` · ${duration(dur)}` : ""}</span>
        <span>{total} {$t("llm.central.sessions.turns_unit")}</span>
        <span>{compact(totalTokens(meta.usage))} tokens</span>
        <span>{usd(meta.cost_usd)}</span>
        {#if meta.models.length}<span>{meta.models.join(", ")}</span>{/if}
        {#if meta.parent_session}<a href={sessionHref(meta.tool, meta.parent_session)}>{$t("llm.central.sessions.open_parent")}</a>{/if}
      </p>
    {/if}
    <div class="actions">
      <button type="button" class="btn btn-primary btn-sm" onclick={() => (dialog = "resume")}>{$t("llm.central.sessions.resume.button")}</button>
      <button type="button" class="btn btn-secondary btn-sm" onclick={() => (dialog = "analysis")}>{$t("llm.central.sessions.analysis.button")}</button>
      <button type="button" class="btn btn-secondary btn-sm" onclick={() => (dialog = "export")}>{$t("llm.central.sessions.export.button")}</button>
      <button type="button" class="btn btn-secondary btn-sm" onclick={() => (dialog = "convert")}>{$t("llm.central.sessions.convert.button")}</button>
    </div>
    <div class="tabs" role="tablist" aria-label={$t("llm.central.sessions.viewer_tabs")}>
      <button type="button" role="tab" id="tab-chat" aria-controls="panel-chat" aria-selected={tab === "chat"} class="segmented-btn" class:active={tab === "chat"} onclick={() => (tab = "chat")}>{$t("llm.central.sessions.tab_chat")}</button>
      <button type="button" role="tab" id="tab-team" aria-controls="panel-team" aria-selected={tab === "team"} class="segmented-btn" class:active={tab === "team"} onclick={() => (tab = "team")}>
        {$t("llm.central.sessions.tab_team")}{childCount ? ` · ${childCount}` : ""}
      </button>
    </div>
  </header>

  {#if error}
    <div class="notice notice-danger"><div class="notice-body"><div class="notice-text">{error}</div></div></div>
  {/if}

  {#if tab === "chat"}
    <div class="toolbar" id="panel-chat" role="tabpanel" aria-labelledby="tab-chat">
      <div class="find">
        <input
          class="input input-search"
          type="search"
          bind:value={query}
          onkeydown={onSearchKey}
          placeholder={$t("llm.central.sessions.viewer.search_placeholder")}
          aria-label={$t("llm.central.sessions.viewer.search_placeholder")}
        />
        <span class="find-count" aria-live="polite">
          {#if searching}…{:else if searched}{hits.length ? `${hitIdx + 1}/${hits.length}` : $t("llm.central.sessions.viewer.no_match")}{/if}
        </span>
        <button type="button" class="btn btn-icon btn-sm" aria-label={$t("llm.central.sessions.viewer.prev")} disabled={!hits.length} onclick={() => step(-1)}>↑</button>
        <button type="button" class="btn btn-icon btn-sm" aria-label={$t("llm.central.sessions.viewer.next")} disabled={!hits.length} onclick={() => step(1)}>↓</button>
      </div>
      <label class="toggle-row">
        <button type="button" class="toggle toggle-sm" role="switch" aria-checked={showTools} aria-label={$t("llm.central.sessions.viewer.show_tools")} onclick={() => (showTools = !showTools)}><span class="toggle-knob"></span></button>
        <span>{$t("llm.central.sessions.viewer.show_tools")}</span>
      </label>
    </div>
    <div class="scroller" bind:this={scroller}>
      <div bind:this={sentinel} class="sentinel">
        {#if hasOlder}
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => loadOlder(true)} disabled={loading}>
            {loading ? $t("llm.central.sessions.loading") : `${$t("llm.central.sessions.viewer.load_older")} (${oldest})`}
          </button>
        {:else if ordered.length}
          <span class="start">{$t("llm.central.sessions.viewer.start")}</span>
        {/if}
      </div>
      {#each ordered as [i, tn] (i)}
        <TurnView turn={tn} index={i} {showTools} mark={searched} current={currentTurn === i} />
      {/each}
      {#if !loading && ordered.length === 0 && !error}
        <p class="empty">{$t("llm.central.sessions.viewer.empty")}</p>
      {/if}
    </div>
  {:else}
    <div class="team-wrap" id="panel-team" role="tabpanel" aria-labelledby="tab-team">
      <TeamView {tool} {id} />
    </div>
  {/if}
</div>

{#if dialog === "analysis"}
  <AnalysisModal {tool} {id} onclose={() => (dialog = "")} />
{:else if dialog === "export" || dialog === "convert"}
  <ExportDialog {tool} {id} mode={dialog} onclose={() => (dialog = "")} />
{:else if dialog === "resume"}
  <ResumeDialog {tool} {id} command={resumeCommand} onclose={() => (dialog = "")} />
{/if}

<style>
  .viewer {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .head {
    padding: var(--space-4) var(--space-5) var(--space-3);
    display: flex;
    flex-direction: column;
    gap: 6px;
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }
  .back {
    font-size: var(--text-xs);
    color: var(--text-dim);
    text-decoration: none;
    align-self: flex-start;
  }
  .back:hover {
    color: var(--text);
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .title {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .meta {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 2px 12px;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .meta a {
    color: var(--accent-hi);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 4px;
  }
  .tabs {
    display: inline-flex;
    align-self: flex-start;
    padding: 2px;
    gap: 1px;
    background: var(--fill-2);
    border-radius: var(--radius-md);
    margin-top: 4px;
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: center;
    gap: 8px 16px;
    padding: 8px var(--space-5);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }
  .find {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1 1 260px;
    max-width: 520px;
  }
  .find input {
    flex: 1;
    min-width: 0;
  }
  .find-count {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    min-width: 3.5em;
    text-align: center;
  }
  .toggle-row {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .scroller {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-3) var(--space-5) var(--space-6);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .scroller > :global(*) {
    flex-shrink: 0;
    max-width: 920px;
    width: 100%;
    margin-inline: auto;
    box-sizing: border-box;
  }
  .sentinel {
    display: flex;
    justify-content: center;
    min-height: 28px;
  }
  .start {
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .empty {
    color: var(--text-dim);
    text-align: center;
  }
  .team-wrap {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-4) var(--space-5);
  }
  @media (max-width: 640px) {
    .head,
    .toolbar,
    .scroller,
    .team-wrap {
      padding-inline: var(--space-3);
    }
  }
</style>
