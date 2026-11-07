<script lang="ts">
  import { t } from "$lib/i18n";
  import { moveItems, removeItems, rotateItems, selectRange, type PageItem } from "$lib/pdf/pages";
  import type { Thumbs } from "$lib/pdf/thumbs";
  import PageThumb from "./PageThumb.svelte";

  let {
    items,
    selected = $bindable(),
    thumbs,
    tones,
    disabled = false,
    onchange,
  }: {
    items: PageItem[];
    selected: Set<string>;
    thumbs: Thumbs;
    tones: Map<string, number>;
    disabled?: boolean;
    onchange: (items: PageItem[]) => void;
  } = $props();

  const DRAG_START = 6;
  const EDGE = 64;

  let grid = $state<HTMLUListElement | null>(null);
  let anchor = $state<string | null>(null);
  let focusKey = $state<string | null>(null);
  let drag = $state<{ key: string; x: number; y: number; active: boolean; pointer: number } | null>(null);
  let drop = $state<{ index: number; key: string; after: boolean } | null>(null);

  let multiSource = $derived(tones.size > 1);

  function label(item: PageItem, index: number): string {
    if (item.kind === "blank") return $t("pdf.page_blank_label", { n: index + 1 }) as string;
    return $t("pdf.page_label", { n: index + 1, page: item.page + 1 }) as string;
  }

  function pick(event: MouseEvent | KeyboardEvent, key: string) {
    if (event.shiftKey && anchor) {
      selected = selectRange(items, anchor, key);
    } else if (event.metaKey || event.ctrlKey) {
      const next = new Set(selected);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      selected = next;
      anchor = key;
    } else {
      selected = new Set([key]);
      anchor = key;
    }
  }

  function act(keys: Set<string>, kind: "left" | "right" | "delete") {
    if (disabled || keys.size === 0) return;
    if (kind === "delete") {
      const index = items.findIndex((i) => keys.has(i.key));
      const next = removeItems(items, keys);
      selected = new Set();
      onchange(next);
      focusKey = next[Math.min(index, next.length - 1)]?.key ?? null;
      return;
    }
    onchange(rotateItems(items, keys, kind === "left" ? -90 : 90));
  }

  function onTileAction(event: MouseEvent, key: string, kind: "left" | "right" | "delete") {
    event.stopPropagation();
    act(selected.has(key) ? selected : new Set([key]), kind);
  }

  function scrollParent(node: HTMLElement | null): HTMLElement | null {
    let el = node?.parentElement ?? null;
    while (el) {
      const style = getComputedStyle(el);
      if (/(auto|scroll)/.test(style.overflowY) && el.scrollHeight > el.clientHeight) return el;
      el = el.parentElement;
    }
    return null;
  }

  function dropTarget(x: number, y: number): { index: number; key: string; after: boolean } | null {
    if (!grid) return null;
    const tiles = [...grid.querySelectorAll<HTMLElement>("[data-key]")];
    let best: { index: number; key: string; after: boolean; dist: number } | null = null;
    tiles.forEach((tile, index) => {
      const r = tile.getBoundingClientRect();
      const cx = Math.max(r.left, Math.min(x, r.right));
      const cy = Math.max(r.top, Math.min(y, r.bottom));
      const dist = Math.hypot(x - cx, y - cy);
      if (!best || dist < best.dist) {
        const after = x > r.left + r.width / 2;
        best = { index: after ? index + 1 : index, key: tile.dataset.key ?? "", after, dist };
      }
    });
    if (!best) return null;
    const { index, key, after } = best;
    return { index, key, after };
  }

  function onPointerDown(event: PointerEvent, key: string) {
    if (disabled || event.button !== 0) return;
    if ((event.target as HTMLElement).closest("button")) return;
    drag = { key, x: event.clientX, y: event.clientY, active: false, pointer: event.pointerId };
  }

  function onPointerMove(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.pointer) return;
    if (!drag.active) {
      if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < DRAG_START) return;
      drag.active = true;
      if (!selected.has(drag.key)) {
        selected = new Set([drag.key]);
        anchor = drag.key;
      }
      grid?.setPointerCapture(event.pointerId);
    }
    drop = dropTarget(event.clientX, event.clientY);
    const scroller = scrollParent(grid);
    if (scroller) {
      const r = scroller.getBoundingClientRect();
      if (event.clientY < r.top + EDGE) scroller.scrollBy({ top: -18 });
      else if (event.clientY > r.bottom - EDGE) scroller.scrollBy({ top: 18 });
    }
  }

  function onPointerUp(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.pointer) return;
    const { active, key } = drag;
    drag = null;
    if (grid?.hasPointerCapture(event.pointerId)) grid.releasePointerCapture(event.pointerId);
    if (active) {
      if (drop) onchange(moveItems(items, selected, drop.index));
      drop = null;
      return;
    }
    pick(event, key);
    focusKey = key;
  }

  function onPointerCancel() {
    drag = null;
    drop = null;
  }

  function onKeydown(event: KeyboardEvent) {
    if (disabled || items.length === 0) return;
    const current = items.findIndex((i) => i.key === focusKey);
    const at = current < 0 ? 0 : current;
    const columns = grid ? Math.max(1, getComputedStyle(grid).gridTemplateColumns.split(" ").length) : 1;
    const step: Record<string, number> = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -columns, ArrowDown: columns };
    const mod = event.metaKey || event.ctrlKey;
    if (event.key in step) {
      event.preventDefault();
      if (event.altKey && selected.size > 0) {
        const first = items.findIndex((i) => selected.has(i.key));
        const last = items.length - 1 - [...items].reverse().findIndex((i) => selected.has(i.key));
        const delta = step[event.key];
        const to = delta < 0 ? Math.max(0, first + delta) : Math.min(items.length, last + 1 + delta);
        onchange(moveItems(items, selected, to));
        return;
      }
      const next = items[Math.max(0, Math.min(items.length - 1, at + step[event.key]))];
      focusKey = next.key;
      if (event.shiftKey) {
        anchor ??= items[at].key;
        selected = selectRange(items, anchor, next.key);
      }
      return;
    }
    if (event.key === " " || event.key === "Enter") {
      event.preventDefault();
      if (items[at]) pick(event, items[at].key);
    } else if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      act(selected, "delete");
    } else if (mod && event.key.toLowerCase() === "a") {
      event.preventDefault();
      selected = new Set(items.map((i) => i.key));
    } else if (event.key === "Escape") {
      selected = new Set();
    } else if (event.key.toLowerCase() === "r" && !mod) {
      event.preventDefault();
      act(selected, event.shiftKey ? "left" : "right");
    }
  }

  $effect(() => {
    if (!grid || !focusKey) return;
    const tile = grid.querySelector<HTMLElement>(`[data-key="${focusKey}"]`);
    if (tile && grid.contains(document.activeElement)) tile.focus({ preventScroll: false });
  });
</script>

<ul
  class="pg"
  class:dragging={drag?.active}
  bind:this={grid}
  role="listbox"
  aria-multiselectable="true"
  aria-label={$t("pdf.pages_label") as string}
  tabindex="-1"
  onkeydown={onKeydown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerCancel}
>
  {#each items as item, i (item.key)}
    {@const isSelected = selected.has(item.key)}
    {@const tone = item.kind === "page" ? tones.get(item.path) : undefined}
    <li
      class="pg-tile"
      class:selected={isSelected}
      class:lifted={drag?.active && isSelected}
      class:drop-before={drop?.key === item.key && !drop.after}
      class:drop-after={drop?.key === item.key && drop.after}
      data-key={item.key}
      role="option"
      aria-selected={isSelected}
      aria-label={label(item, i)}
      tabindex={(focusKey ?? items[0]?.key) === item.key ? 0 : -1}
      onpointerdown={(e) => onPointerDown(e, item.key)}
      onfocus={() => (focusKey = item.key)}
    >
      <PageThumb {item} {thumbs} />
      <div class="pg-foot">
        {#if multiSource && tone !== undefined}
          <span class="pg-tone" style:--tone="var(--pdf-tone-{tone % 6})" aria-hidden="true"></span>
        {/if}
        <span class="pg-num">{i + 1}</span>
        {#if item.kind === "blank"}
          <span class="pg-note">{$t("pdf.blank")}</span>
        {/if}
      </div>
      {#if !disabled}
        <div class="pg-actions">
          <button type="button" class="pg-act" title={$t("pdf.rotate_left") as string} aria-label={$t("pdf.rotate_left") as string} onclick={(e) => onTileAction(e, item.key, "left")}>
            <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3.5 6.5A5 5 0 1 1 4 11" /><path d="M3 3v3.5h3.5" /></svg>
          </button>
          <button type="button" class="pg-act" title={$t("pdf.rotate_right") as string} aria-label={$t("pdf.rotate_right") as string} onclick={(e) => onTileAction(e, item.key, "right")}>
            <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12.5 6.5A5 5 0 1 0 12 11" /><path d="M13 3v3.5H9.5" /></svg>
          </button>
          <button type="button" class="pg-act pg-act-danger" title={$t("pdf.delete_page") as string} aria-label={$t("pdf.delete_page") as string} onclick={(e) => onTileAction(e, item.key, "delete")}>
            <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5" /></svg>
          </button>
        </div>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .pg {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(152px, 1fr));
    gap: var(--space-3);
    margin: 0;
    padding: var(--space-3);
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    touch-action: pan-y;
    user-select: none;
    outline: none;
  }

  .pg.dragging {
    cursor: grabbing;
  }

  .pg-tile {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-2) var(--space-1);
    border-radius: var(--radius-md);
    cursor: grab;
    outline: none;
    transition: background var(--duration-fast) var(--ease-out), opacity var(--duration-fast) var(--ease-out);
  }

  .pg-tile:hover {
    background: var(--hover-bg, color-mix(in srgb, var(--text) 5%, transparent));
  }

  .pg-tile:focus-visible {
    box-shadow: 0 0 0 2px var(--accent);
  }

  .pg-tile.selected {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }

  .pg-tile.lifted {
    opacity: 0.45;
  }

  .pg-tile.drop-before::before,
  .pg-tile.drop-after::after {
    content: "";
    position: absolute;
    top: 6px;
    bottom: 6px;
    width: 3px;
    border-radius: 2px;
    background: var(--accent);
  }

  .pg-tile.drop-before::before {
    left: calc(var(--space-3) / -2 - 1px);
  }

  .pg-tile.drop-after::after {
    right: calc(var(--space-3) / -2 - 1px);
  }

  .pg-foot {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-height: 18px;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .pg-tone {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--tone);
  }

  .pg-num {
    font-weight: 600;
    color: var(--text-muted);
  }

  .pg-actions {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--pane-bg);
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.18);
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease-out);
  }

  .pg-tile:hover .pg-actions,
  .pg-tile:focus-within .pg-actions {
    opacity: 1;
  }

  .pg.dragging .pg-actions {
    opacity: 0;
  }

  .pg-act {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }

  .pg-act:hover {
    background: color-mix(in srgb, var(--text) 8%, transparent);
    color: var(--text);
  }

  .pg-act-danger:hover {
    color: var(--danger, #e5484d);
  }

  @media (prefers-reduced-motion: reduce) {
    .pg-tile,
    .pg-actions {
      transition: none;
    }
  }
</style>
