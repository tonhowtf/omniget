<script lang="ts">
  import { t } from "$lib/i18n";
  import { categoryKey } from "$lib/fonts/catalog";
  import { MAX_COMPARE, type PreviewFont } from "$lib/fonts/preview";
  import { GLYPHS, type Sample } from "$lib/fonts/sample";
  import Specimen from "./Specimen.svelte";
  import FontPicker, { type PickerGroup } from "./FontPicker.svelte";

  let {
    fonts,
    sample,
    text,
    lang,
    groups,
    onadd,
    onremove,
    onbrowse,
  }: {
    fonts: PreviewFont[];
    sample: Sample;
    text: string;
    lang: string;
    groups: PickerGroup[];
    onadd: (key: string) => void;
    onremove: (key: string) => void;
    onbrowse: () => void;
  } = $props();
</script>

{#if fonts.length === 0}
  <div class="ty-empty">
    <h2>{$t("fonts.compare_empty_title")}</h2>
    <p>{$t("fonts.compare_empty_body")}</p>
    <div class="ty-empty-actions">
      <button type="button" class="btn btn-primary" onclick={onbrowse}>{$t("fonts.browse_fonts")}</button>
    </div>
  </div>
{:else}
  <div class="ty-cmp" style:--cols={fonts.length}>
    {#each fonts as font (font.key)}
      <section class="ty-cmp-col" aria-label={font.family}>
        <header class="ty-cmp-head">
          <div class="ty-cmp-name">
            <h2>{font.family}</h2>
            <p>{font.category ? $t(categoryKey(font.category)) : ""}{font.variable ? ` · ${$t("fonts.variable")}` : ""}</p>
          </div>
          <button type="button" class="btn-icon btn-sm" aria-label={$t("fonts.compare_remove", { name: font.family }) as string} title={$t("fonts.compare_remove", { name: font.family }) as string} onclick={() => onremove(font.key)}>
            <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="m4 4 8 8M12 4l-8 8" /></svg>
          </button>
        </header>
        <div class="ty-cmp-paper" data-theme={sample.theme}>
          <Specimen {font} {text} size={sample.size} weight={sample.weight} italic={sample.italic} lineHeight={sample.lineHeight} tracking={sample.tracking} {lang} multiline={sample.preset !== "pangram"} />
          <Specimen {font} text={GLYPHS} size={15} weight={sample.weight} italic={sample.italic} lineHeight={1.5} {lang} />
        </div>
      </section>
    {/each}
  </div>
  {#if fonts.length < MAX_COMPARE}
    <div class="ty-cmp-add">
      <FontPicker value={null} {groups} label={$t("fonts.compare_add_label") as string} placeholder={$t("fonts.compare_pick") as string} onpick={onadd} />
    </div>
  {:else}
    <p class="ty-cmp-hint">{$t("fonts.compare_full")}</p>
  {/if}
{/if}

<style>
  .ty-cmp {
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    gap: var(--space-3);
  }

  @media (max-width: 760px) {
    .ty-cmp {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .ty-cmp-col {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .ty-cmp-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .ty-cmp-name {
    min-width: 0;
  }

  .ty-cmp-name h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .ty-cmp-name p {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .ty-cmp-paper {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    flex: 1;
    padding: var(--space-5) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--bg);
    color: var(--text);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-cmp-add {
    max-width: 320px;
  }

  .ty-cmp-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .ty-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8) var(--space-5);
    text-align: center;
    border: 1.5px dashed var(--content-border);
    border-radius: var(--radius-xl);
    background: var(--surface);
  }

  .ty-empty h2 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .ty-empty p {
    margin: 0;
    max-width: 420px;
    font-size: var(--text-base);
    color: var(--text-dim);
  }

  .ty-empty-actions {
    margin-top: var(--space-2);
  }
</style>
