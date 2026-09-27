<script lang="ts">
  // Station favicons come from third-party sites and often 404; a broken one
  // is replaced by the placeholder instead of leaving a failed <img> around.
  import Cover from "./Cover.svelte";

  let { src, name, size = 44, radius = 8 }: { src: string; name: string; size?: number; radius?: number } = $props();

  let failed = $state(false);
  $effect(() => {
    void src;
    failed = false;
  });
</script>

{#if src && !failed}
  <img
    {src}
    alt=""
    loading="lazy"
    referrerpolicy="no-referrer"
    style:width="{size}px"
    style:height="{size}px"
    style:border-radius="{radius}px"
    onerror={() => (failed = true)}
  />
{:else}
  <Cover hash={null} seed={name} {size} {radius} icon="radio" />
{/if}

<style>
  img {
    display: block;
    flex-shrink: 0;
    object-fit: cover;
    background: white;
  }
</style>
