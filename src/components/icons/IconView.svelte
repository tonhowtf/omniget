<script lang="ts">
  import { buildSvg, dataUri, dimensions, parts, type BuildOptions } from "$lib/icons/svg";
  import type { IconData } from "$lib/icons/types";

  let {
    icon,
    size = 24,
    palette = false,
    opts = {},
  }: { icon: IconData; size?: number; palette?: boolean; opts?: BuildOptions } = $props();

  let box = $derived(dimensions(parts(icon, opts), size));
  let uri = $derived(dataUri(buildSvg(icon, { ...opts, size, color: palette ? opts.color : "#000" })));
</script>

{#if palette}
  <img class="ic-view" src={uri} width={box.width} height={box.height} alt="" draggable="false" />
{:else}
  <span class="ic-view ic-mask" style:width="{box.width}px" style:height="{box.height}px" style:--ic-mask={`url("${uri}")`} aria-hidden="true"></span>
{/if}

<style>
  .ic-view {
    display: block;
    flex-shrink: 0;
  }

  .ic-mask {
    background-color: currentColor;
    -webkit-mask: var(--ic-mask) center / contain no-repeat;
    mask: var(--ic-mask) center / contain no-repeat;
  }
</style>
