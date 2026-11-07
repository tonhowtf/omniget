<script lang="ts">
  import { initials } from "./format";

  let { src = null, name, size = 32, square = false }: { src?: string | null; name: string; size?: number; square?: boolean } = $props();

  let broken = $state(false);
  let hue = $derived([...name].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7));
</script>

<span
  class="dca"
  class:square
  style:width="{size}px"
  style:height="{size}px"
  style:font-size="{Math.max(10, Math.round(size * 0.38))}px"
  style:--dca-hue={hue}
  aria-hidden="true"
>
  {#if src && !broken}
    <img {src} alt="" loading="lazy" decoding="async" onerror={() => (broken = true)} />
  {:else}
    {initials(name)}
  {/if}
</span>

<style>
  .dca {
    display: inline-grid;
    place-items: center;
    flex-shrink: 0;
    overflow: hidden;
    border-radius: var(--radius-full);
    font-weight: 650;
    color: #fff;
    background: linear-gradient(160deg, hsl(var(--dca-hue) 62% 62%), hsl(var(--dca-hue) 58% 44%));
    user-select: none;
  }

  .dca.square {
    border-radius: 30%;
  }

  .dca img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
</style>
