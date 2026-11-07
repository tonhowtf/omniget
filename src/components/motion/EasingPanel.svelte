<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { DEFAULT_BEZIER, PRESETS, Y_MAX, Y_MIN, bezierEase, clampBezier, findPreset, overshoots, type Bezier, type PresetGroup } from "$lib/motion/bezier";
  import { cssBezier, cssTransition, gsapEase, motionEase, svelteEase } from "$lib/motion/exports";
  import { num } from "$lib/motion/format";
  import { loop, reducedMotion } from "$lib/motion/reduced.svelte";
  import CurveEditor from "./CurveEditor.svelte";
  import CodeBlock from "./CodeBlock.svelte";
  import PreviewLane, { type PreviewProp } from "./PreviewLane.svelte";
  import RangeField from "./RangeField.svelte";

  let { onuse, active = true }: { onuse?: (b: Bezier) => void; active?: boolean } = $props();

  const reduced = reducedMotion();
  const GROUPS: PresetGroup[] = ["css", "material", "apple", "expressive"];
  const PROPS: PreviewProp[] = ["x", "scale", "opacity", "rotate"];
  const GAP = 450;

  let a = $state<Bezier>([...DEFAULT_BEZIER]);
  let b = $state<Bezier>([0.16, 1, 0.3, 1]);
  let comparing = $state(false);
  let editing = $state<"a" | "b">("a");
  let prop = $state<PreviewProp>("x");
  let duration = $state(800);
  let playing = $state(!reduced.current);
  let elapsed = $state(0);
  let usedNote = $state(false);

  let presetA = $derived(findPreset(a));
  let presetB = $derived(findPreset(b));
  let fa = $derived(bezierEase(a));
  let fb = $derived(bezierEase(b));
  let phase = $derived(Math.min(1, elapsed / duration));
  let pa = $derived(fa(phase));
  let pb = $derived(fb(phase));

  $effect(() => {
    if (!playing || !active) return;
    let start = performance.now() - untrack(() => elapsed);
    const once = reduced.current;
    return loop((now) => {
      let e = now - start;
      if (e > duration + GAP) {
        if (once) {
          elapsed = duration;
          playing = false;
          return false;
        }
        start = now;
        e = 0;
      }
      elapsed = Math.min(e, duration + GAP);
    });
  });

  function togglePlay() {
    if (!playing && elapsed >= duration) elapsed = 0;
    playing = !playing;
  }

  function replay() {
    elapsed = 0;
    playing = true;
  }

  function applyPreset(value: Bezier) {
    if (comparing && editing === "b") b = [...value];
    else a = [...value];
    elapsed = 0;
  }

  function setValue(index: number, raw: string) {
    const n = Number(raw);
    if (!Number.isFinite(n)) return;
    const target = comparing && editing === "b" ? b : a;
    const next = target.slice() as Bezier;
    next[index] = n;
    if (comparing && editing === "b") b = clampBezier(next);
    else a = clampBezier(next);
  }

  function swap() {
    const tmp = a;
    a = b;
    b = tmp;
  }

  function useInTimeline() {
    onuse?.(current);
    usedNote = true;
    setTimeout(() => (usedNote = false), 2400);
  }

  let current = $derived(comparing && editing === "b" ? b : a);
  let presetLabel = (p: Bezier) => findPreset(p)?.label ?? ($t("motion.easing.custom") as string);
</script>

<div class="ep">
  <section class="mo-card ep-editor" aria-labelledby="ep-curve-title">
    <div class="mo-card-head">
      <h2 id="ep-curve-title" class="mo-card-title">{$t("motion.easing.curve_title")}</h2>
      <label class="mo-check">
        <input type="checkbox" bind:checked={comparing} />
        {$t("motion.easing.compare")}
      </label>
    </div>

    {#if comparing}
      <div class="segmented" role="radiogroup" aria-label={$t("motion.easing.editing")}>
        <button type="button" class="segmented-btn" role="radio" aria-checked={editing === "a"} onclick={() => (editing = "a")}>
          <span class="ep-dot tone-accent" aria-hidden="true"></span>{$t("motion.easing.curve_a")}
        </button>
        <button type="button" class="segmented-btn" role="radio" aria-checked={editing === "b"} onclick={() => (editing = "b")}>
          <span class="ep-dot tone-teal" aria-hidden="true"></span>{$t("motion.easing.curve_b")}
        </button>
      </div>
    {/if}

    {#if comparing && editing === "b"}
      <CurveEditor bind:value={b} ghost={a} label={$t("motion.easing.editor_label", { name: $t("motion.easing.curve_b") })} />
    {:else}
      <CurveEditor bind:value={a} ghost={comparing ? b : null} label={$t("motion.easing.editor_label", { name: $t("motion.easing.curve_a") })} />
    {/if}

    <div class="ep-values" role="group" aria-label={$t("motion.easing.values")}>
      {#each ["x1", "y1", "x2", "y2"] as name, i (name)}
        <label class="ep-value">
          <span>{name}</span>
          <input
            class="input-number"
            type="number"
            step="0.01"
            min={i % 2 === 0 ? 0 : Y_MIN}
            max={i % 2 === 0 ? 1 : Y_MAX}
            value={num(current[i], 3)}
            onchange={(e) => setValue(i, (e.currentTarget as HTMLInputElement).value)}
          />
        </label>
      {/each}
    </div>
    <p class="mo-meta">
      {presetLabel(current)}
      {#if overshoots(current)}· {$t("motion.easing.overshoot")}{/if}
    </p>
  </section>

  <section class="mo-card ep-presets" aria-labelledby="ep-presets-title">
    <h2 id="ep-presets-title" class="mo-card-title">{$t("motion.easing.presets")}</h2>
    {#each GROUPS as group (group)}
      <div class="ep-group">
        <h3 class="mo-sub">{$t(`motion.easing.group_${group}`)}</h3>
        <div class="ep-chips">
          {#each PRESETS.filter((p) => p.group === group) as p (p.id)}
            {@const active = (comparing && editing === "b" ? presetB : presetA)?.id === p.id}
            <button type="button" class="ep-chip" class:active aria-pressed={active} onclick={() => applyPreset(p.value)} title={cssBezier(p.value)}>
              <svg viewBox="-2 -12 28 40" width="18" height="24" aria-hidden="true">
                <path d="M0,24 C{p.value[0] * 24},{24 - p.value[1] * 24} {p.value[2] * 24},{24 - p.value[3] * 24} 24,0" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" />
              </svg>
              {p.label}
            </button>
          {/each}
        </div>
      </div>
    {/each}
  </section>

  <section class="mo-card ep-preview" aria-labelledby="ep-preview-title">
    <div class="mo-card-head">
      <h2 id="ep-preview-title" class="mo-card-title">{$t("motion.preview")}</h2>
      <div class="segmented" role="radiogroup" aria-label={$t("motion.easing.property")}>
        {#each PROPS as p (p)}
          <button type="button" class="segmented-btn" role="radio" aria-checked={prop === p} onclick={() => (prop = p)}>{$t(`motion.prop.${p}`)}</button>
        {/each}
      </div>
    </div>
    <div class="ep-lanes">
      <PreviewLane progress={pa} {prop} label={$t("motion.easing.lane_a")} />
      {#if comparing}
        <PreviewLane progress={pb} {prop} label={$t("motion.easing.lane_b")} tone="teal" />
      {/if}
    </div>
    <div class="mo-transport">
      <button type="button" class="btn btn-secondary btn-sm" onclick={togglePlay} aria-pressed={playing}>
        {playing ? $t("motion.pause") : $t("motion.play")}
      </button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={replay}>{$t("motion.replay")}</button>
      <div class="ep-duration">
        <RangeField id="ep-duration" label={$t("motion.duration")} bind:value={duration} min={100} max={3000} step={10} unit={$t("motion.unit_ms")} />
      </div>
    </div>
    {#if reduced.current}
      <p class="mo-note" role="note">{$t("motion.reduced_note")}</p>
    {/if}
    {#if comparing}
      <p class="mo-meta">{$t("motion.easing.race", { a: presetLabel(a), b: presetLabel(b) })}</p>
      <button type="button" class="btn btn-ghost btn-sm" onclick={swap}>{$t("motion.easing.swap")}</button>
    {/if}
  </section>

  <section class="mo-card ep-export" aria-labelledby="ep-export-title">
    <div class="mo-card-head">
      <h2 id="ep-export-title" class="mo-card-title">{$t("motion.export")}</h2>
      {#if onuse}
        <span class="mo-meta" role="status" aria-live="polite">{usedNote ? $t("motion.easing.used") : ""}</span>
        <button type="button" class="btn btn-secondary btn-sm" onclick={useInTimeline}>{$t("motion.easing.use_in_timeline")}</button>
      {/if}
    </div>
    <div class="mo-codes">
      <CodeBlock label="CSS" code={`${cssBezier(current)}\n${cssTransition(current, duration)}`} />
      <CodeBlock label="Motion" code={motionEase(current, duration)} hint={$t("motion.easing.motion_hint")} />
      <CodeBlock label="GSAP" code={gsapEase(current)} hint={$t("motion.easing.gsap_hint")} />
      <CodeBlock label="Svelte" code={svelteEase(current)} hint={$t("motion.easing.svelte_hint")} />
    </div>
  </section>
</div>

<style>
  .ep {
    display: grid;
    grid-template-columns: minmax(280px, 340px) 1fr;
    gap: var(--space-4);
  }

  .ep-editor {
    grid-row: span 2;
  }

  .ep-export {
    grid-column: 1 / -1;
  }

  .ep-values {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-2);
  }

  .ep-value {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .ep-value input {
    width: 100%;
    font-variant-numeric: tabular-nums;
  }

  .ep-dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
  }

  .tone-accent {
    background: var(--accent);
  }

  .tone-teal {
    background: var(--teal);
  }

  .ep-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .ep-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .ep-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text-muted);
    font-size: var(--text-xs);
    cursor: pointer;
  }

  .ep-chip:hover {
    color: var(--text);
    background: var(--fill-2);
  }

  .ep-chip.active {
    color: var(--accent-text);
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 var(--hairline) var(--accent);
  }

  .ep-chip:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .ep-lanes {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .ep-duration {
    flex: 1;
    min-width: 220px;
  }

  @media (max-width: 760px) {
    .ep {
      grid-template-columns: 1fr;
    }

    .ep-editor {
      grid-row: auto;
    }
  }
</style>
