<script lang="ts">
  // One tool call in the timeline: icon, tense-aware label, failure badge,
  // hover timestamp; expands (Enter/Space/click) to the command with cwd and
  // output, the mini diff of an edit, touched files as chips, or the raw JSON
  // of an MCP / dynamic tool.
  import { t } from "$lib/i18n";
  import type { WorkEntry } from "$lib/central/threads-ui/work";
  import { diffStats, entryVerbKey } from "$lib/central/threads-ui/work";
  import { formatClock, formatFull, relativeTo, formatTokens } from "$lib/central/threads-ui/format";
  import type { IconName } from "$lib/central/threads-ui/icons";
  import Icon from "./Icon.svelte";

  let {
    entry,
    depth = 0,
    cwd = null,
    onopenfile,
    onopendiff,
    expanded = null,
    ontoggle,
  }: {
    entry: WorkEntry;
    /** Controlled disclosure (survives virtual-list recycling). */
    expanded?: boolean | null;
    ontoggle?: () => void;
    depth?: number;
    cwd?: string | null;
    onopenfile?: (path: string) => void;
    onopendiff?: (path: string) => void;
  } = $props();

  let localOpen = $state(false);
  let open = $derived(expanded ?? localOpen);

  const ICON: Record<WorkEntry["kind"], IconName> = {
    command: "terminal-window",
    read: "eye",
    edit: "pencil-simple",
    web: "globe",
    search: "magnifying-glass",
    mcp: "plug",
    dynamic: "hammer",
    agent: "robot",
    image: "image",
    approval: "shield-check",
    error: "warning-circle",
    warning: "warning",
    info: "sparkle",
    compaction: "arrows-split",
    other: "hammer",
  };

  let stats = $derived(diffStats(entry.diff));
  let expandable = $derived(!!(entry.output || entry.diff || entry.command || entry.raw || entry.detail || entry.paths.length > 1 || entry.agent?.summary));
  let rawJson = $derived.by(() => {
    if (!open || !entry.raw) return "";
    try {
      return JSON.stringify(entry.raw, null, 2).slice(0, 20_000);
    } catch {
      return "";
    }
  });
  let diffLines = $derived(open && entry.diff ? entry.diff.split("\n").filter((l) => !l.startsWith("---") && !l.startsWith("+++")).slice(0, 80) : []);

  function toggle() {
    if (!expandable) return;
    if (ontoggle) ontoggle();
    else localOpen = !localOpen;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      toggle();
    }
  }

  // A click that ends a text selection must not collapse the row.
  function onClick() {
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed && open) return;
    toggle();
  }
</script>

<div class="work" class:open class:failed={entry.status === "failed"} class:severe={entry.severe} style:--depth={depth} data-kind={entry.kind}>
  <div
    class="head"
    role="button"
    tabindex="0"
    aria-expanded={expandable ? open : undefined}
    onclick={onClick}
    onkeydown={onKey}
  >
    <span class="ico" class:running={entry.status === "running"}><Icon name={ICON[entry.kind]} size={13} /></span>
    <span class="verb">{$t(entryVerbKey(entry))}</span>
    <span class="label" class:mono={entry.kind === "command" || entry.kind === "read" || entry.kind === "edit"} title={entry.label}>
      {entry.kind === "read" || entry.kind === "edit" ? relativeTo(entry.label, cwd) : entry.label}
    </span>
    {#if entry.kind === "edit" && entry.paths.length > 1}<span class="more">+{entry.paths.length - 1}</span>{/if}
    {#if entry.kind === "edit" && (stats.add || stats.del)}<span class="stat"><span class="add">+{stats.add}</span> <span class="del">−{stats.del}</span></span>{/if}
    {#if entry.status === "failed"}<span class="badge fail">{$t("llm.central.threads.work.failed")}</span>
    {:else if entry.status === "declined"}<span class="badge">{$t("llm.central.threads.work.declined")}</span>
    {:else if entry.status === "stopped"}<span class="badge">{$t("llm.central.threads.work.stopped")}</span>{/if}
    <span class="spacer"></span>
    <time class="ts" datetime={entry.createdAt} title={formatFull(entry.createdAt)}>{formatClock(entry.createdAt)}</time>
    <span class="chev" class:hidden={!expandable}><Icon name="caret-right" size={10} /></span>
  </div>

  {#if open}
    <div class="body">
      {#if entry.command}
        <div class="cmd"><span class="prompt">$</span><code>{entry.command}</code></div>
        {#if entry.cwd}<div class="cwd">{relativeTo(entry.cwd, cwd)}</div>{/if}
      {/if}
      {#if entry.agent}
        <div class="agent">
          <strong>{entry.agent.title}</strong>
          <span class="mono muted">{entry.agent.status}{entry.agent.model ? ` · ${entry.agent.model}` : ""}{entry.agent.tokens ? ` · ${formatTokens(entry.agent.tokens)} tok` : ""}</span>
          {#if entry.agent.summary}<pre class="out">{entry.agent.summary}</pre>{/if}
        </div>
      {/if}
      {#if diffLines.length}
        <pre class="mini-diff">{#each diffLines as l, i (i)}<span class={l.startsWith("+") ? "a" : l.startsWith("-") ? "d" : l.startsWith("@@") ? "h" : ""}>{l}
</span>{/each}</pre>
        {#if entry.paths[0] && onopendiff}
          <button type="button" class="link" onclick={() => onopendiff?.(entry.paths[0])}><Icon name="git-diff" size={12} />{$t("llm.central.threads.work.open_diff")}</button>
        {/if}
      {/if}
      {#if entry.output}<pre class="out">{entry.output.length > 20_000 ? entry.output.slice(-20_000) : entry.output}</pre>{/if}
      {#if entry.detail && entry.detail !== entry.output}<pre class="out detail">{entry.detail}</pre>{/if}
      {#if rawJson}<pre class="out">{rawJson}</pre>{/if}
      {#if entry.paths.length > 1 || (entry.paths.length && entry.kind !== "read" && entry.kind !== "edit")}
        <div class="chips">
          {#each entry.paths as p (p)}
            <button type="button" class="file" onclick={() => onopenfile?.(p)}><Icon name="file-text" size={11} />{relativeTo(p, cwd)}</button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .work { margin-left: calc(var(--depth) * 18px); border-radius: 8px; }
  .head { display: flex; align-items: center; gap: 7px; min-height: 28px; padding: 3px 8px; border-radius: 8px; cursor: default; color: var(--text-muted); font-size: 13px; position: relative; }
  .head[aria-expanded] { cursor: pointer; }
  .head:hover { background: var(--fill-1); }
  .head:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .ico { display: inline-flex; color: var(--text-muted); opacity: 0.85; }
  .ico.running { color: var(--blue, var(--accent)); animation: breathe 1.4s ease-in-out infinite; }
  .failed .ico { color: color-mix(in srgb, var(--red, #ef4444) 70%, var(--text-muted)); }
  .severe .head { color: var(--red, #ef4444); }
  .severe .ico { color: var(--red, #ef4444); opacity: 1; }
  @keyframes breathe { 50% { opacity: 0.4; } }
  .verb { color: var(--text); font-weight: 500; white-space: nowrap; }
  .label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .open .label { white-space: normal; overflow-wrap: anywhere; user-select: text; }
  .mono { font-family: var(--font-mono); font-size: 12px; }
  .more { font-size: 11px; color: var(--text-muted); }
  .stat { font-family: var(--font-mono); font-size: 11.5px; white-space: nowrap; }
  .add { color: var(--green, #22c55e); }
  .del { color: var(--red, #ef4444); }
  .badge { font-size: 10.5px; padding: 0 6px; border-radius: 5px; background: var(--fill-2); white-space: nowrap; }
  .badge.fail { color: var(--red, #ef4444); background: color-mix(in srgb, var(--red, #ef4444) 12%, transparent); }
  .spacer { flex: 1; }
  .ts { position: absolute; right: 28px; font-size: 11px; color: var(--text-muted); opacity: 0; transition: opacity 120ms; font-variant-numeric: tabular-nums; pointer-events: none; background: inherit; }
  .head:hover .ts, .head:focus-within .ts { opacity: 1; }
  .chev { display: inline-flex; transition: transform 150ms; color: var(--text-muted); }
  .chev.hidden { visibility: hidden; }
  .open .chev { transform: rotate(90deg); }
  .body { margin: 2px 0 6px 28px; display: grid; gap: 6px; }
  .cmd { display: flex; gap: 6px; font-family: var(--font-mono); font-size: 12px; background: var(--fill-1); border-radius: 8px; padding: 6px 10px; overflow-x: auto; }
  .cmd code { white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
  .prompt { color: var(--text-muted); user-select: none; }
  .cwd { font-size: 11px; color: var(--text-muted); font-family: var(--font-mono); margin-top: -3px; }
  .out { margin: 0; max-height: 16rem; overflow: auto; font-family: var(--font-mono); font-size: 11.5px; line-height: 1.45; background: var(--fill-1); border-radius: 8px; padding: 8px 10px; white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; color: var(--text); }
  .out.detail { color: var(--text-muted); }
  .mini-diff { margin: 0; max-height: 16rem; overflow: auto; font-family: var(--font-mono); font-size: 11.5px; line-height: 1.45; background: var(--fill-1); border-radius: 8px; padding: 6px 0; white-space: pre; user-select: text; }
  .mini-diff span { display: block; padding: 0 10px; }
  .mini-diff .a { background: color-mix(in srgb, var(--green, #22c55e) 14%, transparent); }
  .mini-diff .d { background: color-mix(in srgb, var(--red, #ef4444) 14%, transparent); }
  .mini-diff .h { color: var(--text-muted); }
  .agent { display: grid; gap: 4px; font-size: 12.5px; }
  .muted { color: var(--text-muted); }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .file, .link { display: inline-flex; align-items: center; gap: 4px; border: 0; background: var(--accent-soft); color: var(--accent-hi); font-family: var(--font-mono); font-size: 11px; padding: 2px 7px; border-radius: 6px; cursor: pointer; }
  .link { font-family: inherit; justify-self: start; }
  @media (prefers-reduced-motion: reduce) { .ico.running { animation: none; } }
</style>
