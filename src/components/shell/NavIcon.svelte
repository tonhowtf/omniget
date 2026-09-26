<script lang="ts">
  /**
   * Sidebar icon. Core sections use the Loop sticker art in
   * static/icons/menu (32/64/128 px WebP, see assets/icons/menu for the
   * masters); ids without art fall back to the
   * macOS System Settings tile: a rounded square with a colour of its own and
   * a chunky white glyph (Phosphor Fill, MIT, shipped in static/icons).
   */
  let {
    icon,
    size = 22,
    active = false,
  }: { icon: string; size?: number; active?: boolean } = $props();

  // glyph file + tile gradient per nav id. Colours follow Apple's system
  // palette so the column reads like a native sidebar.
  const TILES: Record<string, { glyph: string; from: string; to: string }> = {
    home: { glyph: "house", from: "#5AA9FF", to: "#1E6FE8" },
    downloads: { glyph: "tray-arrow-down", from: "#FFB340", to: "#F28500" },
    chat: { glyph: "chats-circle", from: "#4CD964", to: "#2AA845" },
    llm: { glyph: "sparkle", from: "#C77DFF", to: "#7B3FE4" },
    help: { glyph: "book-open-text", from: "var(--accent)", to: "var(--accent)" },
    world: { glyph: "globe-hemisphere-west", from: "#67D27E", to: "#2F9E52" },
    superpowers: { glyph: "lightning", from: "#F06CB8", to: "#C42F86" },
    settings: { glyph: "gear-six", from: "#A3A3A8", to: "#6F6F75" },
    about: { glyph: "info", from: "#5AA9FF", to: "#1E6FE8" },
    league: { glyph: "sword", from: "#E8B84A", to: "#B8860B" },
    fallback: { glyph: "puzzle-piece", from: "#8E8E93", to: "#5C5C60" },
  };

  const ART = new Set([
    "home", "downloads", "llm", "help", "world",
    "superpowers", "settings", "about", "league",
  ]);

  let tile = $derived(TILES[icon] ?? TILES.fallback);
  let art = $derived(ART.has(icon));
  // 1x/2x pair picked from the rendered size: the 32 px file covers the rail
  // and the list at 1x, 64/128 cover retina and the Superpowers cards.
  let artSrc = $derived(size > 32 ? `/icons/menu/${icon}-64.webp` : `/icons/menu/${icon}-32.webp`);
  let artSrcset = $derived(
    size > 32
      ? `/icons/menu/${icon}-64.webp 1x, /icons/menu/${icon}-128.webp 2x`
      : `/icons/menu/${icon}-32.webp 1x, /icons/menu/${icon}-64.webp 2x`,
  );
  let glyphSize = $derived(Math.round(size * 0.64));
</script>

{#if art}
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
{:else}
<span
  class="nav-icon nav-tile"
  class:nav-icon-active={active}
  style:--tile-from={tile.from}
  style:--tile-to={tile.to}
  style:--tile-size="{size}px"
  aria-hidden="true"
>
  <span class="nav-glyph" style:--glyph="url(/icons/{tile.glyph}.svg)" style:width="{glyphSize}px" style:height="{glyphSize}px"></span>
</span>
{/if}

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

  .nav-tile {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--tile-size);
    height: var(--tile-size);
    flex-shrink: 0;
    border-radius: calc(var(--tile-size) * 0.28);
    background: linear-gradient(180deg, var(--tile-from), var(--tile-to));
    box-shadow:
      inset 0 0 0 0.5px rgba(255, 255, 255, 0.25),
      inset 0 -1px 1px rgba(0, 0, 0, 0.12),
      0 0.5px 1px rgba(0, 0, 0, 0.25);
    color: #fff;
  }

  .nav-glyph {
    display: block;
    background: #fff;
    -webkit-mask: var(--glyph) center / contain no-repeat;
    mask: var(--glyph) center / contain no-repeat;
    filter: drop-shadow(0 0.5px 0 rgba(0, 0, 0, 0.18));
  }
</style>
