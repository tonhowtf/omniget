<script lang="ts">
  /**
   * Right pane: what the selected agent is allowed to do and what the running
   * turn is costing. Everything here is read from state the store already has —
   * no command of its own, no polling.
   */
  import { t } from "$lib/i18n";
  import type { AgentDef, ToolGrant } from "$lib/llm/types";
  import { effectiveModelLabel } from "$lib/llm/types";
  import { getActiveConversation, getActiveTurn } from "$lib/stores/llm-store.svelte";
  import PermissionRules from "./PermissionRules.svelte";
  import { invoke } from "@tauri-apps/api/core";

  // Tools that need a project folder: absent in a personal conversation
  // (the backend removes them from the effective agent; this only says so).
  const PROJECT_TOOLS = new Set(["fs_read", "fs_list", "fs_glob", "fs_grep", "todo_write", "fs_edit", "fs_write", "fs_apply_patch", "shell_exec", "kb_search", "kb_write"]);
  let personal = $state(false);

  let { agent = null }: { agent?: AgentDef | null } = $props();

  let conversation = $derived(getActiveConversation());
  let turn = $derived(getActiveTurn());
  let modelText = $derived(effectiveModelLabel(conversation, agent));
  let grants = $derived<ToolGrant[]>(agent?.tools ?? []);
  let budget = $derived(agent?.budget ?? {});

  function grantName(grant: ToolGrant): string {
    if (grant.source === "internal") return grant.name;
    if (grant.source === "mcp") return `${grant.server}/${grant.tool}`;
    return grant.name;
  }

  function fmtUsd(value: number | null | undefined): string {
    return value === null || value === undefined ? $t("llm.inspector.unlimited") : `$${value.toFixed(4)}`;
  }

  function fmtNum(value: number | null | undefined): string {
    return value === null || value === undefined ? $t("llm.inspector.unlimited") : String(value);
  }

  let usage = $derived(turn?.usage ?? null);

  $effect(() => {
    const id = conversation?.id;
    if (!id) { personal = false; return; }
    invoke<{ kind?: string } | null>("assist_conversation_context", { conversationId: id })
      .then((ctx) => { if (conversation?.id === id) personal = ctx?.kind === "projectless"; })
      .catch(() => { personal = false; });
  });
</script>

<aside class="inspector" aria-label={$t("llm.inspector.title")}>
  <section class="block">
    <h3>{$t("llm.inspector.model")}</h3>
    <p class="mono">{modelText || "—"}</p>
  </section>

  <section class="block">
    <h3>{$t("llm.inspector.tools")}</h3>
    {#if grants.length === 0}
      <p class="dim">{$t("llm.inspector.no_tools")}</p>
    {:else}
      <ul class="tool-list">
        {#each grants as grant (grantName(grant) + grant.mode)}
          <li>
            <span class="mono">{grantName(grant)}</span>
            <span class="tag">{$t(`llm.inspector.grant_${grant.mode}`)}</span>
            {#if personal && grant.source === "internal" && PROJECT_TOOLS.has(grant.name)}
              <span class="tag off">{$t("assist.conversation.tool_needs_project")}</span>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <PermissionRules agentId={agent?.id ?? null} refreshKey={turn ? 1 : 0} />

  <section class="block">
    <h3>{$t("llm.inspector.budget")}</h3>
    <dl class="kv">
      <dt>{$t("llm.inspector.usd_per_day")}</dt>
      <dd>{fmtUsd(budget.usd_per_day)}</dd>
      <dt>{$t("llm.inspector.tokens_per_turn")}</dt>
      <dd>{fmtNum(budget.tokens_per_turn)}</dd>
      <dt>{$t("llm.inspector.max_tool_calls")}</dt>
      <dd>{fmtNum(budget.max_tool_calls_per_turn)}</dd>
    </dl>
  </section>

  <section class="block">
    <h3>{$t("llm.inspector.turn")}</h3>
    {#if !turn}
      <p class="dim">{$t("llm.inspector.no_turn")}</p>
    {:else}
      <dl class="kv">
        <dt>{$t("llm.inspector.turn_cost")}</dt>
        <dd>{usage?.cost_usd != null ? `$${usage.cost_usd.toFixed(4)}` : "—"}</dd>
        <dt>{$t("llm.inspector.tokens")}</dt>
        <dd>{usage ? `${usage.input_tokens} / ${usage.output_tokens}` : "—"}</dd>
        <dt>{$t("llm.inspector.latency")}</dt>
        <dd>{usage?.first_token_ms != null ? `${usage.first_token_ms} ms` : "—"}</dd>
      </dl>
    {/if}
  </section>
</aside>

<style>
  .inspector {
    width: 300px;
    min-width: 300px;
    overflow-y: auto;
    padding: var(--space-4);
    border-left: var(--hairline) solid var(--separator);
    background: var(--surface);
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .block h3 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .block p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .dim {
    color: var(--text-dim);
  }

  .tool-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .tool-list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .kv {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--space-1) var(--space-3);
    margin: 0;
  }

  .kv dt {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .kv dd {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
</style>
