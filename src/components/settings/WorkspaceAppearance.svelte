<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { PRESETS, getWorkspaceDesign, saveWorkspaceDesign, type Preset, type Mode } from "$lib/stores/workspace-design.svelte";
  let current = $derived(getWorkspaceDesign());
  let preset = $state<Preset>(untrack(() => current.preset));
  let mode = $state<Mode>(untrack(() => current.mode));
  let notice = $state("");
  $effect(() => { preset = current.preset; mode = current.mode; });
  let previewMode = $derived(mode === "system" ? current.systemDark ? "dark" : "light" : mode);
  let changed = $derived(preset !== current.preset || mode !== current.mode);
  function apply() { notice = saveWorkspaceDesign(preset, mode) ? "design.applied" : "design.error"; }
</script>
<section class="workspace-appearance" data-setting-key="design.title">
  <h2>{$t("design.title")}</h2>
  <p>{$t("design.hint")}</p>
  <div class="presets">
    {#each PRESETS as item (item.id)}
      <button type="button" class="preset" aria-pressed={preset === item.id} onclick={() => { preset = item.id; notice = ""; }}>
        <span class="swatches" aria-hidden="true">{#each item.colors as color}<i style:background={color}></i>{/each}</span>
        <span>{item.name}</span>{#if preset === item.id}<span aria-hidden="true">✓</span>{/if}
      </button>
    {/each}
  </div>
  <div class="modes">
    {#each ["system", "light", "dark"] as item}<button type="button" class="button" aria-pressed={mode === item} data-setting-key={`design.${item}`} onclick={() => { mode = item as Mode; notice = ""; }}>{$t(`design.${item}`)}</button>{/each}
  </div>
  <div class="preview ds-scope" data-ds-preset={preset} data-ds-mode={previewMode}>
    <strong>{$t("design.preview")}</strong><p>{$t("design.preview_body")}</p>
    <span class="preview-action">{$t("llm.roster.new")}</span>
  </div>
  <div class="modes">
    <button type="button" class="button primary" disabled={!changed} onclick={apply}>{$t("design.apply")}</button>
    <button type="button" class="button" onclick={() => { preset = "amber-minimal"; mode = "system"; notice = ""; }}>{$t("design.reset")}</button>
  </div>
  {#if changed}<p role="status">{$t("settings.appearance.preview_pending")}</p>{/if}
  {#if notice}<p role={notice === "design.error" ? "alert" : "status"}>{$t(notice)}</p>{/if}
</section>
<style>
  .workspace-appearance { margin-bottom: var(--space-6); max-width: 800px; }
  h2 { font-size: var(--text-lg); } p { color: var(--text-muted); font-size: var(--text-sm); }
  .presets { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: var(--space-3); }
  .preset { display: flex; align-items: center; gap: var(--space-2); padding: var(--space-3); border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface); color: var(--text); text-align: left; cursor: pointer; }
  [aria-pressed="true"] { outline: 2px solid var(--accent-hi); outline-offset: 2px; }
  .swatches { display: flex; } i { width: 14px; height: 24px; border: 1px solid #8886; }
  .modes { display: flex; flex-wrap: wrap; gap: var(--space-3); margin-block: var(--space-4); }
  .preview { padding: var(--space-5); border: 1px solid var(--border); border-radius: var(--radius-md); }
  .modes .primary { background: var(--accent); color: var(--on-accent); }
  .preview-action { display: inline-block; background: var(--accent); color: var(--on-accent); padding: 8px 16px; border-radius: 6px; }
</style>
