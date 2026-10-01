<script lang="ts">
  import { t } from "$lib/i18n";
  import { isTerminalKind, type CanvasNode } from "$lib/canvas/model/types";
  import { agentStates, stateCss, type AgentState } from "$lib/canvas/state/agents.svelte";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { noteRead } from "$lib/canvas/notes/api";
  import { ACCOUNT_AGENTS, canvasAccounts } from "$lib/canvas/accounts/store.svelte";
  import { terminalIdOf } from "$lib/canvas/term/gl-types";
  import AccountBadge from "../accounts/AccountBadge.svelte";

  let { node, selected = false }: { node: CanvasNode; selected?: boolean } = $props();

  const agent = $derived(isTerminalKind(node.kind));
  const signal = $derived(agent ? agentStates.of(node) : undefined);
  const agentState = $derived<AgentState>(signal?.state ?? "unknown");
  const role = $derived(typeof node.data.roleId === "string" ? (node.data.roleId as string) : "");
  const agentName = $derived(typeof node.data.agent === "string" ? (node.data.agent as string) : "");
  void canvasAccounts.load();
  const accountBadge = $derived.by(() => {
    if (agent && ACCOUNT_AGENTS.has(agentName || "shell")) {
      const tid = terminalIdOf(node);
      return canvasAccounts.terminalBadge(node, tid, tid in canvasAccounts.running);
    }
    if (node.kind === "portal") return canvasAccounts.sessionBadge(node);
    return null;
  });
  const kindName = $derived(
    node.kind === "portal" ? $t("canvas.engine.spawn.portal", { default: "Portal" }) : agentName,
  );
  const lastLine = $derived(signal?.lastLine ?? signal?.message ?? "");
  const engine = useEngine();
  let fileText = $state<string | null>(null);
  const externalNote = $derived(node.kind === "note" && typeof node.data.path === "string" && node.data.text == null);
  $effect(() => {
    if (!externalNote) return;
    void node.data.lastModifiedAt;
    let alive = true;
    noteRead(engine.store.workspaceId, engine.store.floorId, node.id)
      .then((c) => {
        if (alive) fileText = c.text;
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  });
  const preview = $derived(
    String(fileText ?? node.data.text ?? node.data.url ?? node.data.path ?? "")
      .split("\n")
      .filter((l) => l.trim())
      .slice(0, externalNote || node.kind === "note" ? 14 : 4)
      .join("\n"),
  );
  const segments = $derived.by(() => {
    const used = signal?.contextTokens;
    const limit = signal?.contextLimit;
    if (!used || !limit) return null;
    return Math.max(0, Math.min(8, Math.round((used / limit) * 8)));
  });
  const cost = $derived(signal?.costUsd);

  const STATE_KEYS: Record<AgentState, [string, string]> = {
    idle: ["canvas.engine.state.idle", "Idle"],
    thinking: ["canvas.engine.state.thinking", "Thinking"],
    working: ["canvas.engine.state.working", "Working"],
    waitingPeer: ["canvas.engine.state.waitingPeer", "Waiting for an agent"],
    blocked: ["canvas.engine.state.blocked", "Needs you"],
    success: ["canvas.engine.state.success", "Done"],
    failed: ["canvas.engine.state.failed", "Failed"],
    compacting: ["canvas.engine.state.compacting", "Compacting"],
    looping: ["canvas.engine.state.looping", "Looping"],
    unknown: ["canvas.engine.state.unknown", "No signal"],
  };
</script>

<div class="card" class:selected class:blocked={agentState === "blocked"} class:agent>
  <div class="head">
    {#if agent}<span class="dot" style:background={stateCss(agentState)}></span>{/if}
    <span class="title">{node.title || node.kind}</span>
    {#if accountBadge}
      <AccountBadge badge={accountBadge} readonly />
      {#if kindName}<span class="tag">{kindName}</span>{/if}
    {:else if agent && agentName}<span class="tag">{agentName}</span>{/if}
  </div>
  {#if agent}
    <div class="meta">
      <span class="state" style:color={stateCss(agentState)}>{$t(STATE_KEYS[agentState][0], { default: STATE_KEYS[agentState][1] })}</span>
      {#if role}<span class="role">{role}</span>{/if}
      {#if cost !== undefined}<span class="cost">${cost.toFixed(2)}</span>{/if}
    </div>
    {#if lastLine}<div class="line">{lastLine}</div>{/if}
    {#if segments !== null}
      <div class="meter" aria-label={$t("canvas.engine.context", { default: "Context" })}>
        {#each Array(8) as _, i (i)}
          <span
            class="seg"
            class:on={i < segments}
            class:amber={segments >= 6 && i < segments}
            class:coral={segments >= 7 && i < segments}
          ></span>
        {/each}
      </div>
    {/if}
  {:else if preview}
    <div class="preview">{preview}</div>
  {/if}
</div>

<style>
  .card {
    width: 100%;
    height: 100%;
    box-sizing: border-box;
    border-radius: 10px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow: hidden;
    color: var(--text);
    font-size: 12px;
    pointer-events: none;
  }
  .card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .card.blocked {
    border-color: #d96a62;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .title {
    font-weight: 600;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .tag {
    font-size: 10px;
    color: var(--text-muted);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0 4px;
  }
  .meta {
    display: flex;
    gap: 8px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
  }
  .state {
    font-weight: 600;
  }
  .cost {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
  }
  .line,
  .preview {
    font: 11px/1.4 var(--font-mono, monospace);
    color: var(--text-muted);
    white-space: pre-wrap;
    overflow: hidden;
    flex: 1;
    min-height: 0;
  }
  .meter {
    display: flex;
    gap: 2px;
    margin-top: auto;
  }
  .seg {
    flex: 1;
    height: 4px;
    border-radius: 1px;
    background: var(--fill-2, rgba(127, 127, 127, 0.25));
  }
  .seg.on {
    background: var(--accent);
  }
  .seg.amber {
    background: #dcab3c;
  }
  .seg.coral {
    background: #d96a62;
  }
</style>
