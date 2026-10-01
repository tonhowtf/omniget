<script lang="ts">
  import { t } from "$lib/i18n";
  import { agentsUsingSkill, type SkillManifest } from "$lib/stores/llm-skills-store.svelte";
  import SkillCard from "./SkillCard.svelte";

  let {
    skills = [],
    agents = [],
    busy = null,
    onremove,
  }: {
    skills?: SkillManifest[];
    agents?: { name: string; skills?: string[] }[];
    busy?: string | null;
    onremove?: (name: string) => void;
  } = $props();
</script>

{#if skills.length === 0}
  <p class="empty">{$t("llm.skills.empty")}</p>
{:else}
  <ul class="list">
    {#each skills as skill (skill.name)}
      <li>
        <SkillCard
          {skill}
          usedBy={agentsUsingSkill(agents, skill.name)}
          busy={busy === skill.name}
          {onremove}
        />
      </li>
    {/each}
  </ul>
{/if}

<style>
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--space-3);
  }

  .empty {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
