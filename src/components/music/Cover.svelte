<script lang="ts">
  import { coverUrl } from "$lib/music/library-store.svelte";
  import MusicIcon from "./MusicIcon.svelte";

  let {
    hash,
    seed = "",
    size = 48,
    radius = 6,
    full = false,
    round = false,
    icon = "note",
  }: {
    hash: string | null | undefined;
    seed?: string;
    size?: number | string;
    radius?: number;
    full?: boolean;
    round?: boolean;
    icon?: string;
  } = $props();

  let failed = $state(false);
  let src = $derived(hash && !failed ? coverUrl(hash, !full) : "");

  $effect(() => {
    void hash;
    failed = false;
  });

  function hue(s: string): number {
    let h = 0;
    for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
    return h % 360;
  }
  let h = $derived(hue(seed || "omniget"));
  let dim = $derived(typeof size === "number" ? `${size}px` : size);
</script>

<div
  class="cover"
  class:round
  style:width={dim}
  style:height={dim}
  style:border-radius={round ? "50%" : `${radius}px`}
  style:--h={h}
>
  {#if src}
    <img {src} alt="" loading="lazy" decoding="async" draggable="false" onerror={() => (failed = true)} />
  {:else}
    <span class="ph"><MusicIcon name={icon} size={typeof size === "number" ? Math.max(14, Math.round(size * 0.36)) : 28} /></span>
  {/if}
</div>

<style>
  .cover {
    position: relative;
    flex-shrink: 0;
    overflow: hidden;
    background: linear-gradient(135deg, hsl(var(--h) 45% 42%), hsl(calc(var(--h) + 40) 50% 26%));
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--secondary) 12%, transparent);
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .ph {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: hsl(var(--h) 60% 88% / 0.85);
  }
</style>
