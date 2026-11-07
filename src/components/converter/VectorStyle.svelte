<script lang="ts">
  import { t } from "$lib/i18n";
  import { VECTOR_PRESETS } from "$lib/converter/plan";
  import type { VectorPreset } from "$lib/converter/types";

  let { value = $bindable(), disabled = false }: { value: VectorPreset; disabled?: boolean } = $props();
</script>

<fieldset class="cv-vector" {disabled}>
  <legend class="cv-vector-title">{$t("converter.vector_title")}</legend>
  <div class="cv-vector-options" role="radiogroup" aria-label={$t("converter.vector_title") as string}>
    {#each VECTOR_PRESETS as preset (preset)}
      <label class="cv-vector-option" class:on={value === preset}>
        <input type="radio" class="radio" name="cv-vector" value={preset} bind:group={value} />
        <span>
          <span class="field-label">{$t(`converter.vector_${preset}`)}</span>
          <span class="field-hint">{$t(`converter.vector_${preset}_hint`)}</span>
        </span>
      </label>
    {/each}
  </div>
</fieldset>

<style>
  .cv-vector {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    border: 0;
    min-width: 0;
  }

  .cv-vector-title {
    padding: 0;
    margin-bottom: var(--space-2);
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-dim);
  }

  .cv-vector-options {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: var(--space-2);
  }

  .cv-vector-option {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    cursor: pointer;
  }

  .cv-vector-option.on {
    box-shadow: inset 0 0 0 calc(var(--hairline) * 2) var(--accent);
  }

  .cv-vector-option > span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .cv-vector:disabled .cv-vector-option {
    cursor: default;
    opacity: 0.6;
  }
</style>
