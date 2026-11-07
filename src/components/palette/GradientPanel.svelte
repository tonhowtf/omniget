<script lang="ts">
  import { t } from "$lib/i18n";
  import { cssColor, type Oklch } from "$lib/palette/color";
  import { EASINGS, PRESETS, SPACES, bezierOf, expandStops, gradientCss, nativeGradientCss, sampleAt, sortedStops, type Bezier, type EasingKind, type Space } from "$lib/palette/gradient";
  import { workspace } from "$lib/palette/workspace.svelte";
  import ColorField from "./ColorField.svelte";
  import CopyButton from "./CopyButton.svelte";

  const SPACE_LABELS: Record<Space, string> = { oklch: "OKLCH", oklab: "OKLab", srgb: "sRGB" };

  let project = $derived(workspace.project!);
  let g = $derived(project.gradient);
  let format = $state<"hex" | "oklch">("hex");

  function preview(space: Space): string {
    const stops = expandStops({ ...g, samples: Math.max(g.samples, 16) }, space);
    return `linear-gradient(90deg, ${stops.map((s) => `${cssColor(s.color)} ${(s.pos * 100).toFixed(2)}%`).join(", ")})`;
  }

  let css = $derived(gradientCss(g, g.space, format));
  let native = $derived(nativeGradientCss(g));
  let bezier = $derived(bezierOf(g));
  let curve = $derived(`M0 40 C ${bezier[0] * 40} ${40 - bezier[1] * 40}, ${bezier[2] * 40} ${40 - bezier[3] * 40}, 40 0`);
  let stopCount = $derived(expandStops(g).length);

  function setStop(i: number, color: Oklch) {
    g.stops[i].color = color;
    workspace.touch();
  }

  function setPos(i: number, pos: number) {
    g.stops[i].pos = Math.min(1, Math.max(0, pos));
    workspace.touch();
  }

  function addStop() {
    const sorted = sortedStops(g.stops);
    let gap = { at: 0.5, size: 0 };
    for (let i = 0; i < sorted.length - 1; i++) {
      const size = sorted[i + 1].pos - sorted[i].pos;
      if (size > gap.size) gap = { at: sorted[i].pos + size / 2, size };
    }
    g.stops = [...g.stops, { color: sampleAt(g, gap.at), pos: Math.round(gap.at * 1000) / 1000 }];
    workspace.touch();
  }

  function removeStop(i: number) {
    workspace.deleteStop(i, $t("palette.gradient.stop_removed") as string);
  }

  function setSpace(space: Space) {
    g.space = space;
    workspace.touch();
  }

  function setEasing(easing: EasingKind) {
    if (easing === "custom" && g.easing !== "custom") g.bezier = [...PRESETS[g.easing]] as Bezier;
    g.easing = easing;
    workspace.touch();
  }

  function setBezier(i: number, v: number) {
    if (!Number.isFinite(v)) return;
    const next = [...g.bezier] as Bezier;
    next[i] = i % 2 === 0 ? Math.min(1, Math.max(0, v)) : Math.min(2, Math.max(-1, v));
    g.bezier = next;
    workspace.touch();
  }

  function fromPalette() {
    const pals = project.palettes;
    if (pals.length < 2) return;
    g.stops = [
      { color: { ...pals[0].base }, pos: 0 },
      { color: { ...pals[1].base }, pos: 1 },
    ];
    workspace.touch();
  }
</script>

<div class="pl-grad">
  <div class="pl-grad-compare" aria-label={$t("palette.gradient.compare")}>
    {#each SPACES as space (space)}
      <button type="button" class="pl-grad-row" class:active={g.space === space} aria-pressed={g.space === space} onclick={() => setSpace(space)}>
        <span class="pl-grad-label">
          {SPACE_LABELS[space]}
          {#if g.space === space}<span class="tag tag-accent">{$t("palette.gradient.in_use")}</span>{/if}
        </span>
        <span class="pl-grad-bar" style:background={preview(space)}></span>
      </button>
    {/each}
    <p class="pl-hint">{$t("palette.gradient.compare_hint")}</p>
  </div>

  <div class="pl-grad-grid">
    <section class="pl-card" aria-labelledby="pl-stops-title">
      <div class="pl-card-head">
        <h3 id="pl-stops-title" class="pl-subtitle">{$t("palette.gradient.stops")}</h3>
        {#if project.palettes.length >= 2}
          <button type="button" class="btn btn-ghost btn-sm" onclick={fromPalette}>{$t("palette.gradient.from_palettes")}</button>
        {/if}
      </div>
      <ol class="pl-stops">
        {#each g.stops as stop, i (i)}
          <li>
            <ColorField color={stop.color} label={$t("palette.gradient.stop_color", { n: i + 1 })} onchange={(c) => setStop(i, c)} />
            <input type="range" min="0" max="1" step="0.01" value={stop.pos} aria-label={$t("palette.gradient.stop_pos", { n: i + 1 })} oninput={(e) => setPos(i, Number(e.currentTarget.value))} />
            <output class="pl-num">{Math.round(stop.pos * 100)}%</output>
            <button type="button" class="btn btn-ghost btn-sm pl-icon-btn" disabled={g.stops.length <= 2} title={g.stops.length <= 2 ? $t("palette.gradient.min_stops") : $t("palette.gradient.remove_stop", { n: i + 1 })} aria-label={$t("palette.gradient.remove_stop", { n: i + 1 })} onclick={() => removeStop(i)}>
              <svg viewBox="0 0 12 12" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9" /></svg>
            </button>
          </li>
        {/each}
      </ol>
      <div>
        <button type="button" class="btn btn-secondary btn-sm" disabled={g.stops.length >= 16} onclick={addStop}>{$t("palette.gradient.add_stop")}</button>
      </div>
    </section>

    <section class="pl-card" aria-labelledby="pl-ease-title">
      <h3 id="pl-ease-title" class="pl-subtitle">{$t("palette.gradient.easing")}</h3>
      <div class="pl-ease">
        <svg class="pl-curve" viewBox="-4 -4 48 48" width="72" height="72" aria-hidden="true">
          <rect x="0" y="0" width="40" height="40" fill="none" stroke="currentColor" stroke-opacity="0.2" />
          <path d={curve} fill="none" stroke="currentColor" stroke-width="2" />
        </svg>
        <div class="pl-ease-controls">
          <select class="input" aria-label={$t("palette.gradient.easing")} value={g.easing} onchange={(e) => setEasing(e.currentTarget.value as EasingKind)}>
            {#each EASINGS as e (e)}<option value={e}>{$t(`palette.easing.${e}`)}</option>{/each}
          </select>
          {#if g.easing === "custom"}
            <div class="pl-bezier">
              {#each ["x1", "y1", "x2", "y2"] as name, i (name)}
                <label>
                  <span>{name}</span>
                  <input class="input" type="number" step="0.01" min={i % 2 === 0 ? 0 : -1} max={i % 2 === 0 ? 1 : 2} value={g.bezier[i]} onchange={(e) => setBezier(i, Number(e.currentTarget.value))} />
                </label>
              {/each}
            </div>
          {/if}
        </div>
      </div>
      <label class="pl-field-row">
        <span class="field-label">{$t("palette.gradient.samples")}</span>
        <input type="range" min="2" max="24" step="1" value={g.samples} disabled={g.easing === "linear" && g.space === "srgb"} oninput={(e) => { g.samples = Number(e.currentTarget.value); workspace.touch(); }} />
        <output class="pl-num">{g.samples}</output>
      </label>
      {#if g.easing === "linear" && g.space === "srgb"}
        <p class="pl-hint">{$t("palette.gradient.samples_off")}</p>
      {/if}
      <label class="pl-field-row">
        <span class="field-label">{$t("palette.gradient.angle")}</span>
        <input type="range" min="0" max="360" step="1" value={g.angle} oninput={(e) => { g.angle = Number(e.currentTarget.value); workspace.touch(); }} />
        <output class="pl-num">{g.angle}°</output>
      </label>
    </section>
  </div>

  <section class="pl-card" aria-labelledby="pl-css-title">
    <div class="pl-card-head">
      <h3 id="pl-css-title" class="pl-subtitle">{$t("palette.gradient.css")}</h3>
      <div class="segmented" role="radiogroup" aria-label={$t("palette.gradient.css_format")}>
        <button type="button" class="segmented-btn" role="radio" aria-checked={format === "hex"} onclick={() => (format = "hex")}>HEX</button>
        <button type="button" class="segmented-btn" role="radio" aria-checked={format === "oklch"} onclick={() => (format = "oklch")}>OKLCH</button>
      </div>
    </div>
    <div class="pl-swatch-big" style:background={css}></div>
    <div class="pl-code-row">
      <code class="pl-code">background: {css};</code>
      <CopyButton text={`background: ${css};`} label={$t("palette.gradient.copy_css")} />
    </div>
    <p class="pl-hint">{$t("palette.gradient.css_hint", { count: stopCount })}</p>
    <div class="pl-code-row">
      <code class="pl-code">background: {native};</code>
      <CopyButton text={`background: ${native};`} label={$t("palette.gradient.copy_native")} />
    </div>
    <p class="pl-hint">{$t("palette.gradient.native_hint")}</p>
  </section>
</div>

<style>
  .pl-grad {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .pl-grad-compare {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .pl-grad-row {
    display: grid;
    grid-template-columns: 170px minmax(0, 1fr);
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2);
    border: 0;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-muted);
    text-align: left;
    cursor: pointer;
  }

  .pl-grad-row:hover {
    background: var(--surface);
  }

  .pl-grad-row.active {
    background: var(--surface);
    color: var(--text);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-grad-row:focus-visible {
    outline: var(--focus-ring);
  }

  .pl-grad-label {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .pl-grad-bar {
    height: 40px;
    border-radius: var(--radius-sm);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-grad-grid {
    display: grid;
    grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr);
    gap: var(--space-4);
  }

  @media (max-width: 860px) {
    .pl-grad-grid {
      grid-template-columns: 1fr;
    }
  }

  .pl-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    min-width: 0;
  }

  .pl-card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
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

  .pl-hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .pl-stops {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .pl-stops li {
    display: grid;
    grid-template-columns: minmax(130px, 170px) minmax(60px, 1fr) 40px auto;
    align-items: center;
    gap: var(--space-2);
  }

  .pl-num {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }

  .pl-icon-btn {
    width: var(--control-h);
    padding: 0;
    justify-content: center;
  }

  .pl-ease {
    display: flex;
    gap: var(--space-3);
    align-items: flex-start;
  }

  .pl-curve {
    flex-shrink: 0;
    color: var(--accent-text);
  }

  .pl-ease-controls {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
  }

  .pl-bezier {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--space-1);
  }

  .pl-bezier label {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--text-caption);
    color: var(--text-dim);
    font-family: var(--font-mono);
  }

  .pl-bezier .input {
    min-width: 0;
    padding-inline: var(--space-1);
    font-family: var(--font-mono);
  }

  .pl-field-row {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr) 40px;
    align-items: center;
    gap: var(--space-2);
  }

  .pl-swatch-big {
    height: 72px;
    border-radius: var(--radius-md);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-code-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
  }

  .pl-code {
    flex: 1;
    min-width: 0;
    max-height: 96px;
    overflow: auto;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--surface-mut);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text);
    overflow-wrap: anywhere;
    user-select: all;
  }
</style>
