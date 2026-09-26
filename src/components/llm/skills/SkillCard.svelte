<script lang="ts">
  /**
   * One installed skill: name, description, origin, the tools its frontmatter
   * allows and the agents whose roster entry lists it. Read-only except for
   * the remove button.
   */
  import { t } from "$lib/i18n";
  import {
    sourceDetail,
    sourceLabelKey,
    type SkillManifest,
  } from "$lib/stores/llm-skills-store.svelte";
  import SkillScanBadge from "./SkillScanBadge.svelte";
  import SkillVersionStatus from "./SkillVersionStatus.svelte";

  let {
    skill,
    usedBy = [],
    busy = false,
    onremove,
  }: {
    skill: SkillManifest;
    /** Names of the agents that have this skill active. */
    usedBy?: string[];
    busy?: boolean;
    onremove?: (name: string) => void;
  } = $props();

  let tools = $derived(skill.allowed_tools ?? []);
  let detail = $derived(sourceDetail(skill.source));
</script>

<article class="skill">
  <header class="head">
    <div class="titles">
      <h3 class="name">{skill.name}</h3>
      <span class="badge">{$t(sourceLabelKey(skill.source))}</span>
      {#if skill.license}
        <span class="chip">{skill.license}</span>
      {:else}
        <!-- Plan D-5: a skill whose SKILL.md declares no licence says so. -->
        <span class="badge warn" title={$t("llm.skills.catalog.unlicensed")}>
          {$t("llm.skills.unlicensed_badge")}
        </span>
      {/if}
      <!-- What the security scan said at install time. "Not scanned" when no
           scanner was there: never a claim that the skill is safe. -->
      <SkillScanBadge scan={skill.scan} />
    </div>
    {#if onremove}
      <button
        type="button"
        class="button danger"
        disabled={busy}
        onclick={() => onremove?.(skill.name)}
      >
        {$t("llm.skills.remove")}
      </button>
    {/if}
  </header>
  <SkillVersionStatus name={skill.name} />

  <a href="/llm/roster">{$t("llm.skills.assign")} →</a>
  <p class="description">{skill.description}</p>

  {#if detail || skill.path}
    <p class="path mono" title={detail || skill.path}>{detail || skill.path}</p>
  {/if}

  <dl class="meta">
    <div class="meta-row">
      <dt>{$t("llm.skills.allowed_tools")}</dt>
      <dd>
        {#if tools.length === 0}
          <span class="dim">{$t("llm.skills.allowed_tools_any")}</span>
        {:else}
          <ul class="chips">
            {#each tools as tool (tool)}
              <li class="chip mono">{tool}</li>
            {/each}
          </ul>
        {/if}
      </dd>
    </div>
    <div class="meta-row">
      <dt>{$t("llm.skills.used_by")}</dt>
      <dd>
        {#if usedBy.length === 0}
          <span class="dim">{$t("llm.skills.used_by_none")}</span>
        {:else}
          <ul class="chips">
            {#each usedBy as agent (agent)}
              <li class="chip">{agent}</li>
            {/each}
          </ul>
        {/if}
      </dd>
    </div>
  </dl>
</article>

<style>
  .skill {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .titles {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    flex-wrap: wrap;
  }

  .name {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .badge {
    padding: 1px var(--space-2);
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    color: var(--accent-hi);
    font-size: var(--text-xs);
    font-weight: 500;
    white-space: nowrap;
  }

  .badge.warn {
    background: color-mix(in srgb, var(--warning, var(--danger)) 16%, transparent);
    color: var(--warning, var(--danger));
  }

  .description {
    margin: 0;
    font-size: var(--text-sm);
    line-height: var(--leading-base);
    color: var(--text-muted);
  }

  .path {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }

  .meta {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .meta-row {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  dt {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-dim);
  }

  dd {
    margin: 0;
    min-width: 0;
  }

  .chips {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .chip {
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
    border: var(--hairline) solid var(--separator);
    font-size: var(--text-xs);
  }

  .mono {
    font-family: var(--font-mono);
  }

  .dim {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
