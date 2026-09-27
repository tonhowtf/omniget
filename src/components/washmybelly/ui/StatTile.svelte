<script lang="ts">
  import type { Snippet } from "svelte";

  /// A number with its label. `tone` colours the value; `onClick` makes the
  /// whole tile a button; `trailing` holds a sparkline or a ring.
  let {
    label,
    value,
    sub = "",
    unit = "",
    tone = "default",
    onClick,
    trailing,
  }: {
    label: string;
    value: string;
    sub?: string;
    unit?: string;
    tone?: "default" | "good" | "warn" | "bad" | "accent";
    onClick?: () => void;
    trailing?: Snippet;
  } = $props();
</script>

<svelte:element
  this={onClick ? "button" : "div"}
  class="wmb-stat tone-{tone}"
  class:interactive={!!onClick}
  onclick={onClick}
  type={onClick ? "button" : undefined}
  role={onClick ? undefined : "group"}
>
  <span class="wmb-stat-text">
    <span class="l">{label}</span>
    <span class="v">{value}{#if unit}<small>{unit}</small>{/if}</span>
    {#if sub}<span class="s">{sub}</span>{/if}
  </span>
  {#if trailing}<span class="wmb-stat-trailing">{@render trailing()}</span>{/if}
</svelte:element>

<style>
  .wmb-stat {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
    padding: var(--space-3) var(--space-4);
    border: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font: inherit;
    text-align: left;
  }
  .wmb-stat.interactive {
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }
  @media (hover: hover) {
    .wmb-stat.interactive:hover {
      background: var(--surface-hi);
    }
  }
  .wmb-stat.interactive:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .wmb-stat-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .l {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .v {
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    font-variant-numeric: tabular-nums;
    line-height: 1.15;
    white-space: nowrap;
  }
  .v small {
    margin-left: 3px;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-dim);
  }
  .s {
    font-size: var(--text-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tone-good .v { color: var(--success); }
  .tone-warn .v { color: var(--warning); }
  .tone-bad .v { color: var(--danger); }
  .tone-accent .v { color: var(--accent-hi); }
  .wmb-stat-trailing {
    flex-shrink: 0;
    display: flex;
    align-items: center;
  }
</style>
