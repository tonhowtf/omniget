<script lang="ts">
  import { t } from "$lib/i18n";
  import { formatBytes } from "$lib/washmybelly/format";
  import { categoryKey, type CategorySlice } from "$lib/washmybelly/clean";

  /// What a junk scan found, per category: one proportional bar (the way
  /// "Storage" in System Settings draws a disk) and a tile per category
  /// with its size and how much of it is ticked. Clicking a tile jumps to
  /// that category in the list. Ported from "Partition Bar" by 8starlabs
  /// (21st.dev): segments sized by share, label and value under each; here
  /// the labels move to a legend grid because a scan has many categories.
  let {
    slices,
    selected,
    onPick,
  }: {
    slices: CategorySlice[];
    /// Selected bytes per category.
    selected: Map<string, number>;
    onPick?: (id: string) => void;
  } = $props();

  // A sliver still shows, so a tiny category is visible in the bar.
  const MIN = 0.012;

  function label(id: string): string {
    const k = categoryKey(id);
    const s = $t(k) as string;
    return s === k ? id.replace(/_/g, " ") : s;
  }
</script>

<div class="wmb-js">
  <div class="bar" role="img" aria-label={slices.map((s) => `${label(s.id)} ${formatBytes(s.bytes)}`).join(", ")}>
    {#each slices as s (s.id)}
      {#if s.bytes > 0}
        <span class="seg" style:flex-grow={Math.max(s.fraction, MIN)} style:background={s.color} title="{label(s.id)} · {formatBytes(s.bytes)}"></span>
      {/if}
    {/each}
  </div>

  <ul class="tiles">
    {#each slices as s (s.id)}
      {@const sel = selected.get(s.id) ?? 0}
      <li>
        <button type="button" class="tile" onclick={() => onPick?.(s.id)}>
          <span class="dot" style:background={s.color} aria-hidden="true"></span>
          <span class="name">{label(s.id)}</span>
          <span class="size">{formatBytes(s.bytes)}</span>
          <span class="sub">
            {#if sel > 0}
              {$t("washmybelly.clean.junk.tile_selected", { size: formatBytes(sel) })}
            {:else}
              {$t("washmybelly.clean.junk.tile_items", { count: s.count })}
            {/if}
          </span>
        </button>
      </li>
    {/each}
  </ul>
</div>

<style>
  .wmb-js {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .bar {
    display: flex;
    gap: 2px;
    height: 12px;
    border-radius: 999px;
    overflow: hidden;
    background: var(--fill-2);
  }
  .seg {
    flex-basis: 0;
    min-width: 3px;
    transition: flex-grow var(--duration-fast, 0.15s) var(--ease-out, ease-out);
  }
  .seg:first-child {
    border-radius: 999px 0 0 999px;
  }
  .seg:last-child {
    border-radius: 0 999px 999px 0;
  }
  .seg:only-child {
    border-radius: 999px;
  }
  .tiles {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--space-2);
  }
  .tile {
    all: unset;
    box-sizing: border-box;
    width: 100%;
    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-areas: "dot name" ". size" ". sub";
    column-gap: var(--space-2);
    row-gap: 1px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border);
    cursor: pointer;
  }
  @media (hover: hover) {
    .tile:hover {
      background: var(--surface-hi, var(--surface));
    }
  }
  .tile:focus-visible {
    outline: var(--focus-ring, 2px solid var(--accent));
    outline-offset: 1px;
  }
  .dot {
    grid-area: dot;
    align-self: center;
    width: 8px;
    height: 8px;
    border-radius: 999px;
  }
  .name {
    grid-area: name;
    font-size: var(--text-sm);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .size {
    grid-area: size;
    font-size: var(--text-lg, 15px);
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .sub {
    grid-area: sub;
    font-size: var(--text-xs, 11px);
    color: var(--text-dim);
  }
  @media (prefers-reduced-motion: reduce) {
    .seg {
      transition: none;
    }
  }
</style>
