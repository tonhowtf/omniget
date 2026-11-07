<script lang="ts">
  import { pluralKey } from "$lib/fonts/text";
  import { t } from "$lib/i18n";
  import * as api from "$lib/fonts/api";
  import { categoryKey, curatedBy } from "$lib/fonts/catalog";
  import type { PreviewFont } from "$lib/fonts/preview";
  import { GLYPHS, type Sample } from "$lib/fonts/sample";
  import type { CatalogFont, InstalledFamily } from "$lib/fonts/types";
  import Specimen from "./Specimen.svelte";
  import InstallPanel from "./InstallPanel.svelte";

  let {
    font,
    catalogFont,
    installedFamily,
    alreadyInstalled,
    sample,
    text,
    lang,
    userDir,
    inCompare,
    compareFull,
    isHeading,
    isBody,
    oncompare,
    onheading,
    onbody,
    onscale,
    oninstalled,
  }: {
    font: PreviewFont;
    catalogFont: CatalogFont | null;
    installedFamily: InstalledFamily | null;
    alreadyInstalled: boolean;
    sample: Sample;
    text: string;
    lang: string;
    userDir: string | null;
    inCompare: boolean;
    compareFull: boolean;
    isHeading: boolean;
    isBody: boolean;
    oncompare: () => void;
    onheading: () => void;
    onbody: () => void;
    onscale: () => void;
    oninstalled: () => void;
  } = $props();

  let by = $derived(font.id ? curatedBy(font.id) : null);
  let waterfall = $derived(font.weights.slice(0, 9));
  let fileCount = $derived(installedFamily?.files.length ?? 0);

  function showFiles() {
    const first = installedFamily?.files[0];
    if (first) void api.reveal(first).catch(() => {});
  }
</script>

<article class="ty-detail" aria-labelledby="ty-detail-title">
  <header class="ty-detail-head">
    <div class="ty-detail-copy">
      <h2 id="ty-detail-title">{font.family}</h2>
      <p class="ty-detail-meta">
        {#if font.category}<span>{$t(categoryKey(font.category))}</span>{/if}
        <span>{$t(pluralKey("fonts.weights_count", font.weights.length), { count: font.weights.length })}</span>
        {#if font.variable}<span class="tag tag-info">{$t("fonts.variable")}</span>{/if}
        {#if font.italic}<span>{$t("fonts.has_italic")}</span>{/if}
        {#if by}<span>{$t("fonts.by", { name: by })}</span>{/if}
        {#if font.source === "installed"}<span class="tag tag-success">{$t("fonts.installed_tag")}</span>{/if}
      </p>
    </div>
    <div class="ty-detail-actions">
      <button type="button" class="btn btn-secondary btn-sm" aria-pressed={inCompare} disabled={!inCompare && compareFull} title={!inCompare && compareFull ? ($t("fonts.compare_full") as string) : undefined} onclick={oncompare}>
        {inCompare ? $t("fonts.in_compare") : $t("fonts.add_compare")}
      </button>
      <button type="button" class="btn btn-ghost btn-sm" aria-pressed={isHeading} onclick={onheading}>{$t("fonts.use_heading")}</button>
      <button type="button" class="btn btn-ghost btn-sm" aria-pressed={isBody} onclick={onbody}>{$t("fonts.use_body")}</button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={onscale}>{$t("fonts.use_scale")}</button>
    </div>
  </header>

  <div class="ty-paper" data-theme={sample.theme}>
    <Specimen
      {font}
      {text}
      size={sample.size}
      weight={sample.weight}
      italic={sample.italic}
      lineHeight={sample.lineHeight}
      tracking={sample.tracking}
      {lang}
      multiline={sample.preset !== "pangram"}
      label={$t("fonts.specimen_label", { name: font.family }) as string}
    />
  </div>

  <section class="ty-detail-block" aria-label={$t("fonts.weights_title") as string}>
    <h3 class="ty-detail-title">{$t("fonts.weights_title")}</h3>
    <ul class="ty-water" data-theme={sample.theme}>
      {#each waterfall as w (w)}
        <li>
          <span class="ty-water-w">{w}</span>
          <Specimen {font} text={GLYPHS} size={20} weight={w} italic={sample.italic} lineHeight={1.3} {lang} />
        </li>
      {/each}
    </ul>
  </section>

  {#if catalogFont}
    <InstallPanel font={catalogFont} {alreadyInstalled} {userDir} {oninstalled} />
  {:else if installedFamily}
    <section class="ty-detail-block ty-detail-files">
      <h3 class="ty-detail-title">{$t("fonts.on_computer_title")}</h3>
      <p class="ty-detail-note">
        {$t(pluralKey("fonts.files_count", fileCount), { count: fileCount })}
        {#if installedFamily.user_installed}· {$t("fonts.user_installed")}{/if}
      </p>
      <ul class="ty-styles">
        {#each installedFamily.styles as s (s.path + s.index + s.name)}
          <li><span>{s.name}</span><span class="ty-styles-w">{s.weight}{s.italic ? " · " + $t("fonts.italic") : ""}</span></li>
        {/each}
      </ul>
      {#if installedFamily.axes.length > 0}
        <p class="ty-detail-note">{$t("fonts.axes")}: {installedFamily.axes.map((a) => `${a.tag} ${a.min}–${a.max}`).join(", ")}</p>
      {/if}
      <div class="ty-detail-row">
        <button type="button" class="btn btn-secondary btn-sm" disabled={fileCount === 0} onclick={showFiles}>{$t("fonts.show_in_folder")}</button>
      </div>
    </section>
  {/if}
</article>

<style>
  .ty-detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  .ty-detail-head {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .ty-detail-copy {
    min-width: 0;
  }

  .ty-detail-copy h2 {
    margin: 0 0 var(--space-1);
    font-size: var(--text-xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .ty-detail-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-3);
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .ty-detail-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .ty-paper {
    padding: var(--space-6) var(--space-5);
    border-radius: var(--radius-lg);
    background: var(--bg);
    color: var(--text);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    min-height: 160px;
  }

  .ty-detail-block {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .ty-detail-title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .ty-water {
    margin: 0;
    padding: var(--space-2) var(--space-4);
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-water li {
    display: grid;
    grid-template-columns: 3.5ch 1fr;
    align-items: baseline;
    gap: var(--space-3);
    padding: var(--space-1) 0;
    white-space: nowrap;
    overflow: hidden;
  }

  .ty-water li :global(.ty-spec-text) {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .ty-water-w {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }

  .ty-detail-files {
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-detail-note {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .ty-styles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-1) var(--space-4);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .ty-styles li {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .ty-styles-w {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .ty-detail-row {
    display: flex;
    justify-content: flex-end;
  }
</style>
