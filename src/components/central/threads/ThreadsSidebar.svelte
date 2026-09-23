<script lang="ts">
  // Sidebar: projects → threads with status in priority order (approval,
  // input, working, failed, done, idle), pills (plan ready, PR, worktree,
  // terminal, harness), pin / snooze / archive with an undo toast, search,
  // inline rename and keyboard navigation. Rows have fixed heights so status
  // changes never shift the list.
  import { t } from "$lib/i18n";
  import { threadsStore } from "$lib/central/threads/threads-store.svelte";
  import type { ThreadRow } from "$lib/central/threads/types";
  import { threadsUi } from "$lib/central/threads-ui/ui-state.svelte";
  import { indicators } from "$lib/central/threads-ui/indicators.svelte";
  import { groupThreads, isImported, isExternalHarness, sidebarStatus, snoozePresets, type SidebarStatus } from "$lib/central/threads-ui/status";
  import { formatAge, formatUntil, middleTruncate, basename } from "$lib/central/threads-ui/format";
  import { driverName, driverTint } from "$lib/central/threads-ui/drivers";
  import Icon from "./Icon.svelte";
  import Elapsed from "./Elapsed.svelte";

  let {
    selectedId,
    onselect,
    onnewthread,
    onnewproject,
  }: {
    selectedId: string | null;
    onselect: (id: string) => void;
    onnewthread: (projectId: string | null) => void;
    onnewproject: () => void;
  } = $props();

  let query = $state("");
  let now = $state(Date.now());
  let renaming = $state<string | null>(null);
  let renameValue = $state("");
  let snoozeFor = $state<string | null>(null);
  let listEl = $state<HTMLElement | null>(null);

  // Minute tick for ages and snooze countdowns; a snooze wake re-sorts at its second.
  $effect(() => {
    const tick = setInterval(() => (now = Date.now()), 60_000);
    return () => clearInterval(tick);
  });
  $effect(() => {
    const next = threadsStore.threads
      .map((x) => (x.snoozedUntil ? Date.parse(x.snoozedUntil) : NaN))
      .filter((v) => v > now)
      .sort((a, b) => a - b)[0];
    if (!next) return;
    const timer = setTimeout(() => (now = Date.now()), Math.min(2 ** 31 - 1, next - Date.now() + 50));
    return () => clearTimeout(timer);
  });

  let groups = $derived(
    groupThreads(threadsStore.projects, threadsStore.threads, { query: query.trim(), showArchived: threadsUi.sidebar.showArchived, now }),
  );
  let instances = $derived(new Map(threadsStore.instances.map((i) => [i.instance.id, i])));
  let archivedCount = $derived(threadsStore.threads.filter((x) => x.archivedAt).length);

  const STATUS_ICON = {
    approval: "shield-warning",
    input: "chat-circle-dots",
    working: "circle-notch",
    failed: "warning-circle",
    done: "check-circle",
    idle: "check",
  } as const;

  function statusLabel(s: SidebarStatus): string {
    return $t(`llm.central.threads.status.${s}`) as string;
  }

  async function run(cmd: Parameters<typeof threadsStore.dispatch>[0]) {
    try {
      await threadsStore.dispatch(cmd);
    } catch (e) {
      threadsStore.error = String(e);
    }
  }

  async function togglePin(th: ThreadRow) {
    const pinned = !!th.pinnedAt;
    await run({ type: pinned ? "thread.unpin" : "thread.pin", threadId: th.threadId });
    threadsUi.showUndo($t(pinned ? "llm.central.threads.toast.unpinned" : "llm.central.threads.toast.pinned", { title: th.title }) as string, () =>
      run({ type: pinned ? "thread.pin" : "thread.unpin", threadId: th.threadId }),
    );
  }

  async function snooze(th: ThreadRow, until: Date) {
    snoozeFor = null;
    await run({ type: "thread.snooze", threadId: th.threadId, until: until.toISOString() });
    threadsUi.showUndo($t("llm.central.threads.toast.snoozed", { title: th.title }) as string, () => run({ type: "thread.unsnooze", threadId: th.threadId }));
  }

  async function wake(th: ThreadRow) {
    await run({ type: "thread.unsnooze", threadId: th.threadId });
  }

  async function archive(th: ThreadRow) {
    await run({ type: "thread.archive", threadId: th.threadId });
    threadsUi.showUndo($t("llm.central.threads.toast.archived", { title: th.title }) as string, () => run({ type: "thread.unarchive", threadId: th.threadId }));
  }

  async function unarchive(th: ThreadRow) {
    await run({ type: "thread.unarchive", threadId: th.threadId });
  }

  function startRename(th: ThreadRow) {
    renaming = th.threadId;
    renameValue = th.title;
  }

  async function commitRename(th: ThreadRow) {
    const title = renameValue.trim();
    renaming = null;
    if (!title || title === th.title) return;
    await run({ type: "thread.rename", threadId: th.threadId, title });
  }

  function focusInput(node: HTMLInputElement) {
    node.focus();
    node.select();
  }

  function onListKey(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Home" && e.key !== "End") return;
    const rows = [...(listEl?.querySelectorAll<HTMLElement>("[data-thread-row]") ?? [])];
    if (!rows.length) return;
    const at = rows.indexOf(document.activeElement as HTMLElement);
    let next = at;
    if (e.key === "ArrowDown") next = at < 0 ? 0 : Math.min(rows.length - 1, at + 1);
    else if (e.key === "ArrowUp") next = at < 0 ? rows.length - 1 : Math.max(0, at - 1);
    else if (e.key === "Home") next = 0;
    else next = rows.length - 1;
    e.preventDefault();
    rows[next]?.focus();
  }

  function onSearchKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      query = "";
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      listEl?.querySelector<HTMLElement>("[data-thread-row]")?.focus();
    } else if (e.key === "Enter") {
      const first = groups.flatMap((g) => g.sections.flatMap((s) => s.threads))[0];
      if (first) onselect(first.threadId);
    }
  }

  function toggleProject(id: string) {
    threadsUi.sidebar.collapsed[id] = !threadsUi.sidebar.collapsed[id];
    threadsUi.saveSidebar();
  }

  function presetLabel(key: string): string {
    return $t(`llm.central.threads.snooze.${key}`) as string;
  }

  function clock(d: Date): string {
    return d.toLocaleString(undefined, { weekday: "short", hour: "2-digit", minute: "2-digit" });
  }
</script>

<aside class="sidebar" aria-label={$t("llm.central.threads.sidebar")}>
  <div class="head">
    <label class="search">
      <Icon name="magnifying-glass" size={14} />
      <input
        type="search"
        placeholder={$t("llm.central.threads.search")}
        aria-label={$t("llm.central.threads.search")}
        bind:value={query}
        onkeydown={onSearchKey}
      />
      {#if query}
        <button type="button" class="clear" aria-label={$t("llm.central.threads.clear_search")} onclick={() => (query = "")}><Icon name="x" size={12} /></button>
      {/if}
    </label>
    <div class="head-actions">
      <button type="button" class="icon-btn" title={$t("llm.central.threads.new_project")} aria-label={$t("llm.central.threads.new_project")} onclick={onnewproject}>
        <Icon name="folder-simple" size={16} />
      </button>
      <button
        type="button"
        class="icon-btn primary"
        title={`${$t("llm.central.threads.new_thread")} (⌘N)`}
        aria-label={$t("llm.central.threads.new_thread")}
        onclick={() => onnewthread(null)}
      >
        <Icon name="note-pencil" size={16} />
      </button>
    </div>
  </div>

  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <nav class="list" bind:this={listEl} onkeydown={onListKey} aria-label={$t("llm.central.threads.list")}>
    {#if !threadsStore.ready && !threadsStore.error}
      {#each [0, 1, 2, 3] as i (i)}<div class="skeleton" aria-hidden="true"></div>{/each}
    {:else if !threadsStore.projects.length && !threadsStore.threads.length}
      <div class="empty">
        <p>{$t("llm.central.threads.empty_projects")}</p>
        <button type="button" class="button primary" onclick={onnewproject}>{$t("llm.central.threads.new_project")}</button>
      </div>
    {:else if query && !groups.length}
      <p class="empty muted">{$t("llm.central.threads.no_results")}</p>
    {/if}

    {#each groups as g (g.projectId)}
      {@const collapsed = !!threadsUi.sidebar.collapsed[g.projectId] && !query}
      <section class="project">
        <div class="project-head">
          <button type="button" class="project-toggle" aria-expanded={!collapsed} onclick={() => toggleProject(g.projectId)}>
            <span class="chev" class:open={!collapsed}><Icon name="caret-right" size={10} /></span>
            <span class="tile" aria-hidden="true"><Icon name="folder-simple" size={11} /></span>
            <span class="project-name" title={g.project?.workspaceRoot ?? ""}>{g.project?.title ?? basename(g.projectId)}</span>
            <span class="count">{g.count}</span>
          </button>
          <button
            type="button"
            class="icon-btn small"
            title={$t("llm.central.threads.new_in_project", { project: g.project?.title ?? "" })}
            aria-label={$t("llm.central.threads.new_in_project", { project: g.project?.title ?? "" })}
            onclick={() => onnewthread(g.projectId)}
          ><Icon name="plus" size={12} /></button>
        </div>

        {#if !collapsed}
          {#each g.sections as sec (sec.key)}
            {#if sec.threads.length}
              {#if sec.key === "snoozed"}
                <button
                  type="button"
                  class="section-head snoozed"
                  aria-expanded={threadsUi.sidebar.snoozedOpen}
                  onclick={() => {
                    threadsUi.sidebar.snoozedOpen = !threadsUi.sidebar.snoozedOpen;
                    threadsUi.saveSidebar();
                  }}
                >
                  <span class="chev" class:open={threadsUi.sidebar.snoozedOpen}><Icon name="caret-right" size={9} /></span>
                  {$t("llm.central.threads.section.snoozed", { count: sec.threads.length })}
                </button>
              {:else if sec.key === "pinned"}
                <div class="section-head">{$t("llm.central.threads.section.pinned")}</div>
              {:else if sec.key === "archived"}
                <div class="section-head">{$t("llm.central.threads.section.archived", { count: sec.threads.length })}</div>
              {/if}
              {#if sec.key !== "snoozed" || threadsUi.sidebar.snoozedOpen || sec.threads.some((x) => x.threadId === selectedId)}
                {#each sec.threads as th (th.threadId)}
                  {#if sec.key !== "snoozed" || threadsUi.sidebar.snoozedOpen || th.threadId === selectedId}
                    {@render row(th, sec.key)}
                  {/if}
                {/each}
              {/if}
            {/if}
          {/each}
          {#if !g.sections.some((s) => s.threads.length)}
            <p class="none">{$t("llm.central.threads.no_threads_in")}</p>
          {/if}
        {/if}
      </section>
    {/each}
  </nav>

  <div class="foot">
    <button
      type="button"
      class="foot-btn"
      aria-pressed={threadsUi.sidebar.showArchived}
      onclick={() => {
        threadsUi.sidebar.showArchived = !threadsUi.sidebar.showArchived;
        threadsUi.saveSidebar();
      }}
    >
      <Icon name="archive-box" size={13} />
      {threadsUi.sidebar.showArchived ? $t("llm.central.threads.hide_archived") : $t("llm.central.threads.show_archived", { count: archivedCount })}
    </button>
  </div>
</aside>

{#snippet row(th: ThreadRow, section: string)}
  {@const status = sidebarStatus(th)}
  {@const selected = th.threadId === selectedId}
  {@const inst = instances.get(th.instanceId)}
  {@const term = indicators.terminals[th.threadId]}
  {@const pr = indicators.prs[th.threadId]}
  {@const planReady = th.interactionMode === "plan" && !th.activeTurnId && th.turnCount > 0 && status === "idle"}
  {@const hasDraft = threadsUi.hasDraft(th.threadId)}
  <div
    class="row"
    class:selected
    class:recede={status === "idle" && !selected}
    class:slim={section === "snoozed" || section === "archived"}
    data-status={status}
  >
    <button
      type="button"
      class="row-main"
      data-thread-row
      aria-current={selected ? "true" : undefined}
      aria-label={`${th.title} — ${statusLabel(status)}`}
      onclick={() => onselect(th.threadId)}
      ondblclick={() => startRename(th)}
    >
      <span class="line1">
        {#if hasDraft}<span class="draft" title={$t("llm.central.threads.has_draft")}><Icon name="pencil-simple" size={11} /></span>{/if}
        {#if renaming === th.threadId}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="rename"
            bind:value={renameValue}
            use:focusInput
            onclick={(e) => e.stopPropagation()}
            onkeydown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter" && !e.isComposing) commitRename(th);
              else if (e.key === "Escape") renaming = null;
            }}
            onblur={() => commitRename(th)}
            aria-label={$t("llm.central.threads.rename")}
          />
        {:else}
          <span class="title">{th.title || $t("llm.central.threads.untitled")}</span>
        {/if}
      </span>
      {#if section !== "snoozed" && section !== "archived"}
        <span class="line2">
          <span class="harness" style:--tint={driverTint(th.driver, inst)} title={driverName(th.driver, inst)}>
            <span class="dot"></span>{driverName(th.driver, inst)}
          </span>
          {#if th.worktreePath}
            <span class="pill" title={$t("llm.central.threads.pill.worktree", { path: th.worktreePath, branch: th.branch ?? "" })}>
              <Icon name="git-branch" size={11} />{middleTruncate(th.branch ?? basename(th.worktreePath), 18)}
            </span>
          {/if}
          {#if term}
            <span class="pill term" class:running={term === "running"} title={$t(term === "running" ? "llm.central.threads.pill.terminal_running" : "llm.central.threads.pill.terminal")}>
              <Icon name="terminal-window" size={11} />
            </span>
          {/if}
          {#if pr}
            <span class="pill pr" data-state={pr.draft ? "draft" : pr.state} title={pr.title}>
              <Icon name="git-pull-request" size={11} />#{pr.number}
            </span>
          {/if}
          {#if planReady}<span class="pill plan"><Icon name="list-checks" size={11} />{$t("llm.central.threads.pill.plan_ready")}</span>{/if}
          {#if isImported(th)}<span class="pill ext" title={$t("llm.central.threads.pill.imported_hint")}>{$t("llm.central.threads.pill.imported")}</span>
          {:else if isExternalHarness(th)}<span class="pill ext" title={$t("llm.central.threads.pill.external_hint")}>{$t("llm.central.threads.pill.external")}</span>{/if}
        </span>
      {/if}
    </button>

    <span class="status-slot">
      <span class="status" data-status={status}>
        {#if section === "snoozed"}
          <span class="snooze-count"><Icon name="moon" size={11} />{formatUntil(th.snoozedUntil, now)}</span>
        {:else if status === "working"}
          <span class="spin"><Icon name={STATUS_ICON.working} size={12} /></span>
          <Elapsed since={th.latestUserMessageAt ?? th.updatedAt} />
        {:else if status !== "idle"}
          <Icon name={STATUS_ICON[status]} size={12} />
          <span class="status-text">{statusLabel(status)}</span>
        {:else}
          <span class="age">{formatAge(th.latestUserMessageAt ?? th.updatedAt, now)}</span>
        {/if}
        {#if th.pinnedAt && section !== "pinned"}<Icon name="push-pin" size={11} />{/if}
      </span>
      <span class="actions">
        {#if section === "archived"}
          <button type="button" class="act" title={$t("llm.central.threads.unarchive")} aria-label={$t("llm.central.threads.unarchive")} onclick={() => unarchive(th)}><Icon name="tray" size={13} /></button>
        {:else}
          <button type="button" class="act" title={th.pinnedAt ? $t("llm.central.threads.unpin") : $t("llm.central.threads.pin")} aria-label={th.pinnedAt ? $t("llm.central.threads.unpin") : $t("llm.central.threads.pin")} onclick={() => togglePin(th)}>
            <Icon name={th.pinnedAt ? "push-pin-slash" : "push-pin"} size={13} />
          </button>
          {#if section === "snoozed"}
            <button type="button" class="act" title={$t("llm.central.threads.wake")} aria-label={$t("llm.central.threads.wake")} onclick={() => wake(th)}><Icon name="alarm" size={13} /></button>
          {:else if status !== "approval" && status !== "input" && !th.activeTurnId}
            <button type="button" class="act" title={$t("llm.central.threads.snooze_title")} aria-label={$t("llm.central.threads.snooze_title")} aria-expanded={snoozeFor === th.threadId} onclick={() => (snoozeFor = snoozeFor === th.threadId ? null : th.threadId)}>
              <Icon name="moon" size={13} />
            </button>
          {/if}
          <button type="button" class="act" title={$t("llm.central.threads.archive")} aria-label={$t("llm.central.threads.archive")} onclick={() => archive(th)}><Icon name="archive-box" size={13} /></button>
        {/if}
      </span>
    </span>

    {#if snoozeFor === th.threadId}
      <div class="snooze-menu" role="menu" aria-label={$t("llm.central.threads.snooze_title")}>
        {#each snoozePresets() as p (p.key)}
          <button type="button" role="menuitem" onclick={() => snooze(th, p.until)}>
            <span>{presetLabel(p.key)}</span><span class="when">{clock(p.until)}</span>
          </button>
        {/each}
        <button type="button" role="menuitem" class="cancel" onclick={() => (snoozeFor = null)}>{$t("llm.central.threads.cancel")}</button>
      </div>
    {/if}
  </div>
{/snippet}

<style>
  .sidebar { display: flex; flex-direction: column; min-height: 0; height: 100%; width: 100%; background: var(--sidebar-bg, transparent); border-right: 1px solid var(--separator); position: relative; }
  .head { display: flex; gap: 6px; padding: 10px 10px 8px; align-items: center; }
  .search { flex: 1; min-width: 0; display: flex; align-items: center; gap: 6px; padding: 0 8px; height: 30px; border-radius: 8px; background: var(--fill-1); color: var(--text-muted); border: 1px solid transparent; }
  .search:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
  .search input { flex: 1; min-width: 0; border: 0; background: transparent; color: var(--text); font-size: 13px; outline: none; }
  .search input::-webkit-search-cancel-button { display: none; }
  .clear { border: 0; background: transparent; color: var(--text-muted); cursor: pointer; display: inline-flex; padding: 2px; }
  .head-actions { display: flex; gap: 2px; }
  .icon-btn { width: 30px; height: 30px; display: inline-flex; align-items: center; justify-content: center; border-radius: 8px; border: 0; background: transparent; color: var(--text-muted); cursor: pointer; }
  .icon-btn:hover { background: var(--fill-2); color: var(--text); }
  .icon-btn.primary { color: var(--accent-hi); }
  .icon-btn.small { width: 22px; height: 22px; border-radius: 6px; opacity: 0; }
  .project-head:hover .icon-btn.small, .icon-btn.small:focus-visible { opacity: 1; }
  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 0 6px 12px; overscroll-behavior: contain; }
  .skeleton { height: 48px; margin: 6px 4px; border-radius: 10px; background: var(--fill-1); animation: pulse 1.4s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.5; } }
  .empty { padding: 24px 12px; text-align: center; color: var(--text-muted); font-size: 13px; display: grid; gap: 12px; justify-items: center; }
  .muted { color: var(--text-muted); }
  .project { margin-top: 6px; }
  .project-head { display: flex; align-items: center; gap: 2px; padding-right: 4px; }
  .project-toggle { flex: 1; min-width: 0; display: flex; align-items: center; gap: 6px; border: 0; background: transparent; padding: 6px 6px; border-radius: 8px; color: var(--text); font-size: 12px; font-weight: 600; cursor: pointer; text-align: left; }
  .project-toggle:hover { background: var(--fill-1); }
  .chev { display: inline-flex; color: var(--text-dim, var(--text-muted)); transition: transform 150ms var(--ease-out, ease); }
  .chev.open { transform: rotate(90deg); }
  .tile { width: 18px; height: 18px; border-radius: 5px; display: inline-flex; align-items: center; justify-content: center; color: #fff; background: linear-gradient(180deg, #5aa9ff, #1e6fe8); box-shadow: inset 0 0 0 0.5px rgba(255,255,255,.25); flex-shrink: 0; }
  .project-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { font-size: 11px; color: var(--text-muted); font-weight: 500; font-variant-numeric: tabular-nums; }
  .section-head { display: flex; align-items: center; gap: 4px; margin: 8px 8px 2px; font-size: 11px; font-weight: 600; letter-spacing: 0.02em; color: var(--text-muted); text-transform: uppercase; border: 0; background: transparent; padding: 0; cursor: default; }
  button.section-head { cursor: pointer; }
  .section-head.snoozed { color: var(--blue, var(--accent)); }
  .none { margin: 2px 12px 6px; font-size: 12px; color: var(--text-muted); }

  .row { position: relative; display: flex; align-items: stretch; border-radius: 10px; margin: 1px 0; height: 50px; }
  .row.slim { height: 34px; }
  .row:hover, .row:focus-within { background: var(--fill-1); }
  .row.selected { background: var(--accent-soft); }
  .row-main { flex: 1; min-width: 0; display: flex; flex-direction: column; justify-content: center; gap: 3px; padding: 4px 4px 4px 10px; border: 0; background: transparent; color: inherit; text-align: left; cursor: pointer; border-radius: 10px; }
  .row-main:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .line1 { display: flex; align-items: center; gap: 5px; min-width: 0; }
  .title { font-size: 13px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
  .row[data-status="approval"] .title, .row[data-status="input"] .title, .row[data-status="done"] .title, .row[data-status="failed"] .title { font-weight: 650; }
  .row.recede .title { color: var(--text-muted); }
  .draft { color: var(--orange, #f59e0b); display: inline-flex; }
  .rename { flex: 1; min-width: 0; font-size: 13px; padding: 2px 4px; border-radius: 5px; border: 1px solid var(--accent); background: var(--input-bg, var(--surface)); color: var(--text); }
  .line2 { display: flex; align-items: center; gap: 4px; min-width: 0; overflow: hidden; height: 16px; }
  .harness { display: inline-flex; align-items: center; gap: 4px; font-size: 11px; color: var(--text-muted); white-space: nowrap; flex-shrink: 0; }
  .harness .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--tint); box-shadow: 0 0 0 1px color-mix(in srgb, var(--tint) 30%, transparent); }
  .pill { display: inline-flex; align-items: center; gap: 3px; font-size: 10.5px; line-height: 15px; padding: 0 5px; border-radius: 5px; background: var(--fill-2); color: var(--text-muted); white-space: nowrap; flex-shrink: 0; }
  .pill.term.running { color: var(--teal, #14b8a6); animation: pulse 1.6s ease-in-out infinite; }
  .pill.pr[data-state="open"] { color: var(--green, #22c55e); }
  .pill.pr[data-state="merged"] { color: var(--purple, #a855f7); }
  .pill.pr[data-state="closed"] { color: var(--red, #ef4444); }
  .pill.plan { color: var(--purple, #a855f7); background: color-mix(in srgb, var(--purple, #a855f7) 14%, transparent); }
  .pill.ext { font-weight: 600; letter-spacing: 0.02em; }

  .status-slot { position: relative; display: flex; align-items: center; justify-content: flex-end; min-width: 64px; padding-right: 8px; }
  .status { display: inline-flex; align-items: center; gap: 4px; font-size: 11px; color: var(--text-muted); white-space: nowrap; transition: opacity 120ms; }
  .status[data-status="approval"] { color: var(--orange, #f59e0b); font-weight: 600; }
  .status[data-status="input"] { color: var(--purple, #8b5cf6); font-weight: 600; }
  .status[data-status="working"] { color: var(--blue, #3b82f6); }
  .status[data-status="failed"] { color: var(--red, #ef4444); font-weight: 600; }
  .status[data-status="done"] { color: var(--green, #22c55e); font-weight: 600; }
  .status-text { max-width: 70px; overflow: hidden; text-overflow: ellipsis; }
  .snooze-count { display: inline-flex; gap: 3px; align-items: center; color: var(--blue, var(--accent)); }
  .spin { display: inline-flex; animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .actions { position: absolute; right: 4px; top: 50%; transform: translateY(-50%); display: flex; gap: 1px; opacity: 0; pointer-events: none; transition: opacity 120ms; background: inherit; }
  .row:hover .actions, .row:focus-within .actions { opacity: 1; pointer-events: auto; }
  .row:hover .status, .row:focus-within .status { opacity: 0; }
  .act { width: 24px; height: 24px; display: inline-flex; align-items: center; justify-content: center; border: 0; border-radius: 6px; background: var(--surface, transparent); color: var(--text-muted); cursor: pointer; }
  .act:hover { background: var(--fill-3, var(--fill-2)); color: var(--text); }
  .snooze-menu { position: absolute; right: 6px; top: calc(100% - 2px); z-index: 20; min-width: 210px; padding: 4px; border-radius: 10px; background: var(--popup-bg, var(--surface)); border: 1px solid var(--separator); box-shadow: 0 10px 30px color-mix(in srgb, #000 22%, transparent); display: grid; }
  .snooze-menu button { display: flex; justify-content: space-between; gap: 12px; border: 0; background: transparent; color: var(--text); font-size: 12.5px; padding: 6px 8px; border-radius: 6px; cursor: pointer; text-align: left; }
  .snooze-menu button:hover, .snooze-menu button:focus-visible { background: var(--accent-soft); }
  .snooze-menu .when { color: var(--text-muted); font-family: var(--font-mono); font-size: 11px; }
  .snooze-menu .cancel { color: var(--text-muted); }
  .foot { padding: 6px 8px; border-top: 1px solid var(--separator); }
  .foot-btn { display: inline-flex; align-items: center; gap: 6px; border: 0; background: transparent; color: var(--text-muted); font-size: 12px; padding: 5px 8px; border-radius: 6px; cursor: pointer; }
  .foot-btn:hover { background: var(--fill-1); color: var(--text); }
  @media (prefers-reduced-motion: reduce) { .spin, .pill.term.running, .skeleton { animation: none; } }
</style>
