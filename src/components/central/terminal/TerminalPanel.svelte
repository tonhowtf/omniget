<script lang="ts">
  // Terminal drawer/panel: tabs, each tab a group of up to four split panes.
  // The layout (tabs, pane ids) is remembered per `storageKey`; the sessions
  // themselves live in the app process, so closing this panel or the window and
  // coming back reattaches every pane with its history and the program still
  // running.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import {
    newTerminalId,
    ptyClose,
    ptyList,
    type PtyForeground,
    type PtySessionInfo,
    type TerminalSelection,
  } from "$lib/central/pty";
  import TerminalView from "./TerminalView.svelte";

  type Dir = "row" | "column";
  type Tab = { key: string; dir: Dir; panes: string[]; active: string };
  type ViewApi = { selection: () => TerminalSelection | null; focus: () => void; clearScreen: () => void };

  const MAX_SPLITS = 4;

  let {
    storageKey = "default",
    cwd = null,
    command = [],
    env = undefined,
    initialInput = undefined,
    fontSize = 12,
    onAddToChat,
    onClosePanel,
  }: {
    /** Layout scope (a thread id, "drawer", …). */
    storageKey?: string;
    cwd?: string | null;
    /** argv for new terminals; empty = default shell. */
    command?: string[];
    env?: Record<string, string>;
    /** Pre-typed (not submitted) into the first terminal opened. */
    initialInput?: string;
    fontSize?: number;
    onAddToChat?: (sel: TerminalSelection) => void;
    onClosePanel?: () => void;
  } = $props();

  const lsKey = () => `omniget.central.terminal.${storageKey}`;

  let tabs = $state<Tab[]>([]);
  let activeTab = $state<string>("");
  let fg = $state<Record<string, PtyForeground>>({});
  let ended = $state<Record<string, boolean>>({});
  let hasSel = $state<Record<string, boolean>>({});
  let apis = $state<Record<string, ViewApi | undefined>>({});
  /** The pane that receives `initialInput` (only a fresh panel's first one). */
  let firstPane = $state<string | null>(null);

  const current = $derived(tabs.find((x) => x.key === activeTab) ?? tabs[0]);

  function save() {
    try {
      localStorage.setItem(lsKey(), JSON.stringify({ tabs, activeTab }));
    } catch {
      // Layout persistence is optional.
    }
  }

  function newTab(): Tab {
    const id = newTerminalId();
    return { key: id, dir: "row", panes: [id], active: id };
  }

  function addTab() {
    const tab = newTab();
    tabs = [...tabs, tab];
    activeTab = tab.key;
    save();
  }

  function split(dir: Dir) {
    const tab = current;
    if (!tab) return;
    if (tab.panes.length >= MAX_SPLITS) {
      tab.dir = dir;
      save();
      return;
    }
    const id = newTerminalId();
    const at = tab.panes.indexOf(tab.active);
    tab.panes.splice(at + 1, 0, id);
    tab.dir = dir;
    tab.active = id;
    tabs = [...tabs];
    save();
  }

  async function closePane(id: string) {
    await ptyClose(id, true).catch(() => {});
    const tab = tabs.find((x) => x.panes.includes(id));
    if (!tab) return;
    const at = tab.panes.indexOf(id);
    tab.panes = tab.panes.filter((p) => p !== id);
    delete fg[id];
    delete ended[id];
    delete hasSel[id];
    delete apis[id];
    if (tab.panes.length === 0) {
      const idx = tabs.indexOf(tab);
      tabs = tabs.filter((x) => x !== tab);
      if (activeTab === tab.key) activeTab = tabs[Math.max(0, idx - 1)]?.key ?? "";
    } else {
      if (tab.active === id) tab.active = tab.panes[Math.max(0, at - 1)];
      tabs = [...tabs];
    }
    save();
  }

  async function closeTab(tab: Tab) {
    for (const id of [...tab.panes]) await closePane(id);
  }

  /** Replaces an ended session with a fresh one in the same pane. */
  async function restart(id: string) {
    await ptyClose(id, true).catch(() => {});
    const tab = tabs.find((x) => x.panes.includes(id));
    if (!tab) return;
    const fresh = newTerminalId();
    tab.panes = tab.panes.map((p) => (p === id ? fresh : p));
    if (tab.active === id) tab.active = fresh;
    delete ended[id];
    delete fg[id];
    tabs = [...tabs];
    save();
  }

  function addSelection() {
    const tab = current;
    if (!tab || !onAddToChat) return;
    const sel = apis[tab.active]?.selection();
    if (sel && sel.text.trim()) onAddToChat(sel);
  }

  function tabLabel(tab: Tab): string {
    const f = fg[tab.active];
    return f?.label || ($t("llm.central.terminal.tab") as string);
  }

  function runningIn(tab: Tab): PtyForeground | null {
    for (const p of tab.panes) if (fg[p]?.running) return fg[p];
    return null;
  }

  onMount(() => {
    let saved: { tabs?: Tab[]; activeTab?: string } | null = null;
    try {
      saved = JSON.parse(localStorage.getItem(lsKey()) || "null");
    } catch {
      saved = null;
    }
    const valid = (saved?.tabs ?? []).filter(
      (x) => x && Array.isArray(x.panes) && x.panes.length > 0 && typeof x.key === "string",
    );
    if (valid.length) {
      tabs = valid.map((x) => ({
        key: x.key,
        dir: x.dir === "column" ? "column" : "row",
        panes: x.panes.slice(0, MAX_SPLITS),
        active: x.panes.includes(x.active) ? x.active : x.panes[0],
      }));
      activeTab = tabs.some((x) => x.key === saved?.activeTab) ? saved!.activeTab! : tabs[0].key;
    } else {
      addTab();
      firstPane = tabs[0]?.active ?? null;
    }
    // Labels for panes before their views mount.
    void ptyList()
      .then((list: PtySessionInfo[]) => {
        for (const s of list) {
          if (tabs.some((x) => x.panes.includes(s.id))) {
            fg[s.id] = s.foreground;
            if (!s.alive) ended[s.id] = true;
          }
        }
      })
      .catch(() => {});
  });
</script>

<section class="term-panel">
  <header class="bar">
    <div class="tabs" role="tablist">
      {#each tabs as tab (tab.key)}
        {@const run = runningIn(tab)}
        <div class="tab" class:active={tab.key === current?.key} role="tab" aria-selected={tab.key === current?.key}>
          <button class="tab-btn" onclick={() => { activeTab = tab.key; save(); }}>
            {#if run}<span class="dot" aria-hidden="true"></span>{/if}
            <span class="tab-label">{tabLabel(tab)}</span>
            {#if tab.panes.length > 1}<span class="count">{tab.panes.length}</span>{/if}
          </button>
          <button class="tab-x" title={$t("llm.central.terminal.close_tab")} aria-label={$t("llm.central.terminal.close_tab")} onclick={() => closeTab(tab)}>
            <svg viewBox="0 0 12 12" width="9" height="9"><path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
          </button>
        </div>
      {/each}
      <button class="icon" title={$t("llm.central.terminal.new_tab")} aria-label={$t("llm.central.terminal.new_tab")} onclick={addTab}>
        <svg viewBox="0 0 14 14" width="12" height="12"><path d="M7 2v10M2 7h10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
      </button>
    </div>
    <div class="actions">
      {#if current}
        {@const run = fg[current.active]}
        {#if run?.running}
          <span class="badge" title={run.pid ? `pid ${run.pid}` : ""}>
            <span class="dot" aria-hidden="true"></span>{$t("llm.central.terminal.running", { name: run.label })}
          </span>
        {/if}
        {#if onAddToChat}
          <button class="text-btn" disabled={!hasSel[current.active]} onclick={addSelection} title={$t("llm.central.terminal.add_to_chat_hint")}>
            {$t("llm.central.terminal.add_to_chat")}
          </button>
        {/if}
        <button class="icon" disabled={current.panes.length >= MAX_SPLITS && current.dir === "row"} title={$t("llm.central.terminal.split_right")} aria-label={$t("llm.central.terminal.split_right")} onclick={() => split("row")}>
          <svg viewBox="0 0 16 16" width="14" height="14"><rect x="1.5" y="2.5" width="13" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M8 2.5v11" stroke="currentColor" stroke-width="1.4"/></svg>
        </button>
        <button class="icon" disabled={current.panes.length >= MAX_SPLITS && current.dir === "column"} title={$t("llm.central.terminal.split_down")} aria-label={$t("llm.central.terminal.split_down")} onclick={() => split("column")}>
          <svg viewBox="0 0 16 16" width="14" height="14"><rect x="1.5" y="2.5" width="13" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M1.5 8h13" stroke="currentColor" stroke-width="1.4"/></svg>
        </button>
        <button class="icon" title={$t("llm.central.terminal.clear")} aria-label={$t("llm.central.terminal.clear")} onclick={() => apis[current.active]?.clearScreen()}>
          <svg viewBox="0 0 16 16" width="14" height="14"><circle cx="8" cy="8" r="5.5" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M4.2 11.8l7.6-7.6" stroke="currentColor" stroke-width="1.4"/></svg>
        </button>
        <button class="icon" title={$t("llm.central.terminal.kill")} aria-label={$t("llm.central.terminal.kill")} onclick={() => closePane(current.active)}>
          <svg viewBox="0 0 16 16" width="14" height="14"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 8.5h5.6l.7-8.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/></svg>
        </button>
      {/if}
      {#if onClosePanel}
        <button class="icon" title={$t("llm.central.terminal.hide")} aria-label={$t("llm.central.terminal.hide")} onclick={onClosePanel}>
          <svg viewBox="0 0 16 16" width="14" height="14"><path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </button>
      {/if}
    </div>
  </header>

  <div class="body">
    {#each tabs as tab (tab.key)}
      <div class="group" class:hidden={tab.key !== current?.key} style:flex-direction={tab.dir}>
        {#each tab.panes as pane (pane)}
          <div
            class="pane"
            class:active={tab.panes.length > 1 && pane === tab.active}
            role="presentation"
            onmousedown={() => { if (tab.active !== pane) { tab.active = pane; tabs = [...tabs]; save(); } }}
          >
            {#if tab.panes.length > 1 && (fg[pane]?.running || ended[pane])}
              <div class="pane-badge">
                {#if ended[pane]}
                  {$t("llm.central.terminal.ended")}
                {:else}
                  <span class="dot" aria-hidden="true"></span>{fg[pane]?.label}
                {/if}
              </div>
            {/if}
            <TerminalView
              id={pane}
              {cwd}
              {command}
              {env}
              {fontSize}
              initialInput={pane === firstPane ? initialInput : undefined}
              focused={tab.key === current?.key && pane === tab.active}
              bind:api={apis[pane]}
              onFocus={() => { if (tab.active !== pane) { tab.active = pane; tabs = [...tabs]; save(); } }}
              onSelectionChange={(v) => (hasSel[pane] = v)}
              onForeground={(f) => (fg[pane] = f)}
              onInfo={(i) => { if (!i.alive) ended[pane] = true; }}
              onExit={() => (ended[pane] = true)}
            />
            {#if ended[pane]}
              <button class="restart" onclick={() => restart(pane)}>{$t("llm.central.terminal.restart")}</button>
            {/if}
          </div>
        {/each}
      </div>
    {/each}
    {#if tabs.length === 0}
      <div class="empty">
        <button class="text-btn" onclick={addTab}>{$t("llm.central.terminal.new_tab")}</button>
      </div>
    {/if}
  </div>
</section>

<style>
  .term-panel {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: var(--pane-bg);
    border-top: var(--hairline) solid var(--separator);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-h-lg);
    padding: 0 var(--space-2);
    border-bottom: var(--hairline) solid var(--separator);
    flex: none;
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
    flex: 1;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tab {
    display: flex;
    align-items: center;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    flex: none;
  }
  .tab.active {
    background: var(--fill-2);
    color: var(--text);
  }
  .tab-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 4px 0 var(--space-2);
    background: none;
    border: 0;
    color: inherit;
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
    max-width: 180px;
  }
  .tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .count {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .tab-x {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    margin-right: 2px;
    border: 0;
    border-radius: var(--radius-xs);
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
  }
  .tab:hover .tab-x,
  .tab.active .tab-x {
    opacity: 1;
  }
  .tab-x:hover {
    background: var(--fill-3);
    color: var(--text);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex: none;
  }
  .icon {
    display: grid;
    place-items: center;
    width: var(--control-h);
    height: var(--control-h);
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-muted);
    cursor: pointer;
  }
  .icon:hover:not(:disabled) {
    background: var(--fill-2);
    color: var(--text);
  }
  .icon:disabled,
  .text-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .text-btn {
    height: 24px;
    padding: 0 var(--space-2);
    border: var(--hairline) solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .text-btn:hover:not(:disabled) {
    background: var(--fill-2);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 var(--space-2);
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--teal) 14%, transparent);
    color: var(--teal);
    font-size: var(--text-xs);
    font-family: var(--font-mono);
    white-space: nowrap;
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--teal);
    flex: none;
    animation: pulse 1.6s var(--ease-in-out) infinite;
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.35; }
  }
  @media (prefers-reduced-motion: reduce) {
    .dot { animation: none; }
  }
  .body {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  .group {
    position: absolute;
    inset: 0;
    display: flex;
    gap: var(--hairline);
    background: var(--separator);
  }
  .group.hidden {
    visibility: hidden;
    pointer-events: none;
  }
  .pane {
    position: relative;
    flex: 1 1 0;
    min-width: 0;
    min-height: 0;
    background: var(--pane-bg);
  }
  .pane.active {
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .pane-badge {
    position: absolute;
    top: 4px;
    right: 8px;
    z-index: 2;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 1px 8px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-muted);
    font-size: var(--text-caption);
    font-family: var(--font-mono);
    pointer-events: none;
  }
  .restart {
    position: absolute;
    bottom: var(--space-3);
    right: var(--space-3);
    z-index: 2;
    height: 26px;
    padding: 0 var(--space-3);
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--on-accent);
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
  }
</style>
