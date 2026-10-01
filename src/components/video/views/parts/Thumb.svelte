<script lang="ts">
  import { hue, img } from "./util";
  import VIcon from "./VIcon.svelte";

  let {
    src,
    seed = "",
    ratio = "16 / 9",
    icon = "film",
    radius = "var(--radius-md)",
    fit = "cover",
  }: {
    src: string | null | undefined;
    seed?: string;
    ratio?: string;
    icon?: string;
    radius?: string;
    fit?: "cover" | "contain";
  } = $props();

  let failed = $state(false);
  let url = $derived(!failed ? img(src) : null);
  let h = $derived(hue(seed || "omniget"));

  $effect(() => {
    void src;
    failed = false;
  });
</script>

<div class="th" style:aspect-ratio={ratio} style:border-radius={radius} style:--h={h}>
  {#if url}
    <img src={url} alt="" loading="lazy" decoding="async" draggable="false" style:object-fit={fit} onerror={() => (failed = true)} />
  {:else}
    <span class="ph"><VIcon name={icon} size={26} /></span>
  {/if}
</div>

<style>
  .th {
    position: relative;
    width: 100%;
    overflow: hidden;
    flex-shrink: 0;
    background: linear-gradient(135deg, hsl(var(--h) 38% 40%), hsl(calc(var(--h) + 40) 45% 22%));
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  img {
    width: 100%;
    height: 100%;
    display: block;
  }
  .ph {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: hsl(var(--h) 60% 88% / 0.8);
  }
</style>
