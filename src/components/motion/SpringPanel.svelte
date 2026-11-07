<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { cssSpring, motionSpring, svelteSpring, swiftSpring } from "$lib/motion/exports";
  import { num } from "$lib/motion/format";
  import { linearStops, toLinear } from "$lib/motion/linear";
  import { loop, reducedMotion } from "$lib/motion/reduced.svelte";
  import { DEFAULT_SPRING, SPRING_PRESETS, simulateSpring, springAt, toSvelteSpring, toSwiftUI, type SpringParams } from "$lib/motion/spring";
  import CodeBlock from "./CodeBlock.svelte";
  import PreviewLane, { type PreviewProp } from "./PreviewLane.svelte";
  import RangeField from "./RangeField.svelte";

  let { active = true }: { active?: boolean } = $props();

  const reduced = reducedMotion();
  const PROPS: PreviewProp[] = ["x", "scale", "opacity", "rotate"];
  const GAP = 500;
  const W = 600;
  const H = 220;
  const PAD = 24;

  let params = $state<SpringParams>({ ...DEFAULT_SPRING });
  let tolerance = $state(0.002);
  let prop = $state<PreviewProp>("x");
  let playing = $state(!reduced.current);
  let elapsed = $state(0);

  let result = $derived(simulateSpring(params));
  let stops = $derived(linearStops(result.samples, tolerance));
  let linear = $derived(toLinear(result.samples, tolerance));
  let svelte = $derived(toSvelteSpring(params));
  let swift = $derived(toSwiftUI(params));
  let ratio = $derived(result.dampingRatio);
  let regime = $derived(ratio < 0.999 ? "under" : ratio <= 1.001 ? "critical" : "over");
  let overshoot = $derived(Math.max(0, result.max - 1));
  let activePreset = $derived(
    SPRING_PRESETS.find(
      (p) => p.params.stiffness === params.stiffness && p.params.damping === params.damping && p.params.mass === params.mass && p.params.velocity === params.velocity,
    )?.id ?? null,
  );

  let span = $derived(Math.max(100, result.settleMs));
  let lo = $derived(Math.min(0, result.min) - 0.05);
  let hi = $derived(Math.max(1, result.max) + 0.05);
  const sx = (ms: number) => PAD + (ms / span) * (W - PAD * 2);
  const sy = (x: number) => H - PAD - ((x - lo) / (hi - lo)) * (H - PAD * 2);
  let plot = $derived(result.samples.map((s, i) => `${i === 0 ? "M" : "L"}${sx(s.t).toFixed(1)},${sy(s.x).toFixed(1)}`).join(" "));
  let progress = $derived(springAt(result, elapsed));
  let head = $derived(Math.min(elapsed, span));

  $effect(() => {
    if (!playing || !active) return;
    const total = span;
    const once = reduced.current;
    let start = performance.now() - untrack(() => elapsed);
    return loop((now) => {
      let e = now - start;
      if (e > total + GAP) {
        if (once) {
          elapsed = total;
          playing = false;
          return false;
        }
        start = now;
        e = 0;
      }
      elapsed = e;
    });
  });

  function togglePlay() {
    if (!playing && elapsed >= span) elapsed = 0;
    playing = !playing;
  }

  function replay() {
    elapsed = 0;
    playing = true;
  }

  function applyPreset(p: SpringParams) {
    params = { ...p };
    elapsed = 0;
  }
</script>

<div class="sp">
  <section class="mo-card" aria-labelledby="sp-params-title">
    <h2 id="sp-params-title" class="mo-card-title">{$t("motion.spring.params")}</h2>
    <div class="sp-presets" role="group" aria-label={$t("motion.spring.presets")}>
      {#each SPRING_PRESETS as p (p.id)}
        <button type="button" class="sp-chip" class:active={activePreset === p.id} aria-pressed={activePreset === p.id} onclick={() => applyPreset(p.params)}>
          {$t(`motion.spring.preset_${p.id}`)}
        </button>
      {/each}
    </div>
    <RangeField id="sp-stiffness" label={$t("motion.spring.stiffness")} bind:value={params.stiffness} min={1} max={1000} step={1} />
    <RangeField id="sp-damping" label={$t("motion.spring.damping")} bind:value={params.damping} min={0} max={100} step={0.5} />
    <RangeField id="sp-mass" label={$t("motion.spring.mass")} bind:value={params.mass} min={0.1} max={10} step={0.1} />
    <RangeField id="sp-velocity" label={$t("motion.spring.velocity")} bind:value={params.velocity} min={-50} max={50} step={0.5} unit={$t("motion.unit_per_s")} />
    <dl class="sp-stats">
      <div>
        <dt>{$t("motion.spring.settle")}</dt>
        <dd>
          {#if result.settled}
            {$t("motion.ms_value", { value: Math.round(result.settleMs) })}
          {:else}
            <span class="sp-warn">
              <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" aria-hidden="true"><path d="M8 2.2 14.3 13H1.7z" /><path d="M8 6.4v3M8 11.2v.1" /></svg>
              {$t("motion.spring.never")}
            </span>
          {/if}
        </dd>
      </div>
      <div>
        <dt>{$t("motion.spring.ratio")}</dt>
        <dd>{num(ratio, 3)} · {$t(`motion.spring.regime_${regime}`)}</dd>
      </div>
      <div>
        <dt>{$t("motion.spring.overshoot")}</dt>
        <dd>{num(overshoot * 100, 1)}%</dd>
      </div>
    </dl>
    {#if !result.settled}
      <p class="mo-note" role="note">{$t("motion.spring.never_body")}</p>
    {/if}
  </section>

  <section class="mo-card" aria-labelledby="sp-plot-title">
    <div class="mo-card-head">
      <h2 id="sp-plot-title" class="mo-card-title">{$t("motion.spring.curve")}</h2>
      <span class="mo-meta">{$t("motion.spring.tolerance_band")}</span>
    </div>
    <svg class="sp-plot" viewBox="0 0 {W} {H}" role="img" aria-label={$t("motion.spring.plot_label", { ms: Math.round(result.settleMs) })}>
      <rect class="sp-band" x={PAD} y={sy(1.02)} width={W - PAD * 2} height={sy(0.98) - sy(1.02)} />
      <line class="sp-axis" x1={PAD} x2={W - PAD} y1={sy(0)} y2={sy(0)} />
      <line class="sp-target" x1={PAD} x2={W - PAD} y1={sy(1)} y2={sy(1)} />
      <path class="sp-line" d={plot} />
      {#each stops as s, i (i)}
        <circle class="sp-stop" cx={sx(s.t * span)} cy={sy(s.x)} r="2.2" />
      {/each}
      {#if elapsed > 0}
        <line class="sp-head" x1={sx(head)} x2={sx(head)} y1={PAD / 2} y2={H - PAD / 2} />
      {/if}
      <text class="sp-tick" x={PAD} y={H - 6}>0</text>
      <text class="sp-tick" x={W - PAD} y={H - 6} text-anchor="end">{$t("motion.ms_value", { value: Math.round(span) })}</text>
      <text class="sp-tick" x={PAD - 4} y={sy(1) + 4} text-anchor="end">1</text>
    </svg>
    <div class="segmented" role="radiogroup" aria-label={$t("motion.easing.property")}>
      {#each PROPS as p (p)}
        <button type="button" class="segmented-btn" role="radio" aria-checked={prop === p} onclick={() => (prop = p)}>{$t(`motion.prop.${p}`)}</button>
      {/each}
    </div>
    <PreviewLane {progress} {prop} label={$t("motion.spring.lane")} />
    <div class="mo-transport">
      <button type="button" class="btn btn-secondary btn-sm" onclick={togglePlay} aria-pressed={playing}>{playing ? $t("motion.pause") : $t("motion.play")}</button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={replay}>{$t("motion.replay")}</button>
    </div>
    {#if reduced.current}
      <p class="mo-note" role="note">{$t("motion.reduced_note")}</p>
    {/if}
  </section>

  <section class="mo-card sp-export" aria-labelledby="sp-export-title">
    <h2 id="sp-export-title" class="mo-card-title">{$t("motion.export")}</h2>
    <RangeField id="sp-tol" label={$t("motion.spring.tolerance")} bind:value={tolerance} min={0.0005} max={0.02} step={0.0005} />
    <p class="mo-meta">{$t("motion.spring.points", { count: stops.length, ms: Math.round(result.settleMs) })}</p>
    <div class="mo-codes">
      <CodeBlock label="CSS linear()" code={`--spring: ${linear};\n${cssSpring("var(--spring)", result.settleMs)}`} hint={$t("motion.spring.linear_hint")} />
      <CodeBlock label="Motion" code={motionSpring(params)} hint={params.velocity !== 0 ? $t("motion.spring.motion_hint_velocity") : $t("motion.spring.motion_hint")} />
      <CodeBlock
        label="Svelte"
        code={`import { Spring } from "svelte/motion";\n\nconst value = ${svelteSpring(params)};`}
        hint={$t(svelte.clamped ? "motion.spring.svelte_clamped" : "motion.spring.svelte_hint", {
          k: num(params.stiffness, 1),
          c: num(params.damping, 1),
          m: num(params.mass, 2),
        })}
      />
      <CodeBlock label="SwiftUI" code={swiftSpring(params)} hint={$t("motion.spring.swift_hint", { response: num(swift.response, 3), fraction: num(swift.dampingFraction, 3) })} />
    </div>
  </section>
</div>

<style>
  .sp {
    display: grid;
    grid-template-columns: minmax(300px, 380px) 1fr;
    gap: var(--space-4);
  }

  .sp-export {
    grid-column: 1 / -1;
  }

  .sp-presets {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .sp-chip {
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text-muted);
    font-size: var(--text-xs);
    cursor: pointer;
  }

  .sp-chip:hover {
    color: var(--text);
    background: var(--fill-2);
  }

  .sp-chip.active {
    color: var(--accent-text);
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 var(--hairline) var(--accent);
  }

  .sp-chip:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .sp-stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(96px, 1fr));
    gap: var(--space-2);
    margin: 0;
  }

  .sp-stats div {
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }

  .sp-stats dt {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .sp-stats dd {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }

  .sp-warn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--warning);
  }

  .sp-plot {
    width: 100%;
    height: auto;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }

  .sp-band {
    fill: var(--accent-soft);
  }

  .sp-axis {
    stroke: var(--separator);
  }

  .sp-target {
    stroke: var(--text-faint);
    stroke-dasharray: 4 4;
  }

  .sp-line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 2.5;
    stroke-linejoin: round;
  }

  .sp-stop {
    fill: var(--teal);
  }

  .sp-head {
    stroke: var(--text-muted);
    stroke-width: 1;
  }

  .sp-tick {
    font-size: 10px;
    fill: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  @media (max-width: 760px) {
    .sp {
      grid-template-columns: 1fr;
    }
  }
</style>
