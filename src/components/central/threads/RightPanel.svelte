<script lang="ts">
  // Right panel of a thread: Diff, Terminal, Files, Plan, Tasks, PR. With no
  // surface chosen it shows the launcher; letters D T F P A R open a surface
  // (outside text fields). Every choice here is a user action, so proactive
  // opens (big-turn diff) stop once the user touched the panel.
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import type { PanelTab } from "$lib/central/threads-ui/ui-state.svelte";
  import type { IconName } from "$lib/central/threads-ui/icons";
  import Icon from "./Icon.svelte";
  import BrowserPanel from "$components/central/preview/BrowserPanel.svelte";
  import { browserTabs } from "$components/central/preview/preview-api";
  import { threadsUi } from "$lib/central/threads-ui/ui-state.svelte";
  import { threadsStore } from "$lib/central/threads/threads-store.svelte";

  let {
    tab,
    counts = {},
    available = {},
    letters = true,
    onselect,
    onclose,
    diff,
    terminal,
    files,
    plan,
    tasks,
    pr,
  }: {
    tab: PanelTab | null;
    counts?: Partial<Record<PanelTab, number>>;
    /** Unavailable surfaces stay listed with a reason. */
    available?: Partial<Record<PanelTab, string | null>>;
    letters?: boolean;
    onselect: (tab: PanelTab | null) => void;
    onclose: () => void;
    diff: Snippet;
    terminal: Snippet;
    files: Snippet;
    plan: Snippet;
    tasks: Snippet;
    pr: Snippet;
  } = $props();

  const SURFACES: { id: PanelTab; key: string; icon: IconName }[] = [
    { id: "diff", key: "D", icon: "git-diff" },
    { id: "terminal", key: "T", icon: "terminal-window" },
    { id: "files", key: "F", icon: "files" },
    { id: "plan", key: "P", icon: "list-checks" },
    { id: "tasks", key: "A", icon: "robot" },
    { id: "pr", key: "R", icon: "git-pull-request" },
  ];

  let launcherEl = $state<HTMLElement | null>(null);

  // Browser surface (preview module): kept outside PanelTab, remembered per
  // thread in memory. Any other surface chosen (by the user or proactively)
  // takes over.
  let browserThread = $derived(threadsUi.selected);
  let browserCwd = $derived.by(() => {
    const th = threadsStore.threads.find((x) => x.threadId === browserThread);
    if (!th) return null;
    return th.worktreePath ?? threadsStore.projects.find((p) => p.projectId === th.projectId)?.workspaceRoot ?? null;
  });
  let browserOn = $state(!!threadsUi.selected && browserTabs.has(threadsUi.selected));
  let lastTab: PanelTab | null | undefined = undefined;
  $effect(() => {
    const cur = tab;
    if (lastTab !== undefined && cur !== lastTab) setBrowser(false);
    lastTab = cur;
  });
  function setBrowser(on: boolean) {
    browserOn = on;
    const id = browserThread;
    if (!id) return;
    if (on) browserTabs.add(id);
    else browserTabs.delete(id);
  }
  function pick(id: PanelTab | null) {
    setBrowser(false);
    onselect(id);
  }
  // The terminal mounts on first use only (it opens a shell), then stays
  // mounted while hidden so switching tabs never kills the session view.
  let terminalUsed = $state(false);
  $effect(() => {
    if (tab === "terminal") terminalUsed = true;
  });

  function isEditable(el: EventTarget | null): boolean {
    const n = el as HTMLElement | null;
    return !!n && (n.isContentEditable || n.tagName === "INPUT" || n.tagName === "TEXTAREA" || n.tagName === "SELECT" || !!n.closest?.(".xterm"));
  }

  function onKey(e: KeyboardEvent) {
    if (tab !== null || browserOn || !letters || e.metaKey || e.ctrlKey || e.altKey || isEditable(e.target)) return;
    if (e.key.toUpperCase() === "B" && browserThread) {
      e.preventDefault();
      setBrowser(true);
      return;
    }
    const s = SURFACES.find((x) => x.key === e.key.toUpperCase());
    if (!s || available[s.id]) return;
    e.preventDefault();
    pick(s.id);
  }

  function onLauncherKey(e: KeyboardEvent) {
    const btns = [...(launcherEl?.querySelectorAll<HTMLButtonElement>("button.surface") ?? [])];
    const at = btns.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "ArrowDown" || e.key === "ArrowRight") {
      e.preventDefault();
      btns[(at + 1) % btns.length]?.focus();
    } else if (e.key === "ArrowUp" || e.key === "ArrowLeft") {
      e.preventDefault();
      btns[(at - 1 + btns.length) % btns.length]?.focus();
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<aside class="panel" aria-label={$t("llm.central.threads.panel.title")}>
  <div class="tabs" role="tablist">
    {#each SURFACES as s (s.id)}
      <button
        type="button"
        role="tab"
        class="tab"
        aria-selected={tab === s.id && !browserOn}
        title={`${$t(`llm.central.threads.panel.${s.id}`)} (${s.key})`}
        disabled={!!available[s.id]}
        onclick={() => pick(s.id)}
      >
        <Icon name={s.icon} size={14} />
        <span class="tl">{$t(`llm.central.threads.panel.${s.id}`)}</span>
        {#if counts[s.id]}<span class="badge">{counts[s.id]}</span>{/if}
      </button>
    {/each}
    <button type="button" role="tab" class="tab" aria-selected={browserOn} title={`${$t("llm.central.preview.tab")} (B)`} disabled={!browserThread} onclick={() => setBrowser(true)}>
      <Icon name="globe" size={14} />
      <span class="tl">{$t("llm.central.preview.tab")}</span>
    </button>
    <span class="grow"></span>
    <button type="button" class="close" aria-label={$t("llm.central.threads.panel.close")} title={$t("llm.central.threads.panel.close")} onclick={onclose}><Icon name="x" size={13} /></button>
  </div>

  <div class="content">
    {#if tab === null && !browserOn}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div class="launcher" bind:this={launcherEl} role="menu" tabindex="-1" onkeydown={onLauncherKey} aria-label={$t("llm.central.threads.panel.launcher")}>
        <p class="lt">{$t("llm.central.threads.panel.launcher")}</p>
        {#each SURFACES as s (s.id)}
          <button type="button" class="surface" role="menuitem" disabled={!!available[s.id]} onclick={() => pick(s.id)}>
            <span class="tile" data-s={s.id}><Icon name={s.icon} size={15} /></span>
            <span class="st">
              <strong>{$t(`llm.central.threads.panel.${s.id}`)}</strong>
              <small>{available[s.id] ?? $t(`llm.central.threads.panel.${s.id}_hint`)}</small>
            </span>
            {#if counts[s.id]}<span class="badge">{counts[s.id]}</span>{/if}
            <kbd>{s.key}</kbd>
          </button>
        {/each}
        <button type="button" class="surface" role="menuitem" disabled={!browserThread} onclick={() => setBrowser(true)}>
          <span class="tile" data-s="browser"><Icon name="globe" size={15} /></span>
          <span class="st">
            <strong>{$t("llm.central.preview.tab")}</strong>
            <small>{$t("llm.central.preview.tab_hint")}</small>
          </span>
          <kbd>B</kbd>
        </button>
      </div>
    {:else}
      <!-- Tabs stay mounted once opened (terminal keeps its session, diff its scroll). -->
      <div class="surface-body" hidden={browserOn || tab !== "diff"}>{#if tab === "diff"}{@render diff()}{/if}</div>
      <div class="surface-body" hidden={browserOn || tab !== "terminal"}>{#if terminalUsed}{@render terminal()}{/if}</div>
      <div class="surface-body" hidden={browserOn || tab !== "files"}>{#if tab === "files"}{@render files()}{/if}</div>
      <div class="surface-body" hidden={browserOn || tab !== "plan"}>{#if tab === "plan"}{@render plan()}{/if}</div>
      <div class="surface-body" hidden={browserOn || tab !== "tasks"}>{#if tab === "tasks"}{@render tasks()}{/if}</div>
      <div class="surface-body" hidden={browserOn || tab !== "pr"}>{#if tab === "pr"}{@render pr()}{/if}</div>
      {#if browserOn && browserThread}<div class="surface-body"><BrowserPanel threadId={browserThread} cwd={browserCwd} /></div>{/if}
    {/if}
  </div>
</aside>

<style>
  .panel { display: flex; flex-direction: column; min-height: 0; height: 100%; border-left: 1px solid var(--separator); background: var(--surface, transparent); min-width: 0; }
  .tabs { display: flex; align-items: center; gap: 2px; padding: 6px 6px; border-bottom: 1px solid var(--separator); overflow-x: auto; scrollbar-width: none; flex-shrink: 0; }
  .tab { display: inline-flex; align-items: center; gap: 5px; border: 0; background: transparent; color: var(--text-muted); font-size: 12px; padding: 5px 8px; border-radius: 7px; cursor: pointer; white-space: nowrap; }
  .tab:hover:not(:disabled) { background: var(--fill-1); color: var(--text); }
  .tab[aria-selected="true"] { background: var(--accent-soft); color: var(--accent-hi); font-weight: 600; }
  .tab:disabled { opacity: 0.4; cursor: default; }
  @container (max-width: 420px) { .tl { display: none; } }
  .panel { container-type: inline-size; }
  .badge { font-size: 10px; min-width: 16px; text-align: center; padding: 0 4px; border-radius: 999px; background: var(--accent); color: var(--on-accent, #fff); font-weight: 700; }
  .grow { flex: 1; }
  .close { border: 0; background: transparent; color: var(--text-muted); padding: 5px; border-radius: 6px; cursor: pointer; display: inline-flex; }
  .close:hover { background: var(--fill-2); }
  .content { flex: 1; min-height: 0; position: relative; }
  .surface-body { position: absolute; inset: 0; overflow: hidden; }
  .surface-body[hidden] { display: none; }
  .launcher { padding: 18px 14px; display: grid; gap: 4px; align-content: start; height: 100%; overflow: auto; box-sizing: border-box; outline: none; }
  .lt { margin: 0 4px 8px; font-size: 12px; font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.03em; }
  .surface { display: flex; align-items: center; gap: 10px; border: 0; background: transparent; color: var(--text); padding: 8px; border-radius: 10px; cursor: pointer; text-align: left; }
  .surface:hover:not(:disabled), .surface:focus-visible { background: var(--fill-1); outline: none; }
  .surface:disabled { opacity: 0.5; cursor: default; }
  .tile { width: 28px; height: 28px; border-radius: 8px; display: inline-flex; align-items: center; justify-content: center; color: #fff; flex-shrink: 0; box-shadow: inset 0 0 0 0.5px rgba(255,255,255,.25), 0 0.5px 1px rgba(0,0,0,.2); }
  .tile[data-s="diff"] { background: linear-gradient(180deg, #ffb340, #f28500); }
  .tile[data-s="terminal"] { background: linear-gradient(180deg, #5c5c60, #2c2c30); }
  .tile[data-s="files"] { background: linear-gradient(180deg, #5aa9ff, #1e6fe8); }
  .tile[data-s="plan"] { background: linear-gradient(180deg, #c77dff, #7b3fe4); }
  .tile[data-s="tasks"] { background: linear-gradient(180deg, #4cd964, #2aa845); }
  .tile[data-s="pr"] { background: linear-gradient(180deg, #ff7a7a, #e33a3a); }
  .tile[data-s="browser"] { background: linear-gradient(180deg, #5ad1ff, #1e8fe8); }
  .st { flex: 1; display: grid; gap: 1px; min-width: 0; }
  .st strong { font-size: 13px; font-weight: 600; }
  .st small { font-size: 11.5px; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  kbd { font-family: var(--font-mono); font-size: 11px; padding: 1px 6px; border-radius: 5px; background: var(--fill-2); color: var(--text-muted); }
</style>
