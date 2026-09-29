<script lang="ts">
  // Subtitles drawn in HTML (not <track>) so size, background, delay and a
  // second language can be changed live.
  import { cueAt } from "$lib/video/format";
  import { player } from "$lib/video/player.svelte";

  let { lifted = false }: { lifted?: boolean } = $props();

  let ms = $derived(player.position * 1000 - player.subs.delayMs);
  let i1 = $derived(cueAt(player.cues, ms));
  let i2 = $derived(cueAt(player.cues2, ms));
  let line1 = $derived(i1 >= 0 ? player.cues[i1].text : "");
  let line2 = $derived(i2 >= 0 ? player.cues2[i2].text : "");
</script>

{#if line1 || line2}
  <div
    class="subs"
    class:bg={player.subs.bg}
    style:bottom={lifted ? `calc(${player.subs.pos}% + 96px)` : `${player.subs.pos}%`}
    style:--sub-scale={player.subs.size / 100}
    aria-live="off"
  >
    {#if line2}<p class="l2">{line2}</p>{/if}
    {#if line1}<p class="l1">{line1}</p>{/if}
  </div>
{/if}

<style>
  .subs {
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    width: min(88%, 1100px);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    pointer-events: none;
    text-align: center;
    transition: bottom var(--duration-base) var(--ease-out);
    z-index: 3;
  }
  p {
    margin: 0;
    padding: 0.08em 0.4em;
    border-radius: var(--radius-sm);
    color: var(--pl-ink);
    font-family: var(--font-body);
    font-weight: 600;
    line-height: 1.3;
    white-space: pre-line;
    text-shadow:
      0 0 3px var(--pl-shade),
      0 1px 2px var(--pl-shade),
      0 0 1px var(--pl-shade);
  }
  .l1 {
    font-size: calc(clamp(16px, 2.6vw, 34px) * var(--sub-scale));
  }
  .l2 {
    font-size: calc(clamp(13px, 2vw, 26px) * var(--sub-scale));
    color: color-mix(in srgb, var(--warning) 70%, var(--pl-ink));
    font-weight: 500;
  }
  .bg p {
    background: color-mix(in srgb, var(--pl-shade) 62%, transparent);
    text-shadow: none;
  }
</style>
