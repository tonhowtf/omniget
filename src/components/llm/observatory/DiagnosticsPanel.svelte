<script lang="ts">
  import { t } from "$lib/i18n";
  import type { AgentTelemetry, LoadState } from "$lib/stores/llm-telemetry-store.svelte";

  interface Props {
    agents: AgentTelemetry[];
    state: LoadState;
    errorCode: string;
    tsMs: number;
    demo: boolean;
    busy: boolean;
    onRefresh: () => void;
    onCancel: (requestId: string) => void;
    onReset: (agentId: string) => void;
  }

  let { agents, state, errorCode, tsMs, demo, busy, onRefresh, onCancel, onReset }: Props = $props();

  function clock(ms: number): string {
    if (!ms) return "—";
    return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  }
</script>

<div class="group">
  <div class="group-row">
    <div class="group-row-content">
      <div class="group-row-title">{$t("llm.observatory.source")}</div>
      <div class="group-row-sub">
        {#if demo}
          {$t("llm.observatory.source_demo")}
        {:else if state === "unavailable"}
          {$t("llm.observatory.source_unavailable")} <code>{errorCode}</code>
        {:else}
          {$t("llm.observatory.source_live")} · {clock(tsMs)}
        {/if}
      </div>
    </div>
    <div class="group-row-trailing">
      <button class="btn btn-secondary btn-sm" type="button" disabled={busy || demo} onclick={onRefresh}>
        {$t("llm.observatory.refresh")}
      </button>
    </div>
  </div>

  {#each agents as a (a.agent_id)}
    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-title">{a.agent_name}</div>
        <div class="group-row-sub">
          {$t(`llm.observatory.state_${a.state}`)} · {a.turns} {$t("llm.observatory.turns")} · {a.errors}
          {$t("llm.observatory.errors")}{#if a.last_error_code} · <code>{a.last_error_code}</code>{/if}
        </div>
      </div>
      <div class="group-row-trailing btn-row">
        {#if a.request_id}
          <button class="btn btn-destructive btn-sm" type="button" onclick={() => onCancel(a.request_id as string)}>
            {$t("llm.observatory.cancel_turn")}
          </button>
        {/if}
        <button class="btn btn-ghost btn-sm" type="button" onclick={() => onReset(a.agent_id)}>
          {$t("llm.observatory.reset_agent")}
        </button>
      </div>
    </div>
  {/each}
</div>

<style>
  code {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
  }
</style>
