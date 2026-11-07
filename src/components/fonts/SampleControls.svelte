<script lang="ts">
  import { t } from "$lib/i18n";
  import { LIMITS, PRESETS, presetDefaults, trackingLabel, type PresetId, type Sample } from "$lib/fonts/sample";

  let {
    sample = $bindable(),
    placeholder,
    showText = true,
  }: { sample: Sample; placeholder: string; showText?: boolean } = $props();

  const PRESET_KEYS: Record<PresetId, string> = {
    pangram: "fonts.preset_pangram",
    paragraph: "fonts.preset_paragraph",
    ui: "fonts.preset_ui",
    code: "fonts.preset_code",
  };

  function pickPreset(preset: PresetId) {
    sample.preset = preset;
    sample.text = "";
    const d = presetDefaults(preset);
    sample.size = d.size;
    sample.lineHeight = d.lineHeight;
  }

  function reset() {
    sample.text = "";
    sample.weight = 400;
    sample.italic = false;
    sample.tracking = 0;
    const d = presetDefaults(sample.preset);
    sample.size = d.size;
    sample.lineHeight = d.lineHeight;
  }
</script>

<div class="ty-ctl">
  <div class="ty-ctl-row">
    <div class="segmented" role="group" aria-label={$t("fonts.preset_label") as string}>
      {#each PRESETS as preset (preset)}
        <button type="button" class="segmented-btn" class:active={sample.preset === preset} aria-pressed={sample.preset === preset} onclick={() => pickPreset(preset)}>
          {$t(PRESET_KEYS[preset])}
        </button>
      {/each}
    </div>
    <div class="segmented" role="group" aria-label={$t("fonts.theme_label") as string}>
      <button type="button" class="segmented-btn" class:active={sample.theme === "dark"} aria-pressed={sample.theme === "dark"} onclick={() => (sample.theme = "dark")}>{$t("fonts.theme_dark")}</button>
      <button type="button" class="segmented-btn" class:active={sample.theme === "light"} aria-pressed={sample.theme === "light"} onclick={() => (sample.theme = "light")}>{$t("fonts.theme_light")}</button>
    </div>
    <button type="button" class="btn btn-ghost btn-sm ty-ctl-reset" onclick={reset}>{$t("fonts.reset_sample")}</button>
  </div>

  {#if showText}
    <label class="ty-ctl-text">
      <span class="field-label">{$t("fonts.sample_text")}</span>
      <textarea class="input ty-ctl-area" rows="2" spellcheck="false" {placeholder} bind:value={sample.text}></textarea>
    </label>
  {/if}

  <div class="ty-ctl-sliders">
    <label class="ty-ctl-slider">
      <span class="field-label">{$t("fonts.size")}</span>
      <input type="range" min={LIMITS.size.min} max={LIMITS.size.max} step={LIMITS.size.step} bind:value={sample.size} />
      <output class="ty-ctl-num">{sample.size}px</output>
    </label>
    <label class="ty-ctl-slider">
      <span class="field-label">{$t("fonts.weight")}</span>
      <input type="range" min={LIMITS.weight.min} max={LIMITS.weight.max} step={LIMITS.weight.step} bind:value={sample.weight} />
      <output class="ty-ctl-num">{sample.weight}</output>
    </label>
    <label class="ty-ctl-slider">
      <span class="field-label">{$t("fonts.line_height")}</span>
      <input type="range" min={LIMITS.lineHeight.min} max={LIMITS.lineHeight.max} step={LIMITS.lineHeight.step} bind:value={sample.lineHeight} />
      <output class="ty-ctl-num">{sample.lineHeight.toFixed(2)}</output>
    </label>
    <label class="ty-ctl-slider">
      <span class="field-label">{$t("fonts.tracking")}</span>
      <input type="range" min={LIMITS.tracking.min} max={LIMITS.tracking.max} step={LIMITS.tracking.step} bind:value={sample.tracking} />
      <output class="ty-ctl-num">{trackingLabel(sample.tracking)}</output>
    </label>
    <label class="ty-ctl-check">
      <input type="checkbox" class="checkbox" bind:checked={sample.italic} />
      <span class="field-label">{$t("fonts.italic")}</span>
    </label>
  </div>
</div>

<style>
  .ty-ctl {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-ctl-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
  }

  .ty-ctl-reset {
    margin-left: auto;
  }

  .ty-ctl-text {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .ty-ctl-area {
    height: auto;
    min-height: 48px;
    padding-block: var(--space-2);
    resize: vertical;
    font-family: inherit;
    line-height: var(--leading-base, 1.4);
  }

  .ty-ctl-sliders {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: var(--space-2) var(--space-4);
    align-items: center;
  }

  .ty-ctl-slider {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .ty-ctl-slider input {
    min-width: 0;
    width: 100%;
    accent-color: var(--accent);
  }

  .ty-ctl-num {
    min-width: 6ch;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    text-align: right;
  }

  .ty-ctl-check {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }
</style>
