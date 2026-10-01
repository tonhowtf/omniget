<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";

  const KEY = "omniget.llm.routing_rule";
  const RULES = [
    { id: "cheap", title: "llm.models.rule_cheap", desc: "llm.models.rule_cheap_desc" },
    { id: "local", title: "llm.models.rule_local", desc: "llm.models.rule_local_desc" },
    { id: "quality", title: "llm.models.rule_quality", desc: "llm.models.rule_quality_desc" },
  ];

  let rule = $state("cheap");

  onMount(() => {
    try {
      const saved = localStorage.getItem(KEY);
      if (saved && RULES.some((r) => r.id === saved)) rule = saved;
    } catch {
    }
  });
</script>

<div class="group rules-group">
  <div class="group-label">{$t("llm.models.routing")}</div>
  <p class="group-footer" role="status">{$t("llm.models.routing_unavailable")}</p>
  {#each RULES as item (item.id)}
    <button
      type="button"
      class="group-row rule"
      disabled
      aria-pressed={rule === item.id}
    >
      <span class="group-row-content">
        <span class="group-row-title">{$t(item.title)}</span>
        <span class="group-row-sub">{$t(item.desc)}</span>
      </span>
      <span class="group-row-trailing">
        {#if rule === item.id}<span class="tag tag-accent">✓</span>{/if}
      </span>
    </button>
  {/each}
  <a class="group-footer" href="/llm/roster">{$t("llm.tab.roster")} →</a>
</div>

<style>
  .rules-group {
    overflow: hidden;
  }

  .rule {
    width: 100%;
    border: none;
    background: transparent;
    text-align: left;
    cursor: default;
    color: inherit;
  }

  .rule[aria-pressed="true"] {
    background: var(--accent-soft);
  }
</style>
