<script lang="ts">
  let {
    icon,
    size = 22,
    active = false,
  }: { icon: string; size?: number; active?: boolean } = $props();

  let artSrc = $derived(size > 32 ? `/icons/menu/${icon}-64.webp` : `/icons/menu/${icon}-32.webp`);
  let artSrcset = $derived(
    size > 32
      ? `/icons/menu/${icon}-64.webp 1x, /icons/menu/${icon}-128.webp 2x`
      : `/icons/menu/${icon}-32.webp 1x, /icons/menu/${icon}-64.webp 2x`,
  );
</script>

<img
  class="nav-icon nav-art"
  class:nav-icon-active={active}
  src={artSrc}
  srcset={artSrcset}
  width={size}
  height={size}
  alt=""
  aria-hidden="true"
  draggable="false"
  decoding="async"
/>

<style>
  .nav-art {
    display: block;
    flex-shrink: 0;
    object-fit: contain;
    user-select: none;
    -webkit-user-drag: none;
    transition: transform var(--duration-fast, 120ms) var(--ease-out, ease-out);
  }

  :global(.mac-nav-item:hover) .nav-art:not(.nav-icon-active) {
    transform: scale(1.06) rotate(-3deg);
  }

  @media (prefers-reduced-motion: reduce) {
    .nav-art { transition: none; }
    :global(.mac-nav-item:hover) .nav-art:not(.nav-icon-active) { transform: none; }
  }
</style>
