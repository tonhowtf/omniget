<script lang="ts">
  /**
   * Tile de ícone da seção Tools: quadrado arredondado com gradiente e um
   * glifo branco em máscara. Aceita `brand:<nome>` (static/brands, Simple
   * Icons) ou `glyph:<nome>` (static/icons, Phosphor Fill), o mesmo esquema
   * do NavIcon da sidebar, só que em tamanho de Launchpad.
   */
  let {
    icon,
    from,
    to,
    via,
    size = 96,
    muted = false,
  }: { icon: string; from: string; to: string; via?: string; size?: number; muted?: boolean } = $props();

  let kind = $derived(icon.startsWith("brand:") ? "brand" : icon.startsWith("app:") ? "app" : "glyph");
  let name = $derived(icon.slice(icon.indexOf(":") + 1));
  let url = $derived(kind === "brand" ? `/brands/${name}.svg` : kind === "app" ? `/apps/${name}.png` : `/icons/${name}.svg`);
  let glyphSize = $derived(Math.round(size * (kind === "brand" ? 0.52 : 0.58)));
  let gradient = $derived(
    via ? `linear-gradient(135deg, ${from} 0%, ${via} 55%, ${to} 100%)` : `linear-gradient(180deg, ${from}, ${to})`,
  );
</script>

{#if kind === "app"}
  <img class="tool-app" class:muted src={url} alt="" width={size} height={size} draggable="false" aria-hidden="true" />
{:else}
  <span
    class="tool-icon"
    class:muted
    style:width="{size}px"
    style:height="{size}px"
    style:border-radius="{Math.round(size * 0.225)}px"
    style:background={gradient}
    aria-hidden="true"
  >
    <span class="tool-glyph" style:--glyph="url({url})" style:width="{glyphSize}px" style:height="{glyphSize}px"></span>
  </span>
{/if}

<style>
  .tool-icon {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    color: #fff;
    box-shadow:
      inset 0 0 0 0.5px rgba(255, 255, 255, 0.28),
      inset 0 1px 0 rgba(255, 255, 255, 0.22),
      inset 0 -2px 3px rgba(0, 0, 0, 0.14),
      0 1px 2px rgba(0, 0, 0, 0.18),
      0 6px 14px -6px rgba(0, 0, 0, 0.35);
    transition: transform var(--duration-fast) var(--ease-out), box-shadow var(--duration-fast) var(--ease-out);
  }

  /* brilho de topo, como os ícones de app do macOS */
  .tool-icon::before {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: linear-gradient(180deg, rgba(255, 255, 255, 0.18) 0%, rgba(255, 255, 255, 0) 55%);
    pointer-events: none;
  }

  .tool-app {
    display: block;
    flex-shrink: 0;
    object-fit: contain;
    filter: drop-shadow(0 6px 14px rgba(0, 0, 0, 0.3));
    transition: transform var(--duration-fast) var(--ease-out);
  }

  .tool-app.muted {
    filter: saturate(0.55) drop-shadow(0 6px 14px rgba(0, 0, 0, 0.3));
    opacity: 0.72;
  }

  .tool-icon.muted {
    filter: saturate(0.55);
    opacity: 0.72;
  }

  .tool-glyph {
    display: block;
    background: #fff;
    -webkit-mask: var(--glyph) center / contain no-repeat;
    mask: var(--glyph) center / contain no-repeat;
    filter: drop-shadow(0 1px 0 rgba(0, 0, 0, 0.2));
  }
</style>
