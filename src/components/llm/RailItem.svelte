<script lang="ts">

  import RailAvatar from "$components/omni/RailAvatar.svelte";
  import { t } from "$lib/i18n";
  import { agentTint, roleLabelKey, roleText, type AgentDef } from "$lib/llm/types";

  let {
    agent,
    active = false,
    speaking = false,
    preview = "",
    onselect,
  }: {
    agent: AgentDef;
    active?: boolean;
    speaking?: boolean;
    preview?: string;
    onselect: (id: string) => void;
  } = $props();

  let tint = $derived(agentTint(agent));
  let roleLabel = $derived(roleText(agent.role) ?? $t(roleLabelKey(agent.role)));
</script>

<button
  type="button"
  class="rail-item"
  class:active
  aria-current={active ? "true" : undefined}
  onclick={() => onselect(agent.id)}
>
  <RailAvatar {tint} size={36} {active} {speaking} />
  <span class="rail-item-body">
    <span class="rail-item-head">
      <span class="rail-item-name">{agent.name}</span>

    </span>
    <span class="rail-item-preview">{speaking ? $t("llm.surface.working") : preview || (typeof agent.role === "string" && ["worker", "coordinator", "advisor"].includes(agent.role) ? $t(`llm.surface.${agent.role}`) : roleLabel)}</span>
  </span>
</button>

<style>
  .rail-item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    height: 56px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .rail-item:hover {
      background: var(--fill-2);
    }
  }

  .rail-item.active {
    background: var(--accent-soft);
  }

  .rail-item-body {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
  }

  .rail-item-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .rail-item-name {
    font-size: var(--text-sm);
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .rail-item-preview {
    font-size: var(--text-caption);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
