<script lang="ts">
  import { tick, untrack } from "svelte";
  import type { SvelteMap } from "svelte/reactivity";
  import { t } from "$lib/i18n";
  import { columnsFor, moveIndex, rowRange, scrollToReveal } from "$lib/icons/grid";
  import { splitId } from "$lib/icons/sets";
  import type { IconData } from "$lib/icons/types";
  import IconView from "./IconView.svelte";
  import Glyph from "./Glyph.svelte";

  let {
    ids,
    bodies,
    favorites,
    selected = null,
    active = $bindable(0),
    label,
    height = 460,
    cell = 76,
    compact = false,
    isPalette,
    onopen,
    onneed,
  }: {
    ids: string[];
    bodies: SvelteMap<string, IconData | null>;
    favorites: Set<string>;
    selected?: string | null;
    active?: number;
    label: string;
    height?: number;
    cell?: number;
    compact?: boolean;
    isPalette: (prefix: string) => boolean;
    onopen: (id: string, index: number) => void;
    onneed: (ids: string[]) => void;
  } = $props();

  const GAP = 6;
  let scroller = $state<HTMLDivElement | null>(null);
  let width = $state(0);
  let scrollTop = $state(0);
  let rowH = $derived(cell + GAP);
  let cols = $derived(columnsFor(width, cell, GAP));
  let cellW = $derived(cols > 0 ? (width - GAP * (cols - 1)) / cols : cell);
  let rowCount = $derived(Math.ceil(ids.length / cols));
  let viewport = $derived(Math.min(height, Math.max(rowCount * rowH - GAP, rowH)));
  let range = $derived(rowRange(scrollTop, viewport, rowH, rowCount, 3));
  let visible = $derived.by(() => {
    const out: number[] = [];
    for (let r = range.first; r <= range.last; r++) {
      for (let c = 0; c < cols; c++) {
        const i = r * cols + c;
        if (i < ids.length) out.push(i);
      }
    }
    return out;
  });
  let iconSize = $derived(compact ? 20 : 24);

  $effect(() => {
    const wanted = visible.map((i) => ids[i]);
    untrack(() => onneed(wanted));
  });

  $effect(() => {
    if (active >= ids.length) active = Math.max(0, ids.length - 1);
  });

  async function focusActive() {
    if (!scroller) return;
    scrollTop = scrollToReveal(active, cols, rowH, scroller.scrollTop, viewport);
    scroller.scrollTop = scrollTop;
    await tick();
    scroller.querySelector<HTMLElement>(`[data-index="${active}"]`)?.focus({ preventScroll: true });
  }

  export function focus() {
    void focusActive();
  }

  function onkey(event: KeyboardEvent) {
    if (event.key === "Enter" || event.key === " ") {
      const id = ids[active];
      if (id) {
        event.preventDefault();
        onopen(id, active);
      }
      return;
    }
    const pageRows = Math.max(1, Math.floor(viewport / rowH));
    const next = moveIndex(active, event.key, cols, ids.length, pageRows);
    if (next === active || next < 0) {
      if (["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "PageUp", "PageDown", "Home", "End"].includes(event.key)) event.preventDefault();
      return;
    }
    event.preventDefault();
    active = next;
    void focusActive();
  }
</script>

<div
  class="ic-grid"
  bind:this={scroller}
  bind:clientWidth={width}
  style:height="{viewport}px"
  onscroll={() => (scrollTop = scroller?.scrollTop ?? 0)}
  onkeydown={onkey}
  role="listbox"
  aria-label={label}
  tabindex="-1"
>
  <div class="ic-grid-inner" style:height="{Math.max(rowCount * rowH - GAP, 0)}px">
    {#each visible as i (ids[i])}
      {@const id = ids[i]}
      {@const parsed = splitId(id)}
      {@const icon = bodies.get(id)}
      <button
        type="button"
        class="ic-cell"
        class:compact
        class:on={selected === id}
        role="option"
        aria-selected={selected === id}
        aria-setsize={ids.length}
        aria-posinset={i + 1}
        aria-label={parsed ? ($t("icons.cell_label", { name: parsed.name, set: parsed.prefix }) as string) : id}
        data-index={i}
        tabindex={i === active ? 0 : -1}
        title={id}
        style:top="{Math.floor(i / cols) * rowH}px"
        style:left="{(i % cols) * (cellW + GAP)}px"
        style:width="{cellW}px"
        style:height="{cell}px"
        onclick={() => {
          active = i;
          onopen(id, i);
        }}
        onfocus={() => (active = i)}
      >
        {#if icon}
          <IconView {icon} size={iconSize} palette={parsed ? isPalette(parsed.prefix) : false} />
        {:else if icon === null}
          <span class="ic-cell-missing"><Glyph name="warning" size={14} /></span>
        {:else}
          <span class="ic-cell-wait" aria-hidden="true"></span>
        {/if}
        {#if favorites.has(id)}
          <span class="ic-cell-fav"><Glyph name="star-fill" size={10} /></span>
        {/if}
        {#if !compact}
          <span class="ic-cell-name">{parsed?.name ?? id}</span>
        {/if}
      </button>
    {/each}
  </div>
</div>

<style>
  .ic-grid {
    position: relative;
    overflow-y: auto;
    overflow-x: hidden;
    overscroll-behavior: contain;
    contain: strict;
    width: 100%;
  }

  .ic-grid-inner {
    position: relative;
    width: 100%;
  }

  .ic-cell {
    position: absolute;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    padding: var(--space-1);
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text);
    font: inherit;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .ic-cell:hover {
      background: var(--fill-1);
    }
  }

  .ic-cell:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .ic-cell.on {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }

  .ic-cell-name {
    position: absolute;
    right: var(--space-1);
    bottom: 3px;
    left: var(--space-1);
    overflow: hidden;
    color: var(--text-dim);
    font-size: var(--text-caption);
    line-height: var(--leading-caption);
    text-align: center;
    text-overflow: ellipsis;
    white-space: nowrap;
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease-out);
  }

  .ic-cell:hover .ic-cell-name,
  .ic-cell:focus-visible .ic-cell-name,
  .ic-cell.on .ic-cell-name {
    opacity: 1;
  }

  .ic-cell-fav {
    position: absolute;
    top: 5px;
    right: 5px;
    color: var(--accent-text);
  }

  .ic-cell-wait {
    width: 22px;
    height: 22px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
  }

  .ic-cell-missing {
    color: var(--text-faint);
  }

  @media (prefers-reduced-motion: reduce) {
    .ic-cell,
    .ic-cell-name {
      transition: none;
    }
  }
</style>
