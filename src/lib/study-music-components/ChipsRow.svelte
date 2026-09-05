<script lang="ts">
  type Chip = {
    id: string;
    label: string;
  };
  type Props = {
    chips: Chip[];
    activeId?: string | null;
    onSelect?: (id: string) => void;
    ariaLabel?: string;
  };
  let { chips, activeId = null, onSelect, ariaLabel }: Props = $props();
</script>

<div class="chips-row" role="tablist" aria-label={ariaLabel ?? undefined}>
  {#each chips as chip (chip.id)}
    <button
      type="button"
      role="tab"
      aria-selected={activeId === chip.id}
      class="chip"
      class:active={activeId === chip.id}
      onclick={() => onSelect?.(chip.id)}
    >
      {chip.label}
    </button>
  {/each}
</div>

<style>
  .chips-row {
    display: flex;
    gap: 8px;
    overflow-x: auto;
    padding: 4px 2px;
    scrollbar-width: none;
  }
  .chips-row::-webkit-scrollbar { display: none; }
  .chip {
    flex: 0 0 auto;
    padding: 8px 16px;
    background: var(--fill-1);
    border: none;
    border-radius: 999px;
    color: var(--text-muted);
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: background 160ms ease, color 160ms ease, border-color 160ms ease;
  }
  .chip:hover {
    background: var(--fill-2);
    color: var(--text);
  }
  .chip.active {
    background: var(--accent);
    color: var(--on-accent);
    border-color: transparent;
  }
  @media (prefers-reduced-motion: reduce) {
    .chip { transition: none; }
  }
</style>
