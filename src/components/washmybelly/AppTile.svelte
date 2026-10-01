<script lang="ts">
  import { monogram, sourceInfo } from "$lib/washmybelly/apps";

  let { name, source, size = 32, dim = false }: { name: string; source: string; size?: number; dim?: boolean } = $props();

  let info = $derived(sourceInfo(source));
  let letters = $derived(monogram(name));
</script>

<span
  class="wmb-app-tile"
  class:dim
  style:--from={info.from}
  style:--to={info.to}
  style:width="{size}px"
  style:height="{size}px"
  style:border-radius="{Math.round(size * 0.225)}px"
  style:font-size="{Math.round(size * (letters.length > 1 ? 0.36 : 0.46))}px"
  aria-hidden="true">{letters}</span
>

<style>
  .wmb-app-tile {
    display: inline-grid;
    place-items: center;
    flex-shrink: 0;
    background: linear-gradient(180deg, var(--from), var(--to));
    color: #fff;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1;
    text-shadow: 0 1px 1px rgb(0 0 0 / 0.18);
    box-shadow:
      inset 0 0 0 0.5px rgb(0 0 0 / 0.14),
      inset 0 1px 0 rgb(255 255 255 / 0.2),
      0 1px 2px rgb(0 0 0 / 0.12);
    user-select: none;
  }
  .wmb-app-tile.dim {
    filter: grayscale(0.85);
    opacity: 0.55;
  }
</style>
