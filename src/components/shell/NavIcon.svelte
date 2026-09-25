<script lang="ts">
  /**
   * Sidebar icon tile in the macOS System Settings idiom: a small rounded
   * square with a colour of its own and a chunky white glyph (Phosphor Fill,
   * MIT, shipped in static/icons).
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
    llm: { glyph: "sparkle", from: "#C77DFF", to: "#7B3FE4" },
    help: { glyph: "book-open-text", from: "var(--accent)", to: "var(--accent)" },
    world: { glyph: "globe-hemisphere-west", from: "#67D27E", to: "#2F9E52" },
    settings: { glyph: "gear-six", from: "#A3A3A8", to: "#6F6F75" },
    about: { glyph: "info", from: "#5AA9FF", to: "#1E6FE8" },
    league: { glyph: "sword", from: "#E8B84A", to: "#B8860B" },
    tools: { glyph: "toolbox", from: "#FF9F5A", to: "#E8641A" },
    fallback: { glyph: "puzzle-piece", from: "#8E8E93", to: "#5C5C60" },
  };

  let tile = $derived(TILES[icon] ?? TILES.fallback);
  let glyphSize = $derived(Math.round(size * 0.64));
</script>

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

<style>
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
