<script lang="ts" module>
  export type { SelectItem } from "$lib/washmybelly/ui";

  const HEADER_H = 36;
  const ROW_H = 38;
  const ROW_DETAIL_H = 50;

  function looksLikePath(s: string): boolean {
    return /^(\/|~[\\/]|[A-Za-z]:\\|\\\\)/.test(s);
  }

  function indeterminate(node: HTMLInputElement, on: boolean) {
    node.indeterminate = on;
    return {
      update(v: boolean) {
        node.indeterminate = v;
      },
    };
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import { formatBytes, shortenPath } from "$lib/washmybelly/format";
  import {
    flattenRows,
    groupItems,
    rowOffsets,
    selectionState,
    setMany,
    sumSelected,
    toggleOne,
    visibleRange,
    type Row,
    type SelectItem as Item,
  } from "$lib/washmybelly/ui";
  import RiskBadge from "./RiskBadge.svelte";
  import Icon from "./Icon.svelte";

  let {
    items,
    selected = $bindable(new Set<string>()),
    showRisk = true,
    groupLabel = (key: string) => key,
    showToolbar = true,
    collapsible = true,
    virtualizeAt = 200,
    maxHeight = 480,
    emptyText = "",
    trailing,
  }: {
    items: Item[];
    selected?: Set<string>;
    showRisk?: boolean;
    groupLabel?: (key: string) => string;
    showToolbar?: boolean;
    collapsible?: boolean;
    virtualizeAt?: number;
    maxHeight?: number;
    emptyText?: string;
    trailing?: Snippet<[Item]>;
  } = $props();

  let collapsed = $state(new Set<string>());
  let scrollTop = $state(0);
  let viewport = $state(480);

  let groups = $derived(groupItems(items));
  let rows = $derived(flattenRows(groups, collapsed));
  let virtual = $derived(rows.length > virtualizeAt);
  let heights = $derived(rows.map(heightOf));
  let offsets = $derived(rowOffsets(heights));
  let range = $derived(
    virtual ? visibleRange(offsets, scrollTop, viewport, 10) : { start: 0, end: rows.length, padTop: 0, padBottom: 0 },
  );
  let windowRows = $derived(rows.slice(range.start, range.end));
  let totals = $derived(sumSelected(items, selected));
  let allState = $derived(selectionState(items, selected));
  let selectable = $derived(items.some((i) => !i.disabled));

  function heightOf(r: Row): number {
    if (r.kind === "header") return HEADER_H;
    return r.item.detail || r.item.hint ? ROW_DETAIL_H : ROW_H;
  }

  function toggleCollapse(key: string) {
    const next = new Set(collapsed);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    collapsed = next;
  }

  function detailText(it: Item): string {
    const d = it.hint ?? it.detail ?? "";
    if (!d) return "";
    return it.detailIsPath || looksLikePath(d) ? shortenPath(d, 72) : d;
  }
  let hasSizes = $derived(items.some((it) => it.bytes !== undefined));
</script>

<div class="wmb-sl-wrap">
  {#if showToolbar && items.length > 0}
    <div class="wmb-sl-bar">
      <label class="wmb-sl-all" class:disabled={!selectable}>
        <input
          type="checkbox"
          checked={allState === "all"}
          disabled={!selectable}
          use:indeterminate={allState === "some"}
          onchange={() => (selected = setMany(selected, items, allState !== "all"))}
        />
        <span>{allState === "all" ? $t("washmybelly.ui.select_none") : $t("washmybelly.ui.select_all")}</span>
      </label>
      <span class="wmb-sl-sum" aria-live="polite">
        {#if totals.count > 0}
          {#if hasSizes}
            {$t("washmybelly.ui.selected_summary", { count: totals.count, size: formatBytes(totals.bytes) })}
          {:else}
            {$t("washmybelly.ui.selected_only", { count: totals.count })}
          {/if}
        {:else}
          {$t("washmybelly.ui.nothing_selected")}
        {/if}
      </span>
    </div>
  {/if}

  {#if items.length === 0}
    {#if emptyText}<div class="group wmb-sl-empty">{emptyText}</div>{/if}
  {:else}
    <div
      class="group wmb-sl"
      class:virtual
      style:max-height={virtual ? `${maxHeight}px` : undefined}
      onscroll={virtual ? (e) => (scrollTop = (e.currentTarget as HTMLElement).scrollTop) : undefined}
      bind:clientHeight={viewport}
      role="list"
    >
      {#if range.padTop > 0}<div style:height="{range.padTop}px" aria-hidden="true"></div>{/if}
      {#each windowRows as row (row.key)}
        {#if row.kind === "header"}
          {@const st = selectionState(row.group.items, selected)}
          <div class="wmb-sl-head" style:height="{HEADER_H}px" role="listitem">
            <input
              type="checkbox"
              checked={st === "all"}
              disabled={row.group.items.every((i) => i.disabled)}
              use:indeterminate={st === "some"}
              aria-label={$t("washmybelly.ui.select_group", { group: groupLabel(row.group.key) }) as string}
              onchange={() => (selected = setMany(selected, row.group.items, st !== "all"))}
            />
            {#if collapsible}
              <button class="wmb-sl-head-btn" aria-expanded={!row.collapsed} onclick={() => toggleCollapse(row.group.key)}>
                <span class="chev" class:open={!row.collapsed} aria-hidden="true"><Icon name="chevron" size={12} /></span>
                <span class="name">{row.group.key ? groupLabel(row.group.key) : $t("washmybelly.ui.other_group")}</span>
                <span class="count">{row.group.count}</span>
              </button>
            {:else}
              <span class="name">{row.group.key ? groupLabel(row.group.key) : $t("washmybelly.ui.other_group")}</span>
              <span class="count">{row.group.count}</span>
            {/if}
            {#if hasSizes}<span class="size">{formatBytes(row.group.bytes)}</span>{/if}
          </div>
        {:else}
          {@const it = row.item}
          {@const d = detailText(it)}
          <label class="wmb-sl-row" class:disabled={it.disabled} class:checked={selected.has(it.id)} style:height="{heightOf(row)}px" role="listitem">
            <input type="checkbox" checked={selected.has(it.id)} disabled={it.disabled} onchange={() => (selected = toggleOne(selected, it.id))} />
            <span class="text">
              <span class="label" title={it.label}>{it.label}</span>
              {#if d}<span class="detail" title={it.detail ?? it.hint ?? ""}>{d}</span>{/if}
            </span>
            {#if trailing}{@render trailing(it)}{/if}
            {#if showRisk && it.risk}<RiskBadge risk={it.risk} />{/if}
            {#if it.bytes !== undefined}<span class="size">{formatBytes(it.bytes)}</span>{/if}
          </label>
        {/if}
      {/each}
      {#if range.padBottom > 0}<div style:height="{range.padBottom}px" aria-hidden="true"></div>{/if}
    </div>
  {/if}
</div>

<style>
  .wmb-sl-wrap {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .wmb-sl-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: 0 var(--space-3);
    min-height: 24px;
  }
  .wmb-sl-all {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-muted);
    cursor: pointer;
  }
  .wmb-sl-all.disabled {
    opacity: 0.5;
    cursor: default;
  }
  .wmb-sl-sum {
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .wmb-sl {
    overflow: hidden;
  }
  .wmb-sl.virtual {
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .wmb-sl-empty {
    padding: var(--space-5);
    text-align: center;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .wmb-sl-head,
  .wmb-sl-row {
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-3);
    position: relative;
  }
  .wmb-sl-head {
    background: var(--fill-1);
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-muted);
  }
  .wmb-sl-head + .wmb-sl-head,
  .wmb-sl-row + .wmb-sl-head {
    box-shadow: inset 0 var(--hairline) 0 var(--separator);
  }
  .wmb-sl-head-btn {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 100%;
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .wmb-sl-head-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
    border-radius: var(--radius-xs);
  }
  .wmb-sl-head > .name {
    flex: 1;
  }
  .chev {
    display: flex;
    color: var(--text-dim);
    transition: transform var(--duration-fast) var(--ease-out);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }
  .wmb-sl-row {
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }
  .wmb-sl-row + .wmb-sl-row::before {
    content: "";
    position: absolute;
    top: 0;
    left: calc(var(--space-3) + 14px + var(--space-3));
    right: 0;
    height: var(--hairline);
    background: var(--separator);
    pointer-events: none;
  }
  @media (hover: hover) {
    .wmb-sl-row:not(.disabled):hover {
      background: var(--fill-1);
    }
  }
  .wmb-sl-row.disabled {
    cursor: default;
  }
  .wmb-sl-row.disabled .text {
    opacity: 0.5;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .label {
    font-size: var(--text-base);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .detail {
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size {
    flex-shrink: 0;
    min-width: 64px;
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    text-align: right;
  }
  .wmb-sl-head .size {
    color: var(--text-muted);
  }
  input[type="checkbox"]:indeterminate {
    background: var(--accent);
    box-shadow: none;
  }
  input[type="checkbox"]:indeterminate::before {
    content: "";
    width: 8px;
    height: 2px;
    border-radius: 1px;
    background: var(--on-accent);
  }
</style>
