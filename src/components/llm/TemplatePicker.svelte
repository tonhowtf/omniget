<script lang="ts">
  import { t } from "$lib/i18n";

  let {
    onapply,
    disabled = false,
  }: { onapply: (template: string) => void; disabled?: boolean } = $props();

  let preview = $state<string | null>(null);
  const TEMPLATES = [
    { id: "solo", title: "llm.template.solo", desc: "llm.template.solo_desc" },
    { id: "duo", title: "llm.template.duo", desc: "llm.template.duo_desc" },
    { id: "research", title: "llm.template.research", desc: "llm.template.research_desc" },
  ];
</script>

<div class="group">
  <div class="group-label">{$t("llm.roster.templates")}</div>
  {#each TEMPLATES as template (template.id)}
    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-title">{$t(template.title)}</div>
        <div class="group-row-sub">{$t(template.desc)}</div>
      </div>
      <div class="group-row-trailing">
        <button type="button" class="button" {disabled} onclick={() => preview = preview === template.id ? null : template.id} aria-expanded={preview === template.id}>
          {$t("llm.template.preview")}
        </button>
      </div>
    </div>
    {#if preview === template.id}
      <div class="group-footer">
        <p>{$t(template.desc)}</p><p>{$t("llm.template.effect")}</p>
        <button type="button" class="button primary" {disabled} onclick={() => onapply(template.id)}>{$t("llm.roster.apply")}</button>
      </div>
    {/if}
  {/each}
</div>
