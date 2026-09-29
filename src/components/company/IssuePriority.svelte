<script lang="ts">
  /** Priority glyph (Linear-style bars, a bang for critical) with its color. */
  import { t } from "$lib/i18n";
  import { priorityColor, priorityKey } from "./company-format";

  let { priority, label = false }: { priority: string; label?: boolean } = $props();
  let bars = $derived(priority === "high" ? 3 : priority === "medium" ? 2 : 1);
</script>

<span class="prio" style:color={priorityColor(priority)} title={$t(priorityKey(priority))}>
  {#if priority === "critical"}
    <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
      <rect x="1" y="1" width="12" height="12" rx="3" fill="currentColor" />
      <rect x="6.2" y="3.5" width="1.6" height="4.6" rx="0.8" fill="var(--surface)" />
      <circle cx="7" cy="10.2" r="0.9" fill="var(--surface)" />
    </svg>
  {:else}
    <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
      {#each [0, 1, 2] as i (i)}
        <rect
          x={1.5 + i * 4}
          y={9 - i * 3}
          width="3"
          height={4 + i * 3}
          rx="1"
          fill="currentColor"
          opacity={i < bars ? 1 : 0.25}
        />
      {/each}
    </svg>
  {/if}
  {#if label}<span class="text">{$t(priorityKey(priority))}</span>{/if}
</span>

<style>
  .prio {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    flex-shrink: 0;
  }
  .text {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
</style>
