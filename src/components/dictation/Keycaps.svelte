<script lang="ts">
  import { shortcutCaps } from "$lib/dictation/logic";
  import type { Platform } from "$lib/dictation/types";

  let { combo, platform, size = "md" }: { combo: string; platform: Platform; size?: "sm" | "md" } = $props();
  let caps = $derived(shortcutCaps(combo, platform));
</script>

<span class="dc-caps dc-caps-{size}">
  {#each caps as cap, i (i)}
    <kbd class="kbd">{cap}</kbd>
  {/each}
</span>

<style>
  .dc-caps {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    white-space: nowrap;
  }

  .dc-caps-md :global(.kbd) {
    min-width: 22px;
    font-size: var(--text-sm);
  }

  .dc-caps-sm :global(.kbd) {
    font-size: var(--text-xs);
  }
</style>
