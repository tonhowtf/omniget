<script lang="ts">
  /**
   * Estado ao vivo de uma sessão: "Trabalhando" (a vez é do agente),
   * "Esperando você" e "Parada". Cor só nos dois estados que pedem atenção,
   * sempre com rótulo; parada/sem estado é um ponto neutro.
   */
  import { t } from "$lib/i18n";
  import type { ActiveState } from "$lib/central/sessions";

  interface Props {
    state: ActiveState | null;
    pending?: boolean;
    compact?: boolean;
  }

  let { state, pending = false, compact = false }: Props = $props();
</script>

{#if state === "working" || state === "waiting"}
  <span class="badge-state {state}" class:compact title={pending ? $t("llm.central.sessions.pending_tool") : ""}>
    <i aria-hidden="true"></i>{#if !compact}<span>{$t(`llm.central.sessions.state.${state}`)}</span>{:else}<span class="sr">{$t(`llm.central.sessions.state.${state}`)}</span>{/if}
  </span>
{:else}
  <span class="badge-state idle" class:compact>
    <i aria-hidden="true"></i><span class="sr">{$t("llm.central.sessions.state.idle")}</span>
  </span>
{/if}

<style>
  .badge-state {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--text-xs);
    font-weight: 500;
    white-space: nowrap;
    min-width: 8px;
  }
  i {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .working {
    color: var(--accent-hi);
  }
  .working i {
    background: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
    animation: pulse 1.6s var(--ease-in-out) infinite;
  }
  .waiting {
    color: var(--text);
  }
  .waiting i {
    background: transparent;
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .idle i {
    background: var(--fill-3);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  @keyframes pulse {
    50% {
      box-shadow: 0 0 0 5px transparent;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .working i {
      animation: none;
    }
  }
</style>
