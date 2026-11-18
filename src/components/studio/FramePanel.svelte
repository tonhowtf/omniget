<script lang="ts">
  import { t } from "$lib/i18n";
  import { hasCropSlack } from "$lib/studio/frame";
  import type { Aspect, FrameMode, IntroSettings, Reframe } from "$lib/studio/types";
  import { radioKeydown, rovingIndex } from "$lib/studio/radio";
  import ColorField from "./ColorField.svelte";

  let {
    reframe = $bindable(),
    intro = $bindable(),
    fadeOut = $bindable(),
    showIntro = $bindable(),
    srcW,
    srcH,
    canDrawText,
    swatches,
    disabled = false,
  }: {
    reframe: Reframe;
    intro: IntroSettings;
    fadeOut: boolean;
    showIntro: boolean;
    srcW: number;
    srcH: number;
    canDrawText: boolean;
    swatches: string[];
    disabled?: boolean;
  } = $props();

  const ASPECTS: { id: Aspect; w: number; h: number }[] = [
    { id: "keep", w: 16, h: 10 },
    { id: "9:16", w: 9, h: 16 },
    { id: "1:1", w: 13, h: 13 },
    { id: "4:5", w: 11, h: 14 },
  ];
  const MODES: FrameMode[] = ["crop", "fit"];
  const ASPECT_IDS = ASPECTS.map((a) => a.id);

  let slack = $derived(hasCropSlack(srcW, srcH, reframe.aspect));
  let sliderOff = $derived(reframe.aspect === "keep" || reframe.mode !== "crop" || !slack);
  let sliderReason = $derived(
    reframe.aspect === "keep"
      ? $t("studio.position_off_keep")
      : reframe.mode !== "crop"
        ? $t("studio.position_off_fit")
        : !slack
          ? $t("studio.position_off_narrow")
          : "",
  );
</script>

<fieldset class="fp" {disabled}>
  <legend class="fp-legend">{$t("studio.frame_title")}</legend>

  <div class="fp-aspects" role="radiogroup" aria-label={$t("studio.aspect")}>
    {#each ASPECTS as a, i (a.id)}
      <button
        type="button"
        role="radio"
        aria-checked={reframe.aspect === a.id}
        tabindex={rovingIndex(ASPECT_IDS, reframe.aspect) === i ? 0 : -1}
        onkeydown={(e) => radioKeydown(e, ASPECT_IDS, reframe.aspect, (v) => (reframe.aspect = v))} class="fp-aspect" class:active={reframe.aspect === a.id} onclick={() => (reframe.aspect = a.id)}>
        <span class="fp-shape" style:width="{a.w * 1.6}px" style:height="{a.h * 1.6}px" aria-hidden="true"></span>
        <span class="fp-aspect-name">{a.id === "keep" ? $t("studio.aspect_keep") : a.id}</span>
      </button>
    {/each}
  </div>

  {#if reframe.aspect !== "keep"}
    <div class="field">
      <span class="field-label" id="fp-mode">{$t("studio.fill_mode")}</span>
      <div class="fp-modes" role="radiogroup" aria-labelledby="fp-mode">
        {#each MODES as m (m)}
          <label class="fp-mode" class:active={reframe.mode === m}>
            <input type="radio" class="radio" name="fp-mode" value={m} bind:group={reframe.mode} />
            <span>
              <span class="field-label">{$t(`studio.mode_${m}`)}</span>
              <span class="field-hint">{$t(`studio.mode_${m}_hint`)}</span>
            </span>
          </label>
        {/each}
      </div>
    </div>
  {/if}

  <div class="field">
    <span class="field-label">{$t("studio.crop_position")}</span>
    <div class="fp-slider">
      <span class="fp-end" aria-hidden="true">{$t("studio.left")}</span>
      <input
        type="range"
        min="0"
        max="1"
        step="0.01"
        bind:value={reframe.position}
        disabled={disabled || sliderOff}
        aria-label={$t("studio.crop_position")}
        aria-describedby={sliderOff ? "fp-pos-why" : undefined}
      />
      <span class="fp-end" aria-hidden="true">{$t("studio.right")}</span>
      <button type="button" class="btn btn-ghost btn-sm" disabled={disabled || sliderOff || reframe.position === 0.5} onclick={() => (reframe.position = 0.5)}>{$t("studio.center")}</button>
    </div>
    {#if sliderOff}
      <span class="field-hint" id="fp-pos-why">{sliderReason}</span>
    {/if}
  </div>

  <label class="fp-check">
    <input type="checkbox" class="checkbox" bind:checked={fadeOut} />
    <span>{$t("studio.fade_out_end")}</span>
  </label>

  <section class="fp-intro" aria-labelledby="fp-intro-title">
    <div class="fp-intro-head">
      <h3 class="fp-h" id="fp-intro-title">{$t("studio.intro_title")}</h3>
      <label class="fp-check">
        <input type="checkbox" class="checkbox" bind:checked={intro.enabled} disabled={disabled || !canDrawText} />
        <span>{$t("studio.intro_enable")}</span>
      </label>
    </div>
    {#if !canDrawText}
      <p class="field-hint">{$t("studio.intro_unavailable")}</p>
    {:else if intro.enabled}
      <label class="field">
        <span class="field-label">{$t("studio.intro_heading")}</span>
        <input class="input" type="text" maxlength="90" bind:value={intro.title} placeholder={$t("studio.intro_heading_ph")} />
      </label>
      <label class="field">
        <span class="field-label">{$t("studio.intro_sub")}</span>
        <input class="input" type="text" maxlength="120" bind:value={intro.subtitle} placeholder={$t("studio.intro_sub_ph")} />
      </label>
      {#if !intro.title.trim() && !intro.subtitle.trim()}
        <p class="field-hint">{$t("studio.intro_needs_text")}</p>
      {/if}
      <label class="field">
        <span class="field-label">{$t("studio.intro_seconds")} <output class="fp-num">{$t("studio.seconds", { num: intro.seconds })}</output></span>
        <input type="range" min="1" max="10" step="0.5" bind:value={intro.seconds} />
      </label>
      <ColorField label={$t("studio.intro_bg") as string} bind:value={intro.background} {swatches} {disabled} />
      <ColorField label={$t("studio.intro_color") as string} bind:value={intro.color} {swatches} {disabled} />
      <div class="fp-checks">
        <label class="fp-check">
          <input type="checkbox" class="checkbox" bind:checked={intro.fade} />
          <span>{$t("studio.intro_fade")}</span>
        </label>
        <label class="fp-check">
          <input type="checkbox" class="checkbox" bind:checked={showIntro} />
          <span>{$t("studio.intro_preview")}</span>
        </label>
      </div>
    {/if}
  </section>
</fieldset>

<style>
  .fp {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: none;
  }

  .fp-legend {
    padding: 0;
    margin-bottom: var(--space-3);
    font-size: var(--text-base);
    font-weight: 650;
    color: var(--text);
  }

  .fp-h {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 650;
    color: var(--text);
  }

  .fp-aspects {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--space-2);
  }

  .fp-aspect {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    height: 76px;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }

  .fp-aspect.active {
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  .fp-aspect:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 2px;
  }

  .fp-shape {
    border-radius: 3px;
    border: 1.5px solid currentColor;
    opacity: 0.7;
  }

  .fp-aspect.active .fp-shape {
    color: var(--accent-text);
    opacity: 1;
  }

  .fp-aspect-name {
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .fp-modes {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-2);
  }

  .fp-mode {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    cursor: pointer;
  }

  .fp-mode > span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .fp-mode.active {
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  .fp-mode input {
    margin-top: 2px;
  }

  .fp-slider {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .fp-slider input {
    flex: 1;
    min-width: 0;
    accent-color: var(--accent);
  }

  .fp-end {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .fp-check {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .fp-checks {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
  }

  .fp-intro {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding-top: var(--space-4);
    border-top: var(--hairline) solid var(--content-border);
  }

  .fp-intro-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .fp-intro input[type="range"] {
    accent-color: var(--accent);
  }

  .fp-num {
    margin-left: var(--space-1);
    color: var(--text-dim);
    font-weight: 400;
  }
</style>
