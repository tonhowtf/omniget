<script lang="ts" module>
  export type ConverterOpts = {
    pretty: boolean;
    sortKeys: boolean;
    quality: number;
    maxWidth: number | null;
    maxHeight: number | null;
    outDir: string | null;
    overwrite: boolean;
  };
</script>

<script lang="ts">
  import { t } from "$lib/i18n";

  let {
    opts = $bindable(),
    showData,
    showImage,
    showQuality,
    disabled = false,
    onchoosefolder,
  }: {
    opts: ConverterOpts;
    showData: boolean;
    showImage: boolean;
    showQuality: boolean;
    disabled?: boolean;
    onchoosefolder: () => void;
  } = $props();
</script>

<details class="cv-opts">
  <summary class="cv-opts-summary">
    <svg class="cv-opts-chev" viewBox="0 0 12 12" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m4.5 2.5 3.5 3.5-3.5 3.5" /></svg>
    {$t("converter.options")}
  </summary>

  <fieldset class="cv-opts-body" {disabled}>
    {#if showData}
      <section class="cv-opts-section">
        <h3 class="cv-opts-title">{$t("converter.opt_data")}</h3>
        <label class="cv-opts-check">
          <input type="checkbox" class="checkbox" bind:checked={opts.pretty} />
          <span>
            <span class="field-label">{$t("converter.opt_pretty")}</span>
            <span class="field-hint">{$t("converter.opt_pretty_hint")}</span>
          </span>
        </label>
        <label class="cv-opts-check">
          <input type="checkbox" class="checkbox" bind:checked={opts.sortKeys} />
          <span>
            <span class="field-label">{$t("converter.opt_sort")}</span>
            <span class="field-hint">{$t("converter.opt_sort_hint")}</span>
          </span>
        </label>
      </section>
    {/if}

    {#if showImage}
      <section class="cv-opts-section">
        <h3 class="cv-opts-title">{$t("converter.opt_images")}</h3>
        {#if showQuality}
          <label class="cv-opts-field">
            <span class="field-label">{$t("converter.opt_quality")}</span>
            <span class="cv-opts-range">
              <input type="range" min="40" max="100" step="1" bind:value={opts.quality} />
              <output class="cv-opts-num">{opts.quality}</output>
            </span>
          </label>
        {/if}
        <div class="cv-opts-size">
          <label class="cv-opts-field">
            <span class="field-label">{$t("converter.opt_max_width")}</span>
            <span class="cv-opts-px">
              <input class="input cv-opts-input" type="number" min="1" inputmode="numeric" bind:value={opts.maxWidth} />
              <span class="cv-opts-unit">{$t("converter.opt_px")}</span>
            </span>
          </label>
          <label class="cv-opts-field">
            <span class="field-label">{$t("converter.opt_max_height")}</span>
            <span class="cv-opts-px">
              <input class="input cv-opts-input" type="number" min="1" inputmode="numeric" bind:value={opts.maxHeight} />
              <span class="cv-opts-unit">{$t("converter.opt_px")}</span>
            </span>
          </label>
        </div>
        <p class="field-hint cv-opts-hint">{$t("converter.opt_max_hint")}</p>
      </section>
    {/if}

    <section class="cv-opts-section">
      <h3 class="cv-opts-title">{$t("converter.opt_output")}</h3>
      <div class="cv-opts-folder">
        <span class="cv-opts-path" title={opts.outDir ?? undefined}>{opts.outDir ?? $t("converter.opt_output_default")}</span>
        <button type="button" class="btn btn-secondary btn-sm" onclick={onchoosefolder}>{$t("converter.opt_choose_folder")}</button>
        {#if opts.outDir}
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => (opts.outDir = null)}>{$t("converter.opt_reset")}</button>
        {/if}
      </div>
      <label class="cv-opts-check">
        <input type="checkbox" class="checkbox" bind:checked={opts.overwrite} />
        <span>
          <span class="field-label">{$t("converter.opt_overwrite")}</span>
          <span class="field-hint">{$t("converter.opt_overwrite_hint")}</span>
        </span>
      </label>
    </section>
  </fieldset>
</details>

<style>
  .cv-opts {
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cv-opts-summary {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
    cursor: pointer;
    list-style: none;
    border-radius: var(--radius-lg);
  }

  .cv-opts-summary::-webkit-details-marker {
    display: none;
  }

  .cv-opts-summary:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .cv-opts-chev {
    color: var(--text-dim);
    transition: transform var(--duration-fast) var(--ease-out);
  }

  .cv-opts[open] .cv-opts-chev {
    transform: rotate(90deg);
  }

  .cv-opts-body {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: var(--space-5);
    margin: 0;
    padding: 0 var(--space-4) var(--space-4);
    border: none;
    min-width: 0;
  }

  .cv-opts-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .cv-opts-title {
    margin: 0;
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .cv-opts-check {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    cursor: pointer;
  }

  .cv-opts-check > span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .cv-opts-check input {
    margin-top: 2px;
    flex-shrink: 0;
  }

  .cv-opts-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .cv-opts-range {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .cv-opts-range input {
    flex: 1;
    accent-color: var(--accent);
  }

  .cv-opts-num {
    min-width: 3ch;
    text-align: end;
    font-variant-numeric: tabular-nums;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .cv-opts-size {
    display: flex;
    gap: var(--space-3);
  }

  .cv-opts-px {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .cv-opts-input {
    width: 90px;
  }

  .cv-opts-unit {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .cv-opts-hint {
    margin: 0;
  }

  .cv-opts-folder {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .cv-opts-path {
    flex: 1 1 100%;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
    color: var(--text);
  }
</style>
