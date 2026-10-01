<script lang="ts">
  import { t } from "$lib/i18n";
  import TgIcon from "./TgIcon.svelte";
  import { MEDIA_TYPES } from "./api";
  import type { FilterForm } from "./filters";

  let { form = $bindable() }: { form: FilterForm } = $props();

  function toggleType(k: string) {
    form.types = form.types.includes(k) ? form.types.filter((x) => x !== k) : [...form.types, k];
  }
</script>

<div class="mf">
  <div class="field">
    <span class="field-label">{$t("telegram.filters.types")}</span>
    <div class="chips" role="group" aria-label={$t("telegram.filters.types")}>
      {#each MEDIA_TYPES as k (k)}
        <button type="button" class="chip" class:on={form.types.includes(k)} aria-pressed={form.types.includes(k)} onclick={() => toggleType(k)}>
          {#if form.types.includes(k)}<TgIcon name="check" size={11} />{/if}
          {$t(`telegram.filters.type.${k}`)}
        </button>
      {/each}
    </div>
    {#if !form.types.length}<span class="field-hint">{$t("telegram.filters.types_all")}</span>{/if}
  </div>

  <div class="row2">
    <label class="field">
      <span class="field-label">{$t("telegram.filters.min_size")}</span>
      <span class="unit"><input class="input" inputmode="decimal" bind:value={form.minMb} placeholder="0" /><span>MB</span></span>
    </label>
    <label class="field">
      <span class="field-label">{$t("telegram.filters.max_size")}</span>
      <span class="unit"><input class="input" inputmode="decimal" bind:value={form.maxMb} placeholder="∞" /><span>MB</span></span>
    </label>
  </div>

  <label class="field">
    <span class="field-label">{$t("telegram.filters.keyword")}</span>
    <input class="input" bind:value={form.keyword} placeholder={$t("telegram.filters.keyword_hint")} />
  </label>

  <div class="row2">
    <label class="field">
      <span class="field-label">{$t("telegram.filters.ext_include")}</span>
      <input class="input" bind:value={form.extIn} placeholder="mp4, mkv" />
    </label>
    <label class="field">
      <span class="field-label">{$t("telegram.filters.ext_exclude")}</span>
      <input class="input" bind:value={form.extOut} placeholder="webp, gif" />
    </label>
  </div>
</div>

<style>
  .mf {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text-muted);
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
  }

  .chip:hover {
    background: var(--fill-2);
  }

  .chip.on {
    background: var(--accent-soft);
    color: var(--accent-hi);
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--accent) 40%, transparent);
  }

  .chip:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .row2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }

  .unit {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .unit .input {
    flex: 1;
    min-width: 0;
  }

  .unit span {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  @media (max-width: 520px) {
    .row2 {
      grid-template-columns: 1fr;
    }
  }
</style>
