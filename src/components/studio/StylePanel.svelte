<script lang="ts">
  import { t } from "$lib/i18n";
  import { FONTS, PRESET_IDS, applyPreset } from "$lib/studio/presets";
  import { cssFontStack } from "$lib/studio/fonts";
  import type { Align, CaptionStyle, Position, PresetId } from "$lib/studio/types";
  import { radioKeydown, rovingIndex } from "$lib/studio/radio";
  import ColorField from "./ColorField.svelte";

  let {
    style = $bindable(),
    swatches,
    disabled = false,
  }: { style: CaptionStyle; swatches: string[]; disabled?: boolean } = $props();

  const POSITIONS: Position[] = ["bottom", "middle", "top"];
  const ALIGNS: Align[] = ["center", "left"];
  const LINES: (1 | 2)[] = [1, 2];

  function pick(id: PresetId) {
    style = applyPreset(id);
  }
</script>

<fieldset class="sp" {disabled}>
  <legend class="sp-legend">{$t("studio.style_title")}</legend>

  <div class="sp-presets" role="radiogroup" aria-label={$t("studio.presets")}>
    {#each PRESET_IDS as id, i (id)}
      {@const p = applyPreset(id)}
      <button
        type="button"
        role="radio"
        aria-checked={style.preset === id}
        tabindex={rovingIndex(PRESET_IDS, style.preset) === i ? 0 : -1}
        onkeydown={(e) => radioKeydown(e, PRESET_IDS, style.preset, pick)}
        class="sp-preset" class:active={style.preset === id} onclick={() => pick(id)}>
        <span class="sp-swatch" aria-hidden="true">
          <span
            class="sp-sample"
            class:boxed={p.box}
            style:font-family={cssFontStack(p.font)}
            style:font-weight={p.bold ? 700 : 400}
            style:text-transform={p.uppercase ? "uppercase" : "none"}
            style:align-self={p.align === "left" ? "flex-start" : "center"}
          >
            {#if p.karaoke}<span class="sp-hi">{$t("studio.sample_word_1")}</span> {$t("studio.sample_word_2")}{:else}{$t("studio.sample_word_1")} {$t("studio.sample_word_2")}{/if}
          </span>
        </span>
        <span class="sp-preset-name">{$t(`studio.preset_${id}`)}</span>
      </button>
    {/each}
  </div>

  <div class="sp-grid">
    <label class="field">
      <span class="field-label">{$t("studio.font")}</span>
      <select class="input" bind:value={style.font}>
        {#each FONTS as font (font)}
          <option value={font}>{font}{font === FONTS[0] ? ` · ${$t("studio.font_bundled")}` : ""}</option>
        {/each}
      </select>
    </label>

    <label class="field">
      <span class="field-label">{$t("studio.size")} <output class="sp-num">{style.size}</output></span>
      <input type="range" min="24" max="140" step="2" bind:value={style.size} />
    </label>

    <label class="field">
      <span class="field-label">{$t("studio.max_chars")} <output class="sp-num">{style.maxChars}</output></span>
      <input type="range" min="10" max="60" step="1" bind:value={style.maxChars} />
    </label>

    <div class="field">
      <span class="field-label" id="sp-lines">{$t("studio.lines")}</span>
      <div class="sp-seg" role="radiogroup" aria-labelledby="sp-lines">
        {#each LINES as n, i (n)}
          <button type="button" role="radio" class="btn-pill" aria-checked={style.maxLines === n} tabindex={rovingIndex(LINES, style.maxLines) === i ? 0 : -1} onkeydown={(e) => radioKeydown(e, LINES, style.maxLines, (v) => (style.maxLines = v))} class:active={style.maxLines === n} onclick={() => (style.maxLines = n)}>{$t(n === 1 ? "studio.lines_one" : "studio.lines_two")}</button>
        {/each}
      </div>
    </div>

    <div class="field">
      <span class="field-label" id="sp-pos">{$t("studio.position")}</span>
      <div class="sp-seg" role="radiogroup" aria-labelledby="sp-pos">
        {#each POSITIONS as pos, i (pos)}
          <button type="button" role="radio" class="btn-pill" aria-checked={style.position === pos} tabindex={rovingIndex(POSITIONS, style.position) === i ? 0 : -1} onkeydown={(e) => radioKeydown(e, POSITIONS, style.position, (v) => (style.position = v))} class:active={style.position === pos} onclick={() => (style.position = pos)}>{$t(`studio.pos_${pos}`)}</button>
        {/each}
      </div>
    </div>

    <div class="field">
      <span class="field-label" id="sp-align">{$t("studio.align")}</span>
      <div class="sp-seg" role="radiogroup" aria-labelledby="sp-align">
        {#each ALIGNS as a, i (a)}
          <button type="button" role="radio" class="btn-pill" aria-checked={style.align === a} tabindex={rovingIndex(ALIGNS, style.align) === i ? 0 : -1} onkeydown={(e) => radioKeydown(e, ALIGNS, style.align, (v) => (style.align = v))} class:active={style.align === a} onclick={() => (style.align = a)}>{$t(`studio.align_${a}`)}</button>
        {/each}
      </div>
    </div>
  </div>

  <div class="sp-checks">
    <label class="sp-check"><input type="checkbox" class="checkbox" bind:checked={style.bold} /> <span>{$t("studio.bold")}</span></label>
    <label class="sp-check"><input type="checkbox" class="checkbox" bind:checked={style.uppercase} /> <span>{$t("studio.uppercase")}</span></label>
    <label class="sp-check"><input type="checkbox" class="checkbox" bind:checked={style.karaoke} /> <span>{$t("studio.karaoke")}</span></label>
    <label class="sp-check"><input type="checkbox" class="checkbox" bind:checked={style.box} /> <span>{$t("studio.box")}</span></label>
  </div>

  <div class="sp-colors">
    <ColorField label={$t("studio.text_color") as string} bind:value={style.color} {swatches} {disabled} />
    {#if style.karaoke}
      <ColorField label={$t("studio.highlight_color") as string} bind:value={style.highlight} {swatches} {disabled} />
    {/if}
    {#if style.box}
      <ColorField label={$t("studio.box_color") as string} bind:value={style.boxColor} {swatches} {disabled} />
    {:else}
      <ColorField label={$t("studio.outline_color") as string} bind:value={style.outlineColor} {swatches} {disabled} />
    {/if}
  </div>

  <div class="sp-grid">
    <label class="field">
      <span class="field-label">{style.box ? $t("studio.box_padding") : $t("studio.outline")} <output class="sp-num">{style.outline}</output></span>
      <input type="range" min="0" max="20" step="1" bind:value={style.outline} />
    </label>
    {#if !style.box}
      <label class="field">
        <span class="field-label">{$t("studio.shadow")} <output class="sp-num">{style.shadow}</output></span>
        <input type="range" min="0" max="10" step="1" bind:value={style.shadow} />
      </label>
    {/if}
  </div>
</fieldset>

<style>
  .sp {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: none;
  }

  .sp-legend {
    padding: 0;
    margin-bottom: var(--space-3);
    font-size: var(--text-base);
    font-weight: 650;
    color: var(--text);
  }

  .sp-presets {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--space-2);
  }

  .sp-preset {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font: inherit;
    cursor: pointer;
    text-align: left;
  }

  .sp-preset.active {
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  .sp-preset:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 2px;
  }

  .sp-swatch {
    display: flex;
    align-items: center;
    height: 44px;
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--primary);
    overflow: hidden;
  }

  .sp-sample {
    font-size: var(--text-sm);
    color: var(--secondary);
    white-space: nowrap;
  }

  .sp-sample.boxed {
    padding: 1px 4px;
    background: var(--secondary);
    color: var(--primary);
  }

  .sp-hi {
    color: var(--warning);
  }

  .sp-preset-name {
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .sp-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-3) var(--space-4);
  }

  .sp-grid input[type="range"] {
    accent-color: var(--accent);
  }

  .sp-num {
    margin-left: var(--space-1);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    font-weight: 400;
  }

  .sp-seg {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .sp-checks {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
  }

  .sp-check {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .sp-colors {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  @media (max-width: 720px) {
    .sp-presets {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
</style>
