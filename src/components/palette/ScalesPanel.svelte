<script lang="ts">
  import { t } from "$lib/i18n";
  import { cssColor, formatOklch, toHex, type Oklch } from "$lib/palette/color";
  import { HARMONY_KINDS, harmony, neutralFrom, type HarmonyKind } from "$lib/palette/scale";
  import { newId, scaleOf, slugify, uniqueToken, type PaletteDef } from "$lib/palette/project";
  import { workspace } from "$lib/palette/workspace.svelte";
  import { wcagRatio } from "$lib/palette/contrast";
  import ColorField from "./ColorField.svelte";

  let project = $derived(workspace.project!);
  let harmonyFor = $state<string | null>(null);

  const WHITE: Oklch = { l: 1, c: 0, h: 0 };
  const BLACK: Oklch = { l: 0, c: 0, h: 0 };

  function inkOn(c: Oklch): string {
    return cssColor(wcagRatio(BLACK, c) >= wcagRatio(WHITE, c) ? BLACK : WHITE);
  }

  function rename(pal: PaletteDef, name: string) {
    pal.name = name.slice(0, 60);
    workspace.touch();
  }

  function retoken(pal: PaletteDef, value: string) {
    const index = project.palettes.indexOf(pal);
    pal.token = uniqueToken(slugify(value, `palette-${index + 1}`), project.palettes.map((p) => p.token), index);
    workspace.touch();
  }

  function addPalette() {
    const base = project.palettes[0]?.base ?? workspace.current;
    const name = $t("palette.scales.untitled", { num: project.palettes.length + 1 }) as string;
    const token = uniqueToken("color", project.palettes.map((p) => p.token));
    project.palettes = [...project.palettes, { id: newId("pal"), name, token, base: { ...base, h: (base.h + 60) % 360 }, hueShift: 0 }];
    workspace.touch();
  }

  function addHarmony(pal: PaletteDef, kind: HarmonyKind) {
    const colors = harmony(kind, pal.base, project.gamut);
    const added: PaletteDef[] = [];
    colors.forEach((base, i) => {
      const suffix = colors.length > 1 ? ` ${i + 1}` : "";
      const name = `${pal.name} · ${$t(`palette.harmony.${kind}`)}${suffix}`;
      const taken = [...project.palettes, ...added].map((p) => p.token);
      const token = uniqueToken(`${pal.token}-${kind.slice(0, 4)}${colors.length > 1 ? `-${i + 1}` : ""}`, taken);
      added.push({ id: newId("pal"), name, token, base, hueShift: pal.hueShift });
    });
    const index = project.palettes.indexOf(pal);
    const list = [...project.palettes];
    list.splice(index + 1, 0, ...added);
    project.palettes = list;
    harmonyFor = null;
    workspace.touch();
    workspace.showToast($t("palette.scales.harmony_added", { count: added.length, kind: $t(`palette.harmony.${kind}`) }) as string, "success");
  }

  function addNeutral(pal: PaletteDef) {
    const token = uniqueToken(`${pal.token}-neutral`, project.palettes.map((p) => p.token));
    const index = project.palettes.indexOf(pal);
    const list = [...project.palettes];
    list.splice(index + 1, 0, { id: newId("pal"), name: `${pal.name} · ${$t("palette.harmony.neutral")}`, token, base: neutralFrom(pal.base), hueShift: 0 });
    project.palettes = list;
    harmonyFor = null;
    workspace.touch();
  }

  function move(pal: PaletteDef, dir: -1 | 1) {
    const i = project.palettes.indexOf(pal);
    const j = i + dir;
    if (j < 0 || j >= project.palettes.length) return;
    const list = [...project.palettes];
    [list[i], list[j]] = [list[j], list[i]];
    project.palettes = list;
    workspace.touch();
  }

  function remove(pal: PaletteDef) {
    workspace.deletePalette(pal.id, (name) => $t("palette.scales.removed", { name }) as string);
  }

  function pick(c: Oklch) {
    workspace.current = { ...c, alpha: 1 };
    workspace.tab = "color";
  }

  function setGamut(g: "srgb" | "p3") {
    project.gamut = g;
    workspace.touch();
  }
</script>

<div class="pl-scales">
  <div class="pl-toolbar">
    <p class="pl-hint">{$t("palette.scales.hint")}</p>
    <div class="segmented" role="radiogroup" aria-label={$t("palette.scales.gamut")}>
      <button type="button" class="segmented-btn" role="radio" aria-checked={project.gamut === "srgb"} onclick={() => setGamut("srgb")}>{$t("palette.gamut_srgb")}</button>
      <button type="button" class="segmented-btn" role="radio" aria-checked={project.gamut === "p3"} onclick={() => setGamut("p3")}>{$t("palette.gamut_p3")}</button>
    </div>
  </div>

  {#if project.palettes.length === 0}
    <div class="pl-empty">
      <h3>{$t("palette.scales.empty_title")}</h3>
      <p>{$t("palette.scales.empty_body")}</p>
      <button type="button" class="btn btn-primary" onclick={addPalette}>{$t("palette.scales.add")}</button>
    </div>
  {:else}
    <ul class="pl-pal-list">
      {#each project.palettes as pal, i (pal.id)}
        {@const scale = scaleOf(pal, project.gamut)}
        <li class="pl-pal">
          <div class="pl-pal-head">
            <input class="input pl-pal-name" type="text" value={pal.name} aria-label={$t("palette.scales.name")} onchange={(e) => rename(pal, e.currentTarget.value)} />
            <span class="pl-token-wrap">
              <span class="pl-token-dash" aria-hidden="true">--</span>
              <input class="input pl-pal-token" type="text" value={pal.token} spellcheck="false" aria-label={$t("palette.scales.token")} title={$t("palette.scales.token_hint")} onchange={(e) => retoken(pal, e.currentTarget.value)} />
            </span>
            <div class="pl-pal-base">
              <ColorField color={pal.base} label={$t("palette.scales.base", { name: pal.name })} onchange={(c) => { pal.base = { ...c, alpha: 1 }; workspace.touch(); }} />
            </div>
            <label class="pl-drift">
              <span class="field-label">{$t("palette.scales.drift")}</span>
              <input type="range" min="-60" max="60" step="1" value={pal.hueShift} aria-valuetext={`${pal.hueShift}°`} oninput={(e) => { pal.hueShift = Number(e.currentTarget.value); workspace.touch(); }} />
              <output>{pal.hueShift > 0 ? "+" : ""}{pal.hueShift}°</output>
            </label>
            <div class="pl-pal-actions">
              <button type="button" class="btn btn-ghost btn-sm" aria-expanded={harmonyFor === pal.id} onclick={() => (harmonyFor = harmonyFor === pal.id ? null : pal.id)}>{$t("palette.scales.harmonies")}</button>
              <button type="button" class="btn btn-ghost btn-sm pl-icon-btn" disabled={i === 0} aria-label={$t("palette.scales.move_up", { name: pal.name })} title={$t("palette.scales.move_up", { name: pal.name })} onclick={() => move(pal, -1)}>
                <svg viewBox="0 0 12 12" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 7.5 6 4.5l3 3" /></svg>
              </button>
              <button type="button" class="btn btn-ghost btn-sm pl-icon-btn" disabled={i === project.palettes.length - 1} aria-label={$t("palette.scales.move_down", { name: pal.name })} title={$t("palette.scales.move_down", { name: pal.name })} onclick={() => move(pal, 1)}>
                <svg viewBox="0 0 12 12" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 4.5 6 7.5l3-3" /></svg>
              </button>
              <button type="button" class="btn btn-ghost btn-sm pl-icon-btn" aria-label={$t("palette.scales.remove", { name: pal.name })} title={$t("palette.scales.remove", { name: pal.name })} onclick={() => remove(pal)}>
                <svg viewBox="0 0 12 12" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9" /></svg>
              </button>
            </div>
          </div>
          {#if harmonyFor === pal.id}
            <div class="pl-harmony" role="group" aria-label={$t("palette.scales.harmonies")}>
              <span class="pl-hint">{$t("palette.scales.harmony_hint")}</span>
              {#each HARMONY_KINDS as kind (kind)}
                <button type="button" class="btn btn-secondary btn-sm" onclick={() => addHarmony(pal, kind)}>
                  <span class="pl-harmony-dots" aria-hidden="true">
                    {#each harmony(kind, pal.base, project.gamut) as c, j (j)}<span style:--c={cssColor(c)}></span>{/each}
                  </span>
                  {$t(`palette.harmony.${kind}`)}
                </button>
              {/each}
              <button type="button" class="btn btn-secondary btn-sm" onclick={() => addNeutral(pal)}>{$t("palette.harmony.neutral")}</button>
            </div>
          {/if}
          <ol class="pl-steps" aria-label={$t("palette.scales.steps", { name: pal.name })}>
            {#each scale as s (s.step)}
              <li>
                <button
                  type="button"
                  class="pl-step"
                  class:base={s.isBase}
                  style:--c={cssColor(s.color)}
                  style:--ink={inkOn(s.color)}
                  title={`${pal.token}-${s.step} · ${toHex(s.color)} · ${formatOklch(s.color)}`}
                  aria-label={$t("palette.scales.step_label", { name: pal.name, step: s.step, hex: toHex(s.color) })}
                  onclick={() => pick(s.color)}
                >
                  <span class="pl-step-num">{s.step}</span>
                  <span class="pl-step-hex">{toHex(s.color).slice(1)}</span>
                  {#if s.isBase}<span class="pl-step-base">{$t("palette.scales.base_mark")}</span>{/if}
                </button>
              </li>
            {/each}
          </ol>
        </li>
      {/each}
    </ul>
    <div>
      <button type="button" class="btn btn-secondary" onclick={addPalette}>{$t("palette.scales.add")}</button>
    </div>
  {/if}
</div>

<style>
  .pl-scales {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .pl-toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .pl-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .pl-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-7) var(--space-5);
    border-radius: var(--radius-lg);
    background: var(--surface);
    text-align: center;
  }

  .pl-empty h3 {
    margin: 0;
    font-size: var(--text-md);
    color: var(--text);
  }

  .pl-empty p {
    margin: 0 0 var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .pl-pal-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .pl-pal {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-pal-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
  }

  .pl-pal-name {
    width: 160px;
    font-weight: 600;
  }

  .pl-token-wrap {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .pl-pal-token {
    width: 120px;
    font-family: var(--font-mono);
  }

  .pl-pal-base {
    width: 150px;
  }

  .pl-drift {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .pl-drift input {
    width: 96px;
  }

  .pl-drift output {
    width: 40px;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }

  .pl-pal-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-left: auto;
  }

  .pl-icon-btn {
    width: var(--control-h);
    padding: 0;
    justify-content: center;
  }

  .pl-harmony {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-mut);
  }

  .pl-harmony-dots {
    display: inline-flex;
  }

  .pl-harmony-dots span {
    width: 10px;
    height: 10px;
    margin-right: -3px;
    border-radius: var(--radius-full);
    background: var(--c);
    box-shadow: 0 0 0 1px var(--surface);
  }

  .pl-steps {
    display: grid;
    grid-template-columns: repeat(11, minmax(0, 1fr));
    gap: 3px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .pl-step {
    position: relative;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    width: 100%;
    height: 64px;
    padding: var(--space-1) 6px;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--c);
    color: var(--ink);
    text-align: left;
    cursor: pointer;
    transition: transform var(--duration-fast) var(--ease-out);
  }

  .pl-step:hover {
    transform: translateY(-2px);
  }

  .pl-step:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .pl-step.base {
    box-shadow: inset 0 0 0 2px var(--ink);
  }

  .pl-step-num {
    font-size: var(--text-xs);
    font-weight: 650;
  }

  .pl-step-hex {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    opacity: 0.8;
  }

  .pl-step-base {
    position: absolute;
    top: var(--space-1);
    left: 6px;
    font-size: var(--text-caption);
    font-weight: 650;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
  }

  @media (prefers-reduced-motion: reduce) {
    .pl-step {
      transition: none;
    }
    .pl-step:hover {
      transform: none;
    }
  }
</style>
