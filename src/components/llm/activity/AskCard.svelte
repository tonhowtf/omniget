<script lang="ts">
  import { t } from "$lib/i18n";
  import { answerAsk, respondApproval, type ApprovalDecision, type PendingAsk } from "$lib/stores/llm-jobs-store.svelte";

  let { ask, agentName = "" }: { ask: PendingAsk; agentName?: string } = $props();
  let busy = $state(false);

  async function answer(decision: ApprovalDecision) {
    if (busy) return;
    busy = true;
    try {
      if (ask.id) await respondApproval(ask.id, decision);
      else await answerAsk(ask, decision !== "decline", decision === "accept_always");
    } finally {
      busy = false;
    }
  }
</script>

<div class="ask" role="group" aria-label={$t("llm.jobs.ask.title", { tool: ask.tool })}>
  <div class="ask-title">
    {#if agentName}{agentName} · {/if}{$t("llm.jobs.ask.title", { tool: ask.tool })}
  </div>
  {#if ask.preview}<pre class="pre">{ask.preview}</pre>{/if}
  <div class="actions">
    <button type="button" class="button active" disabled={busy} onclick={() => void answer("accept")}>
      {$t("llm.jobs.ask.allow")}
    </button>
    {#if ask.id}
      <button type="button" class="button" disabled={busy} onclick={() => void answer("accept_session")}>
        {$t("llm.jobs.ask.session")}
      </button>
    {/if}
    <button type="button" class="button" disabled={busy} onclick={() => void answer("accept_always")}>
      {$t("llm.jobs.ask.always")}
    </button>
    <button type="button" class="button" disabled={busy} onclick={() => void answer("decline")}>
      {$t("llm.jobs.ask.deny")}
    </button>
    {#if ask.id}
      <button type="button" class="button" disabled={busy} onclick={() => void answer("cancel")}>
        {$t("llm.jobs.ask.stop")}
      </button>
    {/if}
  </div>
  {#if ask.id}
    <p class="hint">{$t("llm.jobs.ask.cli", { id: ask.id })}</p>
  {/if}
</div>

<style>
  .ask {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) var(--space-3);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--orange) 8%, transparent);
  }

  .ask-title {
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .pre {
    margin: 0;
    padding: var(--space-2);
    max-height: 200px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    user-select: text;
  }
</style>
