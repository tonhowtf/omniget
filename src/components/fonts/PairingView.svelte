<script lang="ts">
  import { t } from "$lib/i18n";
  import { ensureFont } from "$lib/fonts/loader";
  import { fontStack, snapWeight, type PreviewFont } from "$lib/fonts/preview";
  import type { Theme } from "$lib/fonts/sample";
  import FontPicker, { type PickerGroup } from "./FontPicker.svelte";

  let {
    heading,
    body,
    groups,
    theme,
    lang,
    onheading,
    onbody,
    onswap,
  }: {
    heading: PreviewFont | null;
    body: PreviewFont | null;
    groups: PickerGroup[];
    theme: Theme;
    lang: string;
    onheading: (key: string) => void;
    onbody: (key: string) => void;
    onswap: () => void;
  } = $props();

  let headingWeight = $state(700);
  let bodySize = $state(17);
  let hw = $derived(heading ? snapWeight(heading, headingWeight) : headingWeight);

  $effect(() => {
    void ensureFont(heading, hw, false, lang);
    void ensureFont(heading, 600, false, lang);
  });

  $effect(() => {
    void ensureFont(body, 400, false, lang);
    void ensureFont(body, 400, true, lang);
    void ensureFont(body, 600, false, lang);
  });

  let headStack = $derived(fontStack(heading));
  let bodyStack = $derived(fontStack(body));
</script>

<div class="ty-pair-ctl">
  <FontPicker value={heading?.key ?? null} {groups} label={$t("fonts.pair_heading") as string} onpick={onheading} />
  <button type="button" class="btn btn-ghost btn-sm ty-pair-swap" onclick={onswap} aria-label={$t("fonts.pair_swap") as string} title={$t("fonts.pair_swap") as string}>
    <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 5h9l-2.5-2.5M13 11H4l2.5 2.5" /></svg>
    {$t("fonts.pair_swap_short")}
  </button>
  <FontPicker value={body?.key ?? null} {groups} label={$t("fonts.pair_body") as string} onpick={onbody} />
  <label class="ty-pair-slider">
    <span class="field-label">{$t("fonts.pair_heading_weight")}</span>
    <input type="range" min="100" max="900" step="1" bind:value={headingWeight} />
    <output>{hw}</output>
  </label>
  <label class="ty-pair-slider">
    <span class="field-label">{$t("fonts.pair_body_size")}</span>
    <input type="range" min="12" max="24" step="1" bind:value={bodySize} />
    <output>{bodySize}px</output>
  </label>
</div>

<div class="ty-pair" data-theme={theme} style:--head={headStack} style:--body={bodyStack} style:--hw={hw} style:--bs="{bodySize}px">
  <article class="ty-pair-article">
    <p class="ty-pair-kicker">{$t("fonts.article_kicker")}</p>
    <h1>{$t("fonts.article_title")}</h1>
    <p class="ty-pair-lead">{$t("fonts.article_lead")}</p>
    <h2>{$t("fonts.article_h2")}</h2>
    <p>{$t("fonts.article_p1")}</p>
    <blockquote>{$t("fonts.article_quote")}</blockquote>
    <p>{$t("fonts.article_p2")}</p>
  </article>

  <aside class="ty-pair-ui" aria-label={$t("fonts.pair_ui_label") as string}>
    <p class="ty-pair-ui-title">{$t("fonts.ui_title")}</p>
    <p class="ty-pair-ui-body">{$t("fonts.ui_body")}</p>
    <div class="ty-pair-ui-field">
      <span class="ty-pair-ui-label">{$t("fonts.ui_label")}</span>
      <span class="ty-pair-ui-input">{$t("fonts.ui_value")}</span>
    </div>
    <div class="ty-pair-ui-buttons">
      <span class="ty-pair-ui-btn secondary">{$t("fonts.ui_cancel")}</span>
      <span class="ty-pair-ui-btn">{$t("fonts.ui_confirm")}</span>
    </div>
    <p class="ty-pair-ui-meta">{$t("fonts.ui_meta")}</p>
  </aside>
</div>

<style>
  .ty-pair-ctl {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: end;
    gap: var(--space-2) var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-pair-slider {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: var(--space-2);
    grid-column: span 1;
  }

  .ty-pair-slider:last-child {
    grid-column: 3;
  }

  .ty-pair-slider input {
    width: 100%;
    min-width: 0;
    accent-color: var(--accent);
  }

  .ty-pair-slider output {
    min-width: 4ch;
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .ty-pair-swap {
    margin-bottom: 0;
  }

  .ty-pair {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 280px;
    gap: var(--space-6);
    padding: var(--space-7) var(--space-6);
    border-radius: var(--radius-lg);
    background: var(--bg);
    color: var(--text);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  @media (max-width: 820px) {
    .ty-pair {
      grid-template-columns: minmax(0, 1fr);
    }

    .ty-pair-ctl {
      grid-template-columns: minmax(0, 1fr);
    }

    .ty-pair-slider:last-child {
      grid-column: auto;
    }
  }

  .ty-pair-article {
    max-width: 62ch;
    font-family: var(--body);
    font-size: var(--bs);
    line-height: 1.6;
  }

  .ty-pair-article h1,
  .ty-pair-article h2 {
    font-family: var(--head);
    font-weight: var(--hw);
    color: var(--text);
    letter-spacing: -0.015em;
  }

  .ty-pair-article h1 {
    margin: 0 0 0.4em;
    font-size: 2.6em;
    line-height: 1.08;
  }

  .ty-pair-article h2 {
    margin: 1.6em 0 0.5em;
    font-size: 1.45em;
    line-height: 1.2;
  }

  .ty-pair-article p {
    margin: 0 0 1em;
    color: var(--text-muted);
  }

  .ty-pair-article .ty-pair-kicker {
    margin-bottom: 0.8em;
    font-family: var(--head);
    font-size: 0.78em;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--accent-text);
  }

  .ty-pair-article .ty-pair-lead {
    font-size: 1.2em;
    line-height: 1.5;
    color: var(--text);
  }

  .ty-pair-article blockquote {
    margin: 1.4em 0;
    padding-left: 1em;
    border-left: 3px solid var(--accent);
    font-style: italic;
    color: var(--text);
  }

  .ty-pair-ui {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    align-self: start;
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-family: var(--body);
  }

  .ty-pair-ui p {
    margin: 0;
  }

  .ty-pair-ui-title {
    font-family: var(--head);
    font-size: 18px;
    font-weight: var(--hw);
    color: var(--text);
  }

  .ty-pair-ui-body,
  .ty-pair-ui-meta {
    font-size: 13px;
    line-height: 1.45;
    color: var(--text-muted);
  }

  .ty-pair-ui-meta {
    font-size: 11px;
    color: var(--text-dim);
  }

  .ty-pair-ui-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .ty-pair-ui-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }

  .ty-pair-ui-input {
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
    font-size: 13px;
    color: var(--text);
  }

  .ty-pair-ui-buttons {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .ty-pair-ui-btn {
    padding: 6px 12px;
    border-radius: var(--radius-sm);
    background: var(--cta);
    color: var(--on-cta);
    font-size: 13px;
    font-weight: 600;
  }

  .ty-pair-ui-btn.secondary {
    background: var(--fill-2);
    color: var(--text);
  }
</style>
