<script lang="ts">
  import { onMount } from "svelte";
  import { orb } from "./orb";
  import { tone, type Activity } from "./limits";

  let {
    outer,
    inner,
    glyph,
    activity,
    theme,
    small = false,
    pace = null,
  }: {
    outer: number | null;
    inner: number | null;
    glyph: string;
    activity: Activity;
    theme: "dark" | "light";
    small?: boolean;
    pace?: { at: number; arc: "outer" | "inner" } | null;
  } = $props();

  let shown = $state(false);
  onMount(() => {
    const id = requestAnimationFrame(() => requestAnimationFrame(() => (shown = true)));
    return () => cancelAnimationFrame(id);
  });

  const clamp = (v: number | null) => Math.max(0, Math.min(1, v ?? 0));
  let two = $derived(inner != null);
  let busy = $derived(!small && (activity === "working" || activity === "waiting"));
  let tick = $derived.by(() => {
    if (!pace) return null;
    const [r, w] = !two ? [13.6, 3.6] : pace.arc === "inner" ? [10.9, 2.4] : [14.4, 3];
    const a = clamp(pace.at) * 2 * Math.PI;
    const [from, to] = [r - w / 2 - 0.9, r + w / 2 + 0.9];
    return { x1: 16 + from * Math.cos(a), y1: 16 + from * Math.sin(a), x2: 16 + to * Math.cos(a), y2: 16 + to * Math.sin(a) };
  });
</script>

<span class="lr" class:small class:two>
  <svg viewBox="0 0 32 32" aria-hidden="true">
    {#if two}
      <circle class="track" cx="16" cy="16" r="14.4" />
      <circle
        class="arc t-{tone(outer)}"
        class:zero={!shown || clamp(outer) < 0.005}
        cx="16"
        cy="16"
        r="14.4"
        pathLength="100"
        style:stroke-dashoffset={shown ? 100 - clamp(outer) * 100 : 100}
      />
      <circle class="track in" cx="16" cy="16" r="10.9" />
      <circle
        class="arc in t-{tone(inner)}"
        class:zero={!shown || clamp(inner) < 0.005}
        cx="16"
        cy="16"
        r="10.9"
        pathLength="100"
        style:stroke-dashoffset={shown ? 100 - clamp(inner) * 100 : 100}
      />
    {:else}
      <circle class="track solo" cx="16" cy="16" r="13.6" />
      {#if outer != null}
        <circle
          class="arc solo t-{tone(outer)}"
          class:zero={!shown || clamp(outer) < 0.005}
          cx="16"
          cy="16"
          r="13.6"
          pathLength="100"
          style:stroke-dashoffset={shown ? 100 - clamp(outer) * 100 : 100}
        />
      {/if}
    {/if}
    {#if tick}
      <line class="tick-under" x1={tick.x1} y1={tick.y1} x2={tick.x2} y2={tick.y2} />
      <line class="tick" x1={tick.x1} y1={tick.y1} x2={tick.x2} y2={tick.y2} />
    {/if}
  </svg>
  {#if busy}
    <canvas
      class="orb"
      aria-hidden="true"
      use:orb={{ state: activity === "working" ? "working" : "breathing", size: 20, theme }}
    ></canvas>
  {:else}
    <span class="glyph">{glyph}</span>
  {/if}
</span>

<style>
  .lr {
    position: relative;
    display: block;
    width: 32px;
    height: 32px;
  }
  .lr.small {
    width: 18px;
    height: 18px;
  }
  svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    transform: rotate(-90deg);
    overflow: visible;
  }
  .track {
    fill: none;
    stroke: var(--track);
    stroke-width: 3;
  }
  .track.in {
    stroke-width: 2.4;
  }
  .track.solo {
    stroke-width: 3.6;
  }
  .arc {
    fill: none;
    stroke: var(--tone);
    stroke-width: 3;
    stroke-linecap: round;
    stroke-dasharray: 100 100;
    transition:
      stroke-dashoffset 900ms cubic-bezier(0.22, 1.2, 0.36, 1),
      stroke 260ms ease,
      opacity 200ms ease;
  }
  .arc.in {
    stroke-width: 2.4;
    transition-delay: 90ms, 0ms, 0ms;
  }
  .arc.solo {
    stroke-width: 3.6;
  }
  .arc.zero {
    opacity: 0;
  }
  .tick,
  .tick-under {
    stroke-linecap: round;
  }
  .tick {
    stroke: var(--ink);
    stroke-width: 1.2;
  }
  .tick-under {
    stroke: var(--glass);
    stroke-width: 2.6;
  }
  .t-calm {
    --tone: var(--calm);
  }
  .t-amber {
    --tone: var(--amber);
  }
  .t-red {
    --tone: var(--red);
  }
  .glyph {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    font-size: 8.5px;
    font-weight: 750;
    letter-spacing: 0.01em;
    color: var(--ink);
  }
  .small .glyph {
    display: none;
  }
  .orb {
    position: absolute;
    left: 6px;
    top: 6px;
    display: block;
  }
  @media (prefers-reduced-motion: reduce) {
    .arc {
      transition: stroke 120ms linear;
    }
  }
</style>
