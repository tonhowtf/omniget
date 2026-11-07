<script lang="ts">
  import type { Snippet } from "svelte";
  import KeyAnchor from "./KeyAnchor.svelte";

  let { keys, active = true, fill = false, children }: { keys: readonly string[]; active?: boolean; fill?: boolean; children: Snippet } = $props();
</script>

{#if keys.length === 0}
  {@render children()}
{:else}
  <span class="key-anchor" class:fill data-setting-key={active ? keys[0] : undefined}>
    <KeyAnchor keys={keys.slice(1)} {active} {fill}>{@render children()}</KeyAnchor>
  </span>
{/if}

<style>
  .key-anchor { display: inline-flex; align-items: center; justify-content: flex-end; min-inline-size: 0; }
  .key-anchor.fill { flex: 1 1 auto; inline-size: 100%; }
</style>
