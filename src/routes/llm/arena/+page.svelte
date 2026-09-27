<script lang="ts">
  /**
   * Central › Arena (T9): o mesmo prompt em N ferramentas, cada uma na sua
   * thread e worktree, comparadas lado a lado (estado ao vivo, tempo,
   * tokens/custo, testes e o diff); votos, vencedor, "aplicar vencedor" e
   * limpeza das perdedoras; histórico e placar por driver/modelo.
   * Deep links: `?arena=<id>`, `?thread=<id>&plan=<planId>` (plano aprovado →
   * nova arena), `?view=score`. Ao vivo por `central://arena` e
   * `threads://event`; o relógio só anda enquanto alguma coluna roda.
   */
  import { onDestroy, onMount, untrack } from "svelte";
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { threadsStore } from "$lib/central/threads/threads-store.svelte";
  import {
    arenaCleanup,
    arenaDelete,
    arenaError,
    arenaGet,
    arenaList,
    arenaPick,
    arenaScoreboard,
    arenaStop,
    arenaVerify,
    arenaVote,
    highlights,
    isActive,
    onArena,
    type ArenaEntry,
    type ArenaView,
    type ScoreRow,
  } from "$lib/central/arena";
  import ArenaSetup from "$components/central/arena/ArenaSetup.svelte";
  import ArenaHistory from "$components/central/arena/ArenaHistory.svelte";
  import EntryColumn from "$components/central/arena/EntryColumn.svelte";
  import ApplyDialog from "$components/central/arena/ApplyDialog.svelte";
  import Scoreboard from "$components/central/arena/Scoreboard.svelte";
  import Modal from "$components/central/threads/Modal.svelte";
  import Icon from "$components/central/threads/Icon.svelte";

  type View = "new" | "arena" | "score";

  let arenas = $state<ArenaView[]>([]);
  let selectedId = $state<string | null>(null);
  let view = $state<View>("new");
  let score = $state<ScoreRow[]>([]);
  let error = $state<string | null>(null);
  let now = $state(Date.now());
  let showDiffs = $state(true);
  let promptOpen = $state(false);
  let applyFor = $state<string | null>(null);
  let verifyOpen = $state(false);
  let verifyCmd = $state("");
  let cleanupOpen = $state(false);
  let cleanupAll = $state(false);
  let cleanupBranches = $state(false);
  let deleteOpen = $state(false);
  let busy = $state(false);
  let notice = $state<string | null>(null);

  let planThread = $state<string | null>(null);
  let planId = $state<string | null>(null);

  let selected = $derived(arenas.find((a) => a.arenaId === selectedId) ?? null);
  let tags = $derived(selected ? highlights(selected.entries) : {});
  let anyActive = $derived(!!selected && selected.entries.some(isActive));
  let applyEntry = $derived(selected?.entries.find((e) => e.entryId === applyFor) ?? null);
  let winner = $derived(selected?.entries.find((e) => e.winner) ?? null);

  let unlisten: (() => void) | null = null;

  onMount(() => {
    void threadsStore.init();
    void (async () => {
      unlisten = await onArena((p) => {
        if (p.deleted) {
          arenas = arenas.filter((a) => a.arenaId !== p.arenaId);
          if (selectedId === p.arenaId) selectedId = null;
          return;
        }
        if (!p.arena) return;
        const i = arenas.findIndex((a) => a.arenaId === p.arenaId);
        if (i >= 0) arenas[i] = p.arena;
        else arenas = [p.arena, ...arenas];
      });
      await refresh();
      applyQuery();
    })();
  });
  onDestroy(() => unlisten?.());

  async function refresh() {
    try {
      arenas = await arenaList(200);
      error = null;
    } catch (e) {
      error = arenaError(e).message;
    }
  }

  function applyQuery() {
    const q = page.url.searchParams;
    const a = q.get("arena");
    const th = q.get("thread");
    if (q.get("view") === "score") {
      void openScore();
    } else if (th) {
      planThread = th;
      planId = q.get("plan");
      view = "new";
    } else if (a) {
      void select(a);
    } else if (arenas.length && arenas.some((x) => x.status === "running" || x.status === "review")) {
      void select(arenas.find((x) => x.status === "running" || x.status === "review")!.arenaId);
    }
  }

  async function select(id: string) {
    selectedId = id;
    view = "arena";
    promptOpen = false;
    void goto(`/llm/arena?arena=${encodeURIComponent(id)}`, { replaceState: true, keepFocus: true, noScroll: true });
    try {
      const a = await arenaGet(id);
      const i = arenas.findIndex((x) => x.arenaId === id);
      if (i >= 0) arenas[i] = a;
      else arenas = [a, ...arenas];
    } catch (e) {
      error = arenaError(e).message;
    }
  }

  function startNew() {
    planThread = null;
    planId = null;
    view = "new";
    selectedId = null;
    void goto("/llm/arena", { replaceState: true, keepFocus: true, noScroll: true });
  }

  async function openScore() {
    view = "score";
    selectedId = null;
    try {
      score = await arenaScoreboard();
    } catch (e) {
      error = arenaError(e).message;
    }
  }

  function created(a: ArenaView) {
    arenas = [a, ...arenas.filter((x) => x.arenaId !== a.arenaId)];
    void select(a.arenaId);
  }

  // Relógio só enquanto alguma coluna roda.
  $effect(() => {
    if (!anyActive) return;
    const id = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(id);
  });

  function put(a: ArenaView) {
    const i = arenas.findIndex((x) => x.arenaId === a.arenaId);
    if (i >= 0) arenas[i] = a;
  }

  async function run<T>(f: () => Promise<T>): Promise<T | undefined> {
    busy = true;
    error = null;
    try {
      return await f();
    } catch (e) {
      error = arenaError(e).message;
      return undefined;
    } finally {
      busy = false;
    }
  }

  const vote = (e: ArenaEntry, d: 1 | -1) => void run(async () => put(await arenaVote(e.arenaId, e.entryId, d)));
  const pick = (e: ArenaEntry) => void run(async () => put(await arenaPick(e.arenaId, e.winner ? null : e.entryId)));
  const verifyOne = (e: ArenaEntry) => void run(async () => put(await arenaVerify(e.arenaId, e.entryId)));

  function openVerify() {
    verifyCmd = selected?.verifyCommand ?? "";
    verifyOpen = true;
  }

  async function verifyAll() {
    if (!selected) return;
    const id = selected.arenaId;
    verifyOpen = false;
    await run(async () => put(await arenaVerify(id, null, verifyCmd)));
  }

  async function stopAll() {
    if (!selected) return;
    const id = selected.arenaId;
    await run(async () => put(await arenaStop(id)));
  }

  async function cleanup() {
    if (!selected) return;
    const id = selected.arenaId;
    cleanupOpen = false;
    const r = await run(() => arenaCleanup(id, cleanupAll ? "all" : "losers", cleanupBranches));
    if (r) {
      put(r.arena);
      notice = $t("llm.central.arena.apply.cleaned", { count: r.archived.length }) as string;
      if (r.branchesDeleted.length) notice += ` · ${$t("llm.central.arena.branches_deleted", { count: r.branchesDeleted.length })}`;
      if (r.errors.length) notice += ` · ${r.errors.join("; ")}`;
    }
  }

  async function remove() {
    if (!selected) return;
    const id = selected.arenaId;
    deleteOpen = false;
    const ok = await run(async () => {
      await arenaDelete(id);
      return true;
    });
    if (ok) {
      arenas = arenas.filter((a) => a.arenaId !== id);
      startNew();
    }
  }

  function applied(a: ArenaView) {
    put(a);
  }

  $effect(() => {
    void selectedId;
    untrack(() => (notice = null));
  });

  function when(iso: string | null): string {
    return iso ? new Date(iso).toLocaleString() : "—";
  }
</script>

<svelte:head><title>{$t("llm.tab.arena")}</title></svelte:head>

<div class="arena">
  <aside class="side" aria-label={$t("llm.central.arena.history.title")}>
    <div class="side-head">
      <button type="button" class="button new" class:active={view === "new"} onclick={startNew}>
        <Icon name="plus" size={14} /> {$t("llm.central.arena.new")}
      </button>
      <button type="button" class="icon-btn" aria-pressed={view === "score"} title={$t("llm.central.arena.score.title")} onclick={openScore}>
        <Icon name="list-checks" size={16} />
      </button>
    </div>
    <div class="side-label">{$t("llm.central.arena.history.title")}</div>
    <div class="side-list">
      <ArenaHistory {arenas} selected={selectedId} onselect={select} />
    </div>
  </aside>

  <main class="main">
    {#if error}
      <div class="banner" role="alert">
        <Icon name="warning-circle" size={14} />
        <span class="btext">{error}</span>
        <button type="button" class="icon-btn small" aria-label={$t("llm.central.arena.close")} onclick={() => (error = null)}><Icon name="x" size={12} /></button>
      </div>
    {/if}

    {#if view === "new"}
      <div class="scroll">
        {#key `${planThread}:${planId}`}
          <ArenaSetup planThreadId={planThread} {planId} oncreated={created} oncancel={arenas.length ? () => select(arenas[0].arenaId) : undefined} />
        {/key}
      </div>
    {:else if view === "score"}
      <div class="scroll pad">
        <h2 class="h2">{$t("llm.central.arena.score.title")}</h2>
        <p class="lede">{$t("llm.central.arena.score.lede")}</p>
        <Scoreboard rows={score} />
      </div>
    {:else if selected}
      <header class="board-head">
        <div class="titles">
          <h1 class="title" title={selected.title}>{selected.title}</h1>
          <div class="meta">
            <span class="state st-{selected.status}">{$t(`llm.central.arena.arena_status.${selected.status}`)}</span>
            <span><Icon name="git-branch" size={12} /> <span class="mono">{selected.baseBranch ?? selected.baseCommit.slice(0, 7)}</span></span>
            {#if selected.verifyCommand}<span><Icon name="terminal-window" size={12} /> <span class="mono">{selected.verifyCommand}</span></span>{/if}
            <span class="muted">{when(selected.createdAt)}</span>
            {#if selected.status === "applied"}
              <span class="ok"><Icon name="check-circle" size={12} /> {$t("llm.central.arena.applied_to", { branch: selected.appliedBranch ?? "", commit: (selected.appliedCommit ?? "").slice(0, 7) })}</span>
            {/if}
          </div>
        </div>
        <div class="grow"></div>
        <button type="button" class="ghost" onclick={() => (promptOpen = !promptOpen)} aria-expanded={promptOpen}>
          <Icon name="chat-text" size={13} /> {$t("llm.central.arena.prompt")}
        </button>
        <label class="ghost toggle"><input type="checkbox" bind:checked={showDiffs} /> {$t("llm.central.arena.show_diffs")}</label>
        {#if anyActive}
          <button type="button" class="button small" onclick={stopAll} disabled={busy}><Icon name="stop" size={13} /> {$t("llm.central.arena.stop")}</button>
        {/if}
        <button type="button" class="button small" onclick={openVerify} disabled={busy}><Icon name="list-checks" size={13} /> {$t("llm.central.arena.verify_all")}</button>
        {#if winner && selected.status !== "applied"}
          <button type="button" class="button small active" onclick={() => (applyFor = winner.entryId)}><Icon name="git-branch" size={13} /> {$t("llm.central.arena.apply_winner")}</button>
        {/if}
        <button type="button" class="icon-btn" title={$t("llm.central.arena.cleanup")} onclick={() => { cleanupAll = !winner; cleanupOpen = true; }}><Icon name="archive-box" size={16} /></button>
        <button type="button" class="icon-btn" title={$t("llm.central.arena.delete")} onclick={() => (deleteOpen = true)}><Icon name="x-circle" size={16} /></button>
      </header>
      {#if promptOpen}
        <pre class="prompt">{selected.prompt}</pre>
      {/if}
      {#if notice}
        <p class="notice">{notice}</p>
      {/if}
      <div class="board" style:--cols={selected.entries.length}>
        {#each selected.entries as e (e.entryId)}
          <EntryColumn
            arena={selected}
            entry={e}
            {now}
            tags={tags[e.entryId] ?? []}
            showDiff={showDiffs}
            diffHeight="calc(100vh - 380px)"
            onvote={vote}
            onpick={pick}
            onverify={verifyOne}
            onapply={(x) => (applyFor = x.entryId)}
          />
        {/each}
      </div>
    {:else}
      <div class="hero">
        <Icon name="arrows-split" size={40} />
        <h1>{$t("llm.tab.arena")}</h1>
        <p>{$t("llm.central.arena.setup.lede")}</p>
        <button type="button" class="button active" onclick={startNew}>{$t("llm.central.arena.new")}</button>
      </div>
    {/if}
  </main>
</div>

{#if selected}
  <ApplyDialog open={!!applyEntry} arena={selected} entry={applyEntry} onclose={() => (applyFor = null)} ondone={applied} />
{/if}

<Modal open={verifyOpen} title={$t("llm.central.arena.verify_all") as string} onclose={() => (verifyOpen = false)} width={480}>
  <label class="field">
    <span>{$t("llm.central.arena.setup.verify")}</span>
    <input type="text" class="mono" bind:value={verifyCmd} placeholder="npm test" />
    <small class="muted">{$t("llm.central.arena.verify_hint")}</small>
  </label>
  {#snippet footer()}
    <button type="button" class="button" onclick={() => (verifyOpen = false)}>{$t("llm.central.arena.cancel")}</button>
    <button type="button" class="button active" onclick={verifyAll} disabled={!verifyCmd.trim()}>{$t("llm.central.arena.run")}</button>
  {/snippet}
</Modal>

<Modal open={cleanupOpen} title={$t("llm.central.arena.cleanup") as string} onclose={() => (cleanupOpen = false)} width={480}>
  <div class="stack">
    <p class="muted">{$t("llm.central.arena.cleanup_lede")}</p>
    <label class="check"><input type="radio" bind:group={cleanupAll} value={false} disabled={!winner} /> {$t("llm.central.arena.cleanup_losers")}</label>
    <label class="check"><input type="radio" bind:group={cleanupAll} value={true} /> {$t("llm.central.arena.cleanup_all")}</label>
    <label class="check"><input type="checkbox" bind:checked={cleanupBranches} /> {$t("llm.central.arena.apply.delete_branches")}</label>
  </div>
  {#snippet footer()}
    <button type="button" class="button" onclick={() => (cleanupOpen = false)}>{$t("llm.central.arena.cancel")}</button>
    <button type="button" class="button active" onclick={cleanup}>{$t("llm.central.arena.cleanup")}</button>
  {/snippet}
</Modal>

<Modal open={deleteOpen} title={$t("llm.central.arena.delete") as string} onclose={() => (deleteOpen = false)} width={440}>
  <p class="muted">{$t("llm.central.arena.delete_lede")}</p>
  {#snippet footer()}
    <button type="button" class="button" onclick={() => (deleteOpen = false)}>{$t("llm.central.arena.cancel")}</button>
    <button type="button" class="button active" onclick={remove}>{$t("llm.central.arena.delete")}</button>
  {/snippet}
</Modal>

<style>
  .arena { flex: 1; min-height: 0; display: grid; grid-template-columns: 260px minmax(0, 1fr); overflow: hidden; }
  .side { min-height: 0; display: flex; flex-direction: column; border-right: 1px solid var(--separator); }
  .side-head { display: flex; align-items: center; gap: 6px; padding: 10px; }
  .new { flex: 1; display: inline-flex; align-items: center; justify-content: center; gap: 6px; }
  .side-label { font-size: var(--text-xs); font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: var(--track-caps); padding: 4px 14px; }
  .side-list { flex: 1; min-height: 0; overflow-y: auto; }
  .main { min-width: 0; min-height: 0; display: flex; flex-direction: column; position: relative; }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; }
  .pad { padding: 20px 24px; }
  .h2 { margin: 0; font-size: var(--text-xl); font-weight: 700; }
  .lede { color: var(--text-muted); margin: 4px 0 14px; font-size: var(--text-base); }
  .board-head { display: flex; align-items: center; gap: 8px; padding: 10px 14px; border-bottom: 1px solid var(--separator); flex-wrap: wrap; }
  .titles { min-width: 0; flex: 0 1 auto; display: flex; flex-direction: column; gap: 3px; }
  .title { margin: 0; font-size: var(--text-md); font-weight: 700; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 520px; }
  .meta { display: flex; align-items: center; gap: 10px; font-size: var(--text-xs); color: var(--text-muted); flex-wrap: wrap; }
  .meta > span { display: inline-flex; align-items: center; gap: 4px; }
  .state { font-weight: 600; padding: 1px 7px; border-radius: 999px; background: var(--fill-1); }
  .st-running { color: var(--accent-text, var(--accent)); }
  .st-applied, .ok { color: var(--success); }
  .st-discarded { color: var(--text-muted); }
  .mono { font-family: var(--font-mono); }
  .muted { color: var(--text-muted); font-size: var(--text-sm); }
  .grow { flex: 1; }
  .ghost { display: inline-flex; align-items: center; gap: 4px; border: 0; background: transparent; color: var(--text-muted); font-size: var(--text-sm); padding: 4px 7px; border-radius: 6px; cursor: pointer; }
  .ghost:hover { background: var(--fill-2); color: var(--text); }
  .toggle input { margin: 0; }
  .button.small { display: inline-flex; align-items: center; gap: 5px; padding: 4px 10px; font-size: var(--text-sm); }
  .icon-btn { width: 30px; height: 30px; display: inline-flex; align-items: center; justify-content: center; border-radius: 8px; border: 0; background: transparent; color: var(--text-muted); cursor: pointer; flex-shrink: 0; }
  .icon-btn:hover, .icon-btn[aria-pressed="true"] { background: var(--fill-2); color: var(--text); }
  .icon-btn.small { width: 22px; height: 22px; }
  .prompt { margin: 0; padding: 10px 14px; max-height: 200px; overflow: auto; white-space: pre-wrap; font-family: var(--font-body); font-size: var(--text-base); border-bottom: 1px solid var(--separator); background: var(--fill-1); }
  .notice { margin: 0; padding: 6px 14px; font-size: var(--text-sm); color: var(--text-muted); border-bottom: 1px solid var(--separator); overflow-wrap: anywhere; }
  .board { flex: 1; min-height: 0; overflow: auto; display: grid; grid-auto-flow: column; grid-auto-columns: minmax(360px, 1fr); gap: 10px; padding: 12px; align-items: start; }
  .banner { display: flex; align-items: flex-start; gap: 8px; margin: 8px 12px 0; padding: 8px 10px; border-radius: 12px; background: color-mix(in srgb, var(--error) 10%, var(--surface)); color: var(--error); border: 1px solid color-mix(in srgb, var(--error) 30%, transparent); font-size: var(--text-sm); }
  .btext { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .hero { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; padding: 24px; text-align: center; color: var(--text-muted); }
  .hero h1 { margin: 0; font-size: var(--text-xl); color: var(--text); }
  .hero p { margin: 0; max-width: 460px; }
  .field { display: flex; flex-direction: column; gap: 5px; }
  .field > span { font-size: var(--text-sm); font-weight: 600; }
  .field input { font: inherit; font-size: var(--text-base); color: var(--text); background: var(--input-bg, var(--surface)); border: 1px solid var(--input-border, var(--separator)); border-radius: var(--radius-md); padding: 6px 9px; }
  .stack { display: flex; flex-direction: column; gap: 8px; }
  .stack p { margin: 0; }
  .check { display: flex; align-items: center; gap: 6px; font-size: var(--text-base); }
  @media (max-width: 820px) {
    .arena { grid-template-columns: minmax(0, 1fr); }
    .side { display: none; }
  }
</style>
