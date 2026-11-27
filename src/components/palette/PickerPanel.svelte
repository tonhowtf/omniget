<script lang="ts">
  import { t } from "$lib/i18n";
  import {
    FORMAT_IDS,
    cssColor,
    fitToGamut,
    formatAs,
    inP3,
    inSrgb,
    parseColor,
    sameColor,
    toHex,
    type Oklch,
  } from "$lib/palette/color";
  import { newId, slugify, uniqueToken } from "$lib/palette/project";
  import { workspace } from "$lib/palette/workspace.svelte";
  import Track from "./Track.svelte";
  import ChromaPlane from "./ChromaPlane.svelte";
  import CopyButton from "./CopyButton.svelte";
  import StatusIcon from "#components/converter/StatusIcon.svelte";

  const FORMAT_LABELS: Record<string, string> = { hex: "HEX", rgb: "RGB", hsl: "HSL", oklch: "OKLCH", oklab: "OKLab" };

  function formatLabel(id: string): string {
    return id === "p3" ? ($t("palette.gamut_p3") as string) : FORMAT_LABELS[id];
  }

  let color = $derived(workspace.current);
  let text = $state("");
  let invalid = $state(false);
  let last: Oklch | null = null;
  let target = $state<string>("");

  let srgb = $derived(inSrgb(color));
  let p3 = $derived(inP3(color));
  let palettes = $derived(workspace.project?.palettes ?? []);

  $effect(() => {
    if (!last || !sameColor(last, color)) {
      text = formatAs(color, srgb ? "hex" : "oklch");
      invalid = false;
      last = color;
    }
  });

  $effect(() => {
    if (!palettes.some((p) => p.id === target)) target = palettes[0]?.id ?? "";
  });

  function set(next: Partial<Oklch>) {
    workspace.current = { ...color, ...next };
  }

  function commit() {
    const parsed = parseColor(text);
    if (!parsed) {
      invalid = true;
      return;
    }
    invalid = false;
    last = parsed;
    workspace.current = parsed;
  }

  function rail(fn: (t: number) => Oklch): string {
    const stops = Array.from({ length: 19 }, (_, i) => cssColor(fn(i / 18)));
    return `linear-gradient(to right, ${stops.join(", ")})`;
  }

  let lRail = $derived(rail((x) => fitToGamut({ ...color, l: x, alpha: 1 }, "p3")));
  let cRail = $derived(rail((x) => fitToGamut({ ...color, c: x * 0.4, alpha: 1 }, "p3")));
  let hRail = $derived(rail((x) => fitToGamut({ ...color, h: x * 360, alpha: 1 }, "p3")));
  let aRail = $derived(`linear-gradient(to right, ${cssColor({ ...color, alpha: 0 })}, ${cssColor({ ...color, alpha: 1 })})`);

  function fit() {
    workspace.current = fitToGamut(color, "srgb");
  }

  function useAsBase() {
    const p = workspace.project;
    const pal = p?.palettes.find((x) => x.id === target);
    if (!p || !pal) return;
    pal.base = { ...color, alpha: 1 };
    workspace.touch();
    workspace.showToast($t("palette.picker.base_set", { name: pal.name }) as string, "success");
  }

  function addPalette() {
    const p = workspace.project;
    if (!p) return;
    const name = $t("palette.picker.new_palette_name", { hex: toHex({ ...color, alpha: 1 }) }) as string;
    const token = uniqueToken(slugify(`color-${toHex(color).slice(1, 7)}`, "color"), p.palettes.map((x) => x.token));
    p.palettes = [...p.palettes, { id: newId("pal"), name, token, base: { ...color, alpha: 1 }, hueShift: 0 }];
    workspace.touch();
    workspace.showToast($t("palette.picker.palette_added", { name }) as string, "success");
  }
</script>

<div class="pl-picker">
  <div class="pl-picker-left">
    <div class="pl-preview" style:--c={cssColor(color)}>
      <span class="pl-preview-fill" aria-hidden="true"></span>
    </div>
    <label class="field">
      <span class="field-label">{$t("palette.picker.input_label")}</span>
      <input
        class="input pl-mono"
        class:error={invalid}
        type="text"
        spellcheck="false"
        autocomplete="off"
        aria-invalid={invalid}
        aria-describedby="pl-input-hint"
        bind:value={text}
        onchange={commit}
        onkeydown={(e) => e.key === "Enter" && commit()}
      />
      {#if invalid}
        <span class="field-error" id="pl-input-hint" role="alert">{$t("palette.picker.input_error")}</span>
      {:else}
        <span class="field-hint" id="pl-input-hint">{$t("palette.picker.input_hint")}</span>
      {/if}
    </label>

    <ul class="pl-gamut" aria-label={$t("palette.picker.gamut_label")}>
      <li class:ok={srgb}>
        <StatusIcon name={srgb ? "done" : "warning"} size={14} />
        <span>{srgb ? $t("palette.picker.in_srgb") : $t("palette.picker.out_srgb")}</span>
      </li>
      <li class:ok={p3}>
        <StatusIcon name={p3 ? "done" : "warning"} size={14} />
        <span>{p3 ? $t("palette.picker.in_p3") : $t("palette.picker.out_p3")}</span>
      </li>
    </ul>
    {#if !srgb}
      <div class="pl-fit">
        <p>{$t("palette.picker.fit_body", { hex: toHex(color) })}</p>
        <button type="button" class="btn btn-secondary btn-sm" onclick={fit}>{$t("palette.picker.fit")}</button>
      </div>
    {/if}
  </div>

  <div class="pl-picker-mid">
    <ChromaPlane {color} label={$t("palette.picker.plane_label")} onpick={(l, c) => set({ l, c })} />
    <p class="pl-plane-legend">{$t("palette.picker.plane_legend")}</p>
    <div class="pl-tracks">
      <Track label="L" name={$t("palette.picker.lightness")} value={color.l} min={0} max={1} step={0.001} scale={100} digits={1} unit="%" gradient={lRail} onchange={(l) => set({ l })} />
      <Track label="C" name={$t("palette.picker.chroma")} value={color.c} min={0} max={0.4} step={0.001} digits={3} gradient={cRail} onchange={(c) => set({ c })} />
      <Track label="H" name={$t("palette.picker.hue")} value={color.h} min={0} max={360} step={0.1} digits={1} unit="°" gradient={hRail} onchange={(h) => set({ h })} />
      <Track label="A" name={$t("palette.picker.alpha")} value={color.alpha ?? 1} min={0} max={1} step={0.01} scale={100} digits={0} unit="%" gradient={aRail} checker onchange={(alpha) => set({ alpha })} />
    </div>
  </div>

  <div class="pl-picker-right">
    <h3 class="pl-subtitle">{$t("palette.picker.formats")}</h3>
    <ul class="pl-formats">
      {#each FORMAT_IDS as id (id)}
        {@const value = formatAs(color, id)}
        <li>
          <span class="pl-format-name">{formatLabel(id)}</span>
          <code class="pl-mono">{value}</code>
          <CopyButton text={value} label={$t("palette.copy_format", { format: formatLabel(id) })} compact />
        </li>
      {/each}
    </ul>
    {#if !srgb}
      <p class="pl-note">{$t("palette.picker.formats_clipped")}</p>
    {/if}

    <h3 class="pl-subtitle">{$t("palette.picker.use_title")}</h3>
    {#if palettes.length}
      <div class="pl-use">
        <select class="input" aria-label={$t("palette.picker.use_target")} bind:value={target}>
          {#each palettes as pal (pal.id)}
            <option value={pal.id}>{pal.name}</option>
          {/each}
        </select>
        <button type="button" class="btn btn-primary btn-sm" onclick={useAsBase}>{$t("palette.picker.use_base")}</button>
      </div>
    {/if}
    <button type="button" class="btn btn-secondary btn-sm" onclick={addPalette}>{$t("palette.picker.add_palette")}</button>
  </div>
</div>

<style>
  .pl-picker {
    display: grid;
    grid-template-columns: minmax(200px, 240px) minmax(0, 1fr) minmax(240px, 300px);
    gap: var(--space-5);
    align-items: start;
  }

  @media (max-width: 900px) {
    .pl-picker {
      grid-template-columns: 1fr;
    }
  }

  .pl-picker-left,
  .pl-picker-mid,
  .pl-picker-right {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .pl-preview {
    height: 140px;
    border-radius: var(--radius-lg);
    background: repeating-conic-gradient(var(--surface-hi) 0% 25%, var(--surface) 0% 50%) 0 0 / 14px 14px;
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
  }

  .pl-preview-fill {
    display: block;
    height: 100%;
    background: var(--c);
  }

  .pl-mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .pl-gamut {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .pl-gamut li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .pl-fit {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--warning) 10%, transparent);
  }

  .pl-fit p,
  .pl-note,
  .pl-plane-legend {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .pl-fit p {
    color: var(--text);
  }

  .pl-tracks {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .pl-subtitle {
    margin: 0;
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .pl-formats {
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-formats li {
    display: grid;
    grid-template-columns: 64px minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--row-compact);
    padding: var(--space-1) var(--space-1) var(--space-1) var(--space-3);
  }

  .pl-formats li + li {
    border-top: var(--hairline) solid var(--separator);
  }

  .pl-format-name {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .pl-formats code {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    user-select: all;
  }

  .pl-use {
    display: flex;
    gap: var(--space-2);
  }

  .pl-use select {
    flex: 1;
    min-width: 0;
  }
</style>
