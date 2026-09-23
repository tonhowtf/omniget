<script lang="ts">
  // Approval drawer attached to the composer: "1/3" when several wait, the
  // full diff of an edit (DiffViewer) or the command with its cwd and reason,
  // and the engine decisions: Approve, Approve for this session, Always,
  // Decline, Cancel. Keys (outside text fields): Y approve, S session, A always,
  // N decline, [ ] previous/next.
  import { t } from "$lib/i18n";
  import type { ApprovalDecision, ApprovalRow } from "$lib/central/threads/types";
  import DiffViewer from "$components/central/vcs/DiffViewer.svelte";
  import { toFileDiffs } from "$lib/central/threads-ui/work";
  import { relativeTo } from "$lib/central/threads-ui/format";
  import Icon from "./Icon.svelte";

  let {
    approvals,
    cwd = null,
    responding = new Set<string>(),
    onrespond,
  }: {
    approvals: ApprovalRow[];
    cwd?: string | null;
    responding?: Set<string>;
    onrespond: (requestId: string, decision: ApprovalDecision) => void;
  } = $props();

  let index = $state(0);
  let current = $derived(approvals[Math.min(index, approvals.length - 1)] ?? null);
  let busy = $derived(!!current && responding.has(current.requestId));
  let d = $derived(current?.detail ?? null);
  let files = $derived(d?.diff ? toFileDiffs(d.diff, d.paths?.[0] ?? "file") : []);
  let isCommand = $derived(!!d?.command || current?.requestType === "command_execution_approval" || current?.requestType === "exec_command_approval");
  let inputJson = $derived.by(() => {
    if (!d?.input || d.command || d.diff) return "";
    try {
      return JSON.stringify(d.input, null, 2).slice(0, 8000);
    } catch {
      return "";
    }
  });

  $effect(() => {
    if (index >= approvals.length) index = Math.max(0, approvals.length - 1);
  });

  function kindKey(r: ApprovalRow): string {
    switch (r.requestType) {
      case "command_execution_approval":
      case "exec_command_approval":
        return "command";
      case "file_read_approval":
        return "file_read";
      case "file_change_approval":
      case "apply_patch_approval":
        return "file_change";
      case "mcp_elicitation_approval":
        return "mcp";
      case "permission_approval":
        return "permission";
      default:
        return "tool";
    }
  }

  function decide(decision: ApprovalDecision) {
    if (!current || busy) return;
    onrespond(current.requestId, decision);
  }

  /** Provider options replace the defaults when present. */
  let providerOptions = $derived((current?.options ?? []).filter((o) => o.decision));

  function isEditable(el: EventTarget | null): boolean {
    const n = el as HTMLElement | null;
    return !!n && (n.isContentEditable || n.tagName === "INPUT" || n.tagName === "TEXTAREA" || n.tagName === "SELECT");
  }

  function onKey(e: KeyboardEvent) {
    if (!current || e.metaKey || e.ctrlKey || e.altKey || isEditable(e.target)) return;
    const k = e.key.toLowerCase();
    if (k === "y") decide("accept");
    else if (k === "s") decide("acceptForSession");
    else if (k === "a") decide("acceptAlways");
    else if (k === "n") decide("decline");
    else if (k === "[") index = Math.max(0, index - 1);
    else if (k === "]") index = Math.min(approvals.length - 1, index + 1);
    else return;
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={onKey} />

{#if current}
  <section class="drawer" aria-label={$t("llm.central.threads.approval.title")} aria-live="polite">
    <header>
      <span class="shield"><Icon name="shield-warning" size={15} /></span>
      <strong>{$t(`llm.central.threads.approval.kind.${kindKey(current)}`)}</strong>
      {#if d?.toolName}<span class="tool mono">{d.toolName}</span>{/if}
      {#if approvals.length > 1}
        <span class="nav">
          <button type="button" class="nb" aria-label={$t("llm.central.threads.prev")} disabled={index === 0} onclick={() => (index -= 1)}><Icon name="caret-up" size={10} /></button>
          <span class="count">{index + 1}/{approvals.length}</span>
          <button type="button" class="nb" aria-label={$t("llm.central.threads.next")} disabled={index >= approvals.length - 1} onclick={() => (index += 1)}><Icon name="caret-down" size={10} /></button>
        </span>
      {/if}
    </header>

    {#if d?.reason}<p class="reason">{d.reason}</p>{/if}

    {#if isCommand && d?.command}
      <div class="cmd"><span class="prompt">$</span><code>{d.command}</code></div>
      {#if d.cwd}<p class="cwd"><Icon name="folder-simple" size={11} />{d.cwd}</p>{/if}
    {/if}

    {#if files.length}
      <div class="diff">
        <DiffViewer {files} height="min(46vh, 420px)" toolbar={files.length > 1} />
      </div>
    {:else if d?.preview}
      <pre class="detail">{d.preview}</pre>
    {/if}

    {#if !files.length && d?.paths?.length}
      <ul class="paths">
        {#each d.paths as p (p)}<li class="mono">{relativeTo(p, cwd)}</li>{/each}
      </ul>
    {/if}
    {#if inputJson}<pre class="detail">{inputJson}</pre>{/if}

    <footer>
      {#if providerOptions.length}
        {#each providerOptions as o (o.id)}
          <button type="button" class="button" class:primary={o.decision === "accept"} disabled={busy} onclick={() => decide(o.decision!)}>{o.label}</button>
        {/each}
      {:else}
        <button type="button" class="button primary" disabled={busy} onclick={() => decide("accept")}>
          {$t("llm.central.threads.approval.approve")}<kbd>Y</kbd>
        </button>
        <button type="button" class="button" disabled={busy} onclick={() => decide("acceptForSession")}>
          {$t("llm.central.threads.approval.session")}<kbd>S</kbd>
        </button>
        <button type="button" class="button" disabled={busy} onclick={() => decide("acceptAlways")}>
          {$t("llm.central.threads.approval.always")}<kbd>A</kbd>
        </button>
        <span class="grow"></span>
        <button type="button" class="button" disabled={busy} onclick={() => decide("decline")}>
          {$t("llm.central.threads.approval.decline")}<kbd>N</kbd>
        </button>
        <button type="button" class="button danger" disabled={busy} onclick={() => decide("cancel")} title={$t("llm.central.threads.approval.cancel_hint")}>
          {$t("llm.central.threads.approval.cancel")}
        </button>
      {/if}
    </footer>
  </section>
{/if}

<style>
  .drawer { display: grid; gap: 8px; padding: 12px 14px; border-radius: 16px 16px 0 0; border: 1px solid color-mix(in srgb, var(--orange, #f59e0b) 45%, var(--separator)); border-bottom: 0; background: color-mix(in srgb, var(--orange, #f59e0b) 7%, var(--surface, #fff)); }
  header { display: flex; align-items: center; gap: 8px; font-size: 13px; }
  .shield { color: var(--orange, #f59e0b); display: inline-flex; }
  strong { color: var(--text); font-weight: 650; }
  .tool { color: var(--text-muted); font-size: 12px; }
  .mono { font-family: var(--font-mono); }
  .nav { margin-left: auto; display: inline-flex; align-items: center; gap: 4px; }
  .count { font-variant-numeric: tabular-nums; font-size: 12px; color: var(--text-muted); }
  .nb { border: 0; background: var(--fill-1); border-radius: 5px; padding: 3px; cursor: pointer; color: var(--text); display: inline-flex; }
  .nb:disabled { opacity: 0.35; cursor: default; }
  .reason { margin: 0; font-size: 13px; color: var(--text); }
  .cmd { display: flex; gap: 6px; font-family: var(--font-mono); font-size: 12.5px; background: var(--fill-1); border-radius: 8px; padding: 8px 10px; max-height: 8rem; overflow: auto; }
  .cmd code { white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
  .prompt { color: var(--text-muted); }
  .cwd { margin: -2px 0 0; display: flex; align-items: center; gap: 5px; font-size: 11.5px; color: var(--text-muted); font-family: var(--font-mono); overflow-wrap: anywhere; }
  .diff { border: 1px solid var(--separator); border-radius: 10px; overflow: hidden; background: var(--surface, #fff); }
  .detail { margin: 0; max-height: 12rem; overflow: auto; font-family: var(--font-mono); font-size: 11.5px; background: var(--fill-1); border-radius: 8px; padding: 8px 10px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .paths { margin: 0; padding-left: 18px; font-size: 12px; }
  footer { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
  footer .button { display: inline-flex; align-items: center; gap: 6px; }
  .grow { flex: 1; }
  kbd { font-family: var(--font-mono); font-size: 10px; padding: 0 4px; border-radius: 4px; background: color-mix(in srgb, currentColor 14%, transparent); opacity: 0.8; }
  .button.danger { color: var(--red, #ef4444); }
</style>
