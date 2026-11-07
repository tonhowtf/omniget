<script lang="ts">
  let {
    icon,
    size = 22,
    active = false,
  }: { icon: string; size?: number; active?: boolean } = $props();

  const TILES: Record<string, { glyph: string; from: string; to: string }> = {
    home: { glyph: "house", from: "#5AA9FF", to: "#1E6FE8" },
    downloads: { glyph: "tray-arrow-down", from: "#FFB340", to: "#F28500" },
    community: { glyph: "chats", from: "#C77DFF", to: "#7B3FE4" },
    help: { glyph: "book-open-text", from: "var(--accent)", to: "var(--accent)" },
    world: { glyph: "globe-hemisphere-west", from: "#67D27E", to: "#2F9E52" },
    settings: { glyph: "gear-six", from: "#A3A3A8", to: "#6F6F75" },
    league: { glyph: "sword", from: "#E8B84A", to: "#B8860B" },
    music: { glyph: "puzzle-piece", from: "#FF6B8B", to: "#E0245E" },
    washmybelly: { glyph: "sparkle", from: "#FF9BC2", to: "#E0527F" },
    video: { glyph: "puzzle-piece", from: "#FF8A3D", to: "#E5383B" },
    global: { glyph: "globe-hemisphere-west", from: "#40C8E0", to: "#0A7EA8" },
    telegram: { glyph: "puzzle-piece", from: "#2AABEE", to: "#229ED9" },
    read: { glyph: "book-open-text", from: "#FFB86B", to: "#E0712A" },
    converter: { glyph: "stack", from: "#5EDBC8", to: "#14907F" },
    pdf: { glyph: "file-pdf", from: "#FF6B5B", to: "#D93025" },
    photos: { glyph: "image", from: "#FFD36B", to: "#F2994A" },
    cutout: { glyph: "scissors", from: "#7AD7FF", to: "#2E7DF6" },
    palette: { glyph: "palette", from: "#FF7AB6", to: "#7B5CFF" },
    fonts: { glyph: "text-aa", from: "#9B8CFF", to: "#5B4BD6" },
    icons: { glyph: "shapes", from: "#FF9F0A", to: "#AF52DE" },
    motion: { glyph: "wave-sine", from: "#FF9F6B", to: "#E8457A" },
    board: { glyph: "chalkboard-simple", from: "#B197FC", to: "#6741D9" },
    components: { glyph: "squares-four", from: "#8B7CFF", to: "#5B3FD9" },
    capture: { glyph: "record", from: "#FF453A", to: "#FF9F0A" },
    studio: { glyph: "film-strip", from: "#FF7A59", to: "#C2185B" },
    dictation: { glyph: "microphone", from: "#7FB2FF", to: "#3A5BD9" },
    whatsapp: { glyph: "whatsapp-logo", from: "#25D366", to: "#128C7E" },
    discord: { glyph: "discord-logo", from: "#7984F5", to: "#4752C4" },
    x: { glyph: "x-logo", from: "#3A3A3C", to: "#000000" },
    instagram: { glyph: "instagram-logo", from: "#F77737", to: "#C13584" },
    tiktok: { glyph: "tiktok-logo", from: "#25F4EE", to: "#FE2C55" },
    fallback: { glyph: "puzzle-piece", from: "#8E8E93", to: "#5C5C60" },
  };

  const ART = new Set([
    "home", "downloads", "help", "world",
    "superpowers", "settings", "community", "league", "music", "washmybelly", "video",
    "telegram", "tube", "read", "global",
  ]);

  let tile = $derived(TILES[icon] ?? TILES.fallback);
  let art = $derived(ART.has(icon));
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
