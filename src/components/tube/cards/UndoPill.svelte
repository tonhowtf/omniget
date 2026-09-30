<script lang="ts">
  // "Not interested · Undo" pill with a draining ring (21st "Undo Pill").
  // Hover or focus pauses the countdown; when it runs out the action commits.
  import { t } from "$lib/i18n";
  import TIcon from "../TIcon.svelte";
  import { feedback, takeUndo } from "./feedback.svelte";

  const TOTAL = 5000;
  const R = 8;
  const C = 2 * Math.PI * R;

  let left = $state(TOTAL);
  let paused = $state(false);
  let entry = $derived(feedback.undo);

  $effect(() => {
    const id = entry?.id;
    if (id == null) return;
    left = TOTAL;
    let last = performance.now();
    let raf = 0;
    const tick = (now: number) => {
      if (!paused) left -= now - last;
      last = now;
      if (left <= 0) {
        const e = takeUndo();
        if (e && e.id === id) void e.commit();
        return;
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

  async function undo() {
    const e = takeUndo();
    if (e) await e.undo();
  }

  function dismiss() {
    const e = takeUndo();
    if (e) void e.commit();
  }
</script>

{#if entry}
  {#key entry.id}
    <div
      class="pill"
      role="status"
      aria-live="polite"
      onmouseenter={() => (paused = true)}
      onmouseleave={() => (paused = false)}
      onfocusin={() => (paused = true)}
      onfocusout={() => (paused = false)}
    >
      <svg class="ring" width="22" height="22" viewBox="0 0 22 22" aria-hidden="true">
        <circle cx="11" cy="11" r={R} class="bg" />
        <circle cx="11" cy="11" r={R} class="fg" stroke-dasharray={C} stroke-dashoffset={C * (1 - Math.max(0, left) / TOTAL)} />
      </svg>
      <span class="count">{Math.ceil(Math.max(0, left) / 1000)}</span>
      <span class="text">{entry.text}</span>
      <button class="undo" onclick={undo}>{$t("tube.common.undo")}</button>
      <button class="x" onclick={dismiss} aria-label={$t("tube.common.close") as string}><TIcon name="close" size={16} /></button>
    </div>
  {/key}
{/if}

<style>
  .pill {
    position: fixed;
    left: 50%;
    bottom: 24px;
    z-index: 1500;
    display: inline-flex;
    align-items: center;
    gap: 10px;
    height: 44px;
    padding: 0 8px 0 12px;
    border-radius: 999px;
    color: var(--yt-on-inverted);
    background: var(--yt-bg-inverted);
    box-shadow: 0 12px 32px var(--yt-shadow);
    font: var(--yt-type-body-strong);
    transform: translateX(-50%);
    animation: pop-in 260ms var(--yt-ease-standard);
  }
  .ring {
    transform: rotate(-90deg);
    flex: none;
  }
  .ring circle {
    fill: none;
    stroke-width: 2.5;
  }
  .ring .bg {
    stroke: color-mix(in srgb, var(--yt-on-inverted) 20%, transparent);
  }
  .ring .fg {
    stroke: var(--yt-on-inverted);
    stroke-linecap: round;
  }
  .count {
    position: absolute;
    left: 12px;
    width: 22px;
    text-align: center;
    font: 600 9px/22px var(--yt-font);
    font-variant-numeric: tabular-nums;
  }
  .text {
    white-space: nowrap;
  }
  .undo {
    height: 32px;
    padding: 0 12px;
    border: 0;
    border-radius: 999px;
    font: var(--yt-type-body-strong);
    color: var(--yt-link);
    background: transparent;
    cursor: pointer;
  }
  .undo:hover {
    background: color-mix(in srgb, var(--yt-on-inverted) 10%, transparent);
  }
  .x {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: 50%;
    color: inherit;
    background: transparent;
    cursor: pointer;
  }
  .x:hover {
    background: color-mix(in srgb, var(--yt-on-inverted) 10%, transparent);
  }
  .undo:focus-visible,
  .x:focus-visible {
    outline: 2px solid var(--yt-link);
  }
  @keyframes pop-in {
    from {
      opacity: 0;
      transform: translate(-50%, 16px) scale(0.96);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pill {
      animation: none;
    }
  }
</style>
