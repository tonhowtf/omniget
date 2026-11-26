<script lang="ts">
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { t } from "$lib/i18n";
  import { ensureFont } from "$lib/fonts/loader";
  import { fallbackFor, fontStack, type PreviewFont } from "$lib/fonts/preview";
  import { RATIOS, buildScale, num, toCssVariables, toTailwindTheme } from "$lib/fonts/scale";
  import type { Theme } from "$lib/fonts/sample";
  import FontPicker, { type PickerGroup } from "./FontPicker.svelte";
  import StatusIcon from "#components/converter/StatusIcon.svelte";

  let {
    font,
    groups,
    theme,
    lang,
    onfont,
  }: {
    font: PreviewFont | null;
    groups: PickerGroup[];
    theme: Theme;
    lang: string;
    onfont: (key: string) => void;
  } = $props();

  let base = $state(16);
  let ratio = $state(1.25);
  let stepsDown = $state(2);
  let stepsUp = $state(6);
  let fluid = $state(false);
  let maxBase = $state(18);
  let maxRatio = $state(1.333);
  let minViewport = $state(360);
  let maxViewport = $state(1440);
  let format = $state<"css" | "tailwind">("css");
  let copied = $state<"idle" | "done" | "failed">("idle");
  let copyTimer: ReturnType<typeof setTimeout> | null = null;

  let valid = $derived(base > 0 && maxBase > 0 && (!fluid || maxViewport > minViewport));
  let steps = $derived(
    valid
      ? buildScale({
          base,
          ratio,
          stepsDown: Math.min(4, Math.max(0, Math.round(stepsDown))),
          stepsUp: Math.min(10, Math.max(0, Math.round(stepsUp))),
          fluid,
          maxBase,
          maxRatio,
          minViewport,
          maxViewport,
        })
      : [],
  );
  let family = $derived(font ? font.family : null);
  let code = $derived(format === "css" ? toCssVariables(steps, family, fallbackFor(font?.category ?? "")) : toTailwindTheme(steps, family, font?.category ?? ""));
  let stack = $derived(fontStack(font));

  $effect(() => {
    void ensureFont(font, 400, false, lang);
    void ensureFont(font, 700, false, lang);
  });

  function ratioName(value: number): string {
    const preset = RATIOS.find((r) => r.value === value);
    return preset ? `${value} · ${$t(`fonts.ratio_${preset.key}`)}` : String(value);
  }

  async function copy() {
    try {
      await writeText(code);
      copied = "done";
    } catch {
      try {
        await navigator.clipboard.writeText(code);
        copied = "done";
      } catch {
        copied = "failed";
      }
    }
    if (copyTimer) clearTimeout(copyTimer);
    copyTimer = setTimeout(() => (copied = "idle"), 2400);
  }
</script>

<div class="ty-scale-ctl">
  <FontPicker value={font?.key ?? null} {groups} label={$t("fonts.scale_font") as string} onpick={onfont} />
  <label class="ty-scale-field">
    <span class="field-label">{fluid ? $t("fonts.scale_base_min") : $t("fonts.scale_base")}</span>
    <span class="ty-scale-unit"><input class="input" type="number" min="8" max="40" step="1" bind:value={base} /><span>px</span></span>
  </label>
  <label class="ty-scale-field">
    <span class="field-label">{fluid ? $t("fonts.scale_ratio_min") : $t("fonts.scale_ratio")}</span>
    <select class="ty-select" bind:value={ratio}>
      {#each RATIOS as r (r.value)}<option value={r.value}>{ratioName(r.value)}</option>{/each}
    </select>
  </label>
  <label class="ty-scale-field">
    <span class="field-label">{$t("fonts.scale_steps_down")}</span>
    <input class="input" type="number" min="0" max="4" step="1" bind:value={stepsDown} />
  </label>
  <label class="ty-scale-field">
    <span class="field-label">{$t("fonts.scale_steps_up")}</span>
    <input class="input" type="number" min="0" max="10" step="1" bind:value={stepsUp} />
  </label>
  <label class="ty-scale-check">
    <input type="checkbox" class="checkbox" bind:checked={fluid} />
    <span>
      <span class="field-label">{$t("fonts.scale_fluid")}</span>
      <span class="field-hint">{$t("fonts.scale_fluid_hint")}</span>
    </span>
  </label>
  {#if fluid}
    <label class="ty-scale-field">
      <span class="field-label">{$t("fonts.scale_base_max")}</span>
      <span class="ty-scale-unit"><input class="input" type="number" min="8" max="48" step="1" bind:value={maxBase} /><span>px</span></span>
    </label>
    <label class="ty-scale-field">
      <span class="field-label">{$t("fonts.scale_ratio_max")}</span>
      <select class="ty-select" bind:value={maxRatio}>
        {#each RATIOS as r (r.value)}<option value={r.value}>{ratioName(r.value)}</option>{/each}
      </select>
    </label>
    <label class="ty-scale-field">
      <span class="field-label">{$t("fonts.scale_vw_min")}</span>
      <span class="ty-scale-unit"><input class="input" type="number" min="240" max="2560" step="10" bind:value={minViewport} /><span>px</span></span>
    </label>
    <label class="ty-scale-field">
      <span class="field-label">{$t("fonts.scale_vw_max")}</span>
      <span class="ty-scale-unit"><input class="input" type="number" min="320" max="3840" step="10" bind:value={maxViewport} /><span>px</span></span>
    </label>
  {/if}
</div>

{#if !valid}
  <div class="ty-scale-warn" role="alert">
    <StatusIcon name="warning" />
    <p>{$t("fonts.scale_invalid")}</p>
  </div>
{:else}
  <ol class="ty-scale" data-theme={theme} style:font-family={stack}>
    {#each [...steps].reverse() as step (step.name)}
      <li class="ty-scale-row">
        <span class="ty-scale-meta">
          <span class="ty-scale-name">{step.name}</span>
          <span class="ty-scale-px">{fluid ? `${num(step.min, 1)}–${num(step.max, 1)}px` : `${num(step.min, 1)}px`}</span>
        </span>
        <span class="ty-scale-sample" style:font-size={step.value} style:line-height={step.lineHeight} style:font-weight={step.exponent >= 2 ? 700 : 400}>{$t("fonts.scale_sample")}</span>
      </li>
    {/each}
  </ol>
  {#if fluid}
    <p class="ty-scale-hint">{$t("fonts.scale_live_hint")}</p>
  {/if}

  <section class="ty-export" aria-labelledby="ty-export-title">
    <div class="ty-export-head">
      <h3 id="ty-export-title">{$t("fonts.export_title")}</h3>
      <div class="segmented" role="group" aria-label={$t("fonts.export_format") as string}>
        <button type="button" class="segmented-btn" class:active={format === "css"} aria-pressed={format === "css"} onclick={() => (format = "css")}>{$t("fonts.export_css")}</button>
        <button type="button" class="segmented-btn" class:active={format === "tailwind"} aria-pressed={format === "tailwind"} onclick={() => (format = "tailwind")}>{$t("fonts.export_tailwind")}</button>
      </div>
      <button type="button" class="btn btn-primary btn-sm" onclick={copy}>
        {format === "css" ? $t("fonts.copy_css") : $t("fonts.copy_tailwind")}
      </button>
    </div>
    <pre class="ty-export-code"><code>{code}</code></pre>
    <p class="ty-export-status" role="status" aria-live="polite">
      {#if copied === "done"}<StatusIcon name="done" size={13} /> {$t("fonts.copied")}{:else if copied === "failed"}<StatusIcon name="failed" size={13} /> {$t("fonts.copy_failed")}{/if}
    </p>
  </section>
{/if}

<style>
  .ty-scale-ctl {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    align-items: end;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-scale-ctl > :global(.ty-pick) {
    grid-column: span 2;
  }

  .ty-scale-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .ty-scale-field .input,
  .ty-scale-field .ty-select {
    width: 100%;
    min-width: 0;
  }

  .ty-scale-unit {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .ty-scale-check {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    grid-column: span 2;
  }

  .ty-scale-check > span {
    display: flex;
    flex-direction: column;
  }

  .ty-scale-warn {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .ty-scale-warn p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .ty-scale {
    margin: 0;
    padding: var(--space-3) var(--space-5);
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--bg);
    color: var(--text);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-scale-row {
    display: grid;
    grid-template-columns: 110px minmax(0, 1fr);
    align-items: baseline;
    gap: var(--space-4);
    padding: var(--space-2) 0;
  }

  .ty-scale-row + .ty-scale-row {
    border-top: var(--hairline) solid var(--separator);
  }

  .ty-scale-meta {
    display: flex;
    flex-direction: column;
    font-family: var(--font-body);
  }

  .ty-scale-name {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .ty-scale-px {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .ty-scale-sample {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .ty-scale-hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .ty-export {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .ty-export-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .ty-export-head h3 {
    margin: 0;
    margin-right: auto;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .ty-export-code {
    margin: 0;
    padding: var(--space-3) var(--space-4);
    max-height: 320px;
    overflow: auto;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: 1.55;
    color: var(--text);
    user-select: text;
  }

  .ty-export-status {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-height: 1.4em;
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
</style>
