<script lang="ts">
  import { t } from "$lib/i18n";

  let {
    label,
    value = $bindable(),
    swatches,
    disabled = false,
  }: { label: string; value: string; swatches: string[]; disabled?: boolean } = $props();

  const id = `sc-${Math.random().toString(36).slice(2, 9)}`;
</script>

<div class="sc" role="group" aria-labelledby="{id}-label">
  <span class="field-label" id="{id}-label">{label}</span>
  <div class="sc-row">
    {#each swatches as swatch (swatch)}
      <button
        type="button"
        class="sc-swatch"
        class:active={swatch.toUpperCase() === value.toUpperCase()}
        style:background={swatch}
        aria-label={$t("studio.color_swatch", { color: swatch })}
        aria-pressed={swatch.toUpperCase() === value.toUpperCase()}
        {disabled}
        onclick={() => (value = swatch)}
      ></button>
    {/each}
    <label class="sc-custom">
      <input type="color" bind:value {disabled} aria-label={$t("studio.color_custom", { label })} />
      <span class="sc-hex">{value.toUpperCase()}</span>
    </label>
  </div>
</div>

<style>
  .sc {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .sc-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .sc-swatch {
    width: 20px;
    height: 20px;
    padding: 0;
    border: none;
    border-radius: var(--radius-full);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    cursor: pointer;
  }

  .sc-swatch.active {
    box-shadow:
      inset 0 0 0 var(--hairline) var(--content-border),
      0 0 0 2px var(--pane-bg),
      0 0 0 4px var(--accent);
  }

  .sc-swatch:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 3px;
  }

  .sc-swatch:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .sc-custom {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin-left: var(--space-1);
  }

  .sc-custom input {
    width: 28px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: var(--radius-xs);
    background: none;
    cursor: pointer;
  }

  .sc-hex {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
