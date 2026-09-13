<script lang="ts">
  import { tabsStore } from "$lib/study-notes/tabs-store.svelte";
import { t } from "$lib/i18n";
  import {
    notesPagesList,
    notesPagesEnsure,
    type PageSummary,
    type TabSummary,
    type TabViewKind,
  } from "$lib/notes-bridge";
  import NbNewSplitMenu from "./NbNewSplitMenu.svelte";
  import NbTabContextMenu from "./NbTabContextMenu.svelte";

  type Props = { wndId: number };
  let { wndId }: Props = $props();

  const tabs = $derived(tabsStore.sortedTabsForWnd(wndId));
  const activeTabId = $derived(tabsStore.activeTabIdByWnd[wndId] ?? null);
  const isActiveWnd = $derived(tabsStore.activeWndId === wndId);

  let pickerOpen = $state(false);
  let pickerQuery = $state("");
  let pickerPages = $state<PageSummary[]>([]);
  let pickerLoading = $state(false);
  let pickerInputEl = $state<HTMLInputElement | null>(null);

  let context = $state<{ tab: TabSummary; x: number; y: number } | null>(null);

  let dragTabId = $state<number | null>(null);
  let dragOverTabId = $state<number | null>(null);

  async function openPicker() {
    pickerOpen = true;
    pickerQuery = "";
    pickerLoading = true;
    try {
      pickerPages = await notesPagesList();
    } finally {
      pickerLoading = false;
    }
    queueMicrotask(() => pickerInputEl?.focus());
  }

  function closePicker() {
    pickerOpen = false;
    pickerQuery = "";
  }

  const filteredPages = $derived.by(() => {
    const q = pickerQuery.trim().toLowerCase();
    if (!q) return pickerPages.slice(0, 30);
    return pickerPages
      .filter(
        (p) =>
          p.name.toLowerCase().includes(q) ||
          (p.title && p.title.toLowerCase().includes(q)),
      )
      .slice(0, 30);
  });

  async function pickPage(page: PageSummary) {
    closePicker();
    await tabsStore.openTab({
      wndId,
      pageId: page.id,
      viewKind: "editor",
      activate: true,
    });
  }

  async function createNewPage() {
    const name = pickerQuery.trim();
    if (!name) return;
    closePicker();
    const res = await notesPagesEnsure({ name });
    await tabsStore.openTab({
      wndId,
      pageId: res.id,
      viewKind: "editor",
      activate: true,
    });
  }

  function onPickerKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      closePicker();
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (filteredPages.length > 0) {
        void pickPage(filteredPages[0]);
      } else if (pickerQuery.trim()) {
        void createNewPage();
      }
    }
  }

  function activate(tab: TabSummary) {
    tabsStore.setActiveWnd(wndId);
    tabsStore.setActiveTab(wndId, tab.id);
  }

  async function closeTab(e: MouseEvent, tabId: number) {
    e.stopPropagation();
    await tabsStore.closeTab(tabId);
  }

  function showContext(e: MouseEvent, tab: TabSummary) {
    e.preventDefault();
    e.stopPropagation();
    context = { tab, x: e.clientX, y: e.clientY };
  }

  function viewKindIconPath(kind: TabViewKind): string {
    switch (kind) {
      case "graph":
        return "M9 5a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z M19 19a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z M5 19a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z M9 5l8 11 M5 16l4 -11";
      case "search":
        return "M11 4a7 7 0 1 1 0 14 7 7 0 0 1 0-14Z M16 16l5 5";
      case "journal":
        return "M3 4h12a3 3 0 0 1 3 3v14H6a3 3 0 0 1-3-3Z M9 4v18 M14 9h2 M14 13h2";
      case "templates":
        return "M3 3h7v7H3z M14 3h7v4H14z M14 11h7v10H14z M3 14h7v7H3z";
      case "settings":
        return "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.5-2.4.9a7 7 0 0 0-2-1.2l-.4-2.5h-4l-.4 2.5a7 7 0 0 0-2 1.2l-2.4-.9-2 3.5 2 1.5A7 7 0 0 0 5 12c0 .4 0 .8.1 1.2l-2 1.5 2 3.5 2.4-.9c.6.5 1.3.9 2 1.2l.4 2.5h4l.4-2.5a7 7 0 0 0 2-1.2l2.4.9 2-3.5-2-1.5c.1-.4.1-.8.1-1.2Z";
      default:
        return "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z M14 2v6h6 M16 13H8 M16 17H8 M10 9H8";
    }
  }

  function tabLabel(tab: TabSummary): string {
    if (tab.view_kind === "editor") {
      return tab.page_title || tab.page_name || "Sem nome";
    }
    switch (tab.view_kind) {
      case "graph": return "Graph";
      case "search": return "Buscar";
      case "journal": return "Journal";
      case "templates": return "Templates";
      case "settings": return "Settings";
    }
  }

  function onDragStart(e: DragEvent, tabId: number) {
    if (!e.dataTransfer) return;
    dragTabId = tabId;
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", String(tabId));
  }

  function onDragOver(e: DragEvent, overId: number) {
    if (dragTabId == null) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    dragOverTabId = overId;
  }

  function onDragLeave() {
    dragOverTabId = null;
  }

  async function onDrop(e: DragEvent, dropOnId: number) {
    e.preventDefault();
    const draggedId = dragTabId;
    dragTabId = null;
    dragOverTabId = null;
    if (draggedId == null || draggedId === dropOnId) return;
    const ordered = tabs.map((t) => t.id);
    const fromIdx = ordered.indexOf(draggedId);
    const toIdx = ordered.indexOf(dropOnId);
    if (fromIdx === -1 || toIdx === -1) return;
    ordered.splice(fromIdx, 1);
    ordered.splice(toIdx, 0, draggedId);
    await tabsStore.reorderTabs(wndId, ordered);
  }

  function onDragEnd() {
    dragTabId = null;
    dragOverTabId = null;
  }
</script>

<div
  class="nb-tab-strip"
  class:active-wnd={isActiveWnd}
  role="tablist"
  data-wnd-id={wndId}
>
  {#each tabs as tab (tab.id)}
    {@const isActive = tab.id === activeTabId}
    <div
      class="tab"
      class:active={isActive}
      class:pinned={tab.pinned}
      class:drag-over={dragOverTabId === tab.id}
      role="tab"
      tabindex={isActive ? 0 : -1}
      aria-selected={isActive}
      draggable={!tab.pinned}
      onclick={() => activate(tab)}
      onkeydown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          activate(tab);
        }
      }}
      oncontextmenu={(e) => showContext(e, tab)}
      ondragstart={(e) => onDragStart(e, tab.id)}
      ondragover={(e) => onDragOver(e, tab.id)}
      ondragleave={onDragLeave}
      ondrop={(e) => onDrop(e, tab.id)}
      ondragend={onDragEnd}
      title={tab.page_name ?? tab.view_kind}
    >
      <svg
        viewBox="0 0 24 24"
        width="13"
        height="13"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d={viewKindIconPath(tab.view_kind)} />
      </svg>
      <span class="label">{tabLabel(tab)}</span>
      {#if tab.pinned}
        <span class="pin-dot" aria-label="Tab fixada">●</span>
      {:else}
        <button
          type="button"
          class="close"
          aria-label="Fechar tab"
          onclick={(e) => closeTab(e, tab.id)}
        >
          ×
        </button>
      {/if}
    </div>
  {/each}

  <button
    class="add"
    type="button"
    onclick={openPicker}
    aria-label="Nova tab"
    title="Nova tab (Ctrl+T)"
  >
    +
  </button>

  <NbNewSplitMenu {wndId} />

  <div class="filler"></div>
</div>

{#if pickerOpen}
  <div
    class="picker-backdrop"
    onmousedown={(e) => {
      if (e.target === e.currentTarget) closePicker();
    }}
    role="presentation"
  >
    <div class="picker" role="dialog" aria-label={$t("study.notes.nb.open_new_tab")}>
      <input
        bind:this={pickerInputEl}
        bind:value={pickerQuery}
        onkeydown={onPickerKey}
        type="text"
        placeholder={$t("study.notes.nb.search_or_create")}
        class="picker-input"
      />
      <div class="picker-list">
        {#if pickerLoading}
          <div class="picker-empty">Carregando…</div>
        {:else if filteredPages.length === 0}
          <button
            class="picker-item create"
            type="button"
            onclick={createNewPage}
            disabled={!pickerQuery.trim()}
          >
            <span class="prefix">+ Criar:</span>
            <span class="value">{pickerQuery || "(digite um nome)"}</span>
          </button>
        {:else}
          {#each filteredPages as p (p.id)}
            <button
              class="picker-item"
              type="button"
              onclick={() => pickPage(p)}
            >
              <span class="value">{p.title || p.name}</span>
              <span class="hint">{p.name}</span>
            </button>
          {/each}
          {#if pickerQuery.trim() && !filteredPages.some((p) => p.name === pickerQuery.trim() || p.title === pickerQuery.trim())}
            <button
              class="picker-item create"
              type="button"
              onclick={createNewPage}
            >
              <span class="prefix">+ Criar:</span>
              <span class="value">{pickerQuery}</span>
            </button>
          {/if}
        {/if}
      </div>
    </div>
  </div>
{/if}

{#if context}
  <NbTabContextMenu
    tab={context.tab}
    x={context.x}
    y={context.y}
    onClose={() => (context = null)}
  />
{/if}

<style>
  .nb-tab-strip {
    height: 100%;
    display: flex;
    align-items: stretch;
    gap: 0;
    padding: 0 4px 0 8px;
    overflow-x: auto;
    overflow-y: hidden;
    user-select: none;
    background: color-mix(in oklab, var(--primary, var(--page-bg)) 70%, transparent);
    scrollbar-width: thin;
  }
  .nb-tab-strip.active-wnd {
    background: color-mix(in oklab, var(--accent) 4%, var(--primary, var(--page-bg)));
  }
  .tab {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 6px 0 10px;
    height: 100%;
    border-right: none;
    font-size: 12px;
    color: var(--secondary, var(--text));
    cursor: pointer;
    max-width: 220px;
    min-width: 80px;
    transition: background 100ms ease, color 100ms ease;
  }
  .tab:hover {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .tab.active {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
    color: var(--text);
    box-shadow: inset 0 -2px 0 0 var(--accent);
  }
  .tab.pinned {
    background: color-mix(in oklab, var(--accent) 4%, transparent);
  }
  .tab.drag-over {
    background: color-mix(in oklab, var(--accent) 18%, transparent);
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .close,
  .add {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--tertiary, var(--muted-fg));
    cursor: pointer;
    transition: color 120ms ease, background 120ms ease;
  }
  .close {
    width: 16px;
    height: 16px;
    border-radius: 3px;
    font-size: 14px;
    line-height: 1;
    margin-left: 4px;
  }
  .close:hover {
    background: color-mix(in oklab, var(--accent) 20%, transparent);
    color: var(--text);
  }
  .add {
    width: 28px;
    height: 100%;
    font-size: 16px;
  }
  .add:hover {
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .pin-dot {
    color: var(--accent);
    font-size: 10px;
    margin-left: 4px;
  }
  .filler {
    flex: 1;
  }
  .picker-backdrop {
    position: fixed;
    inset: 0;
    z-index: 150;
    background: rgba(0, 0, 0, 0.45);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 14vh;
  }
  .picker {
    width: min(520px, 92vw);
    max-height: 60vh;
    background: var(--secondary-bg, var(--page-bg));
    border: none;
    border-radius: 8px;
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.4);
    display: flex;
    flex-direction: column;
  }
  .picker-input {
    background: transparent;
    border: 0;
    border-bottom: none;
    padding: 12px 16px;
    color: var(--text);
    font-size: 14px;
    outline: none;
  }
  .picker-list {
    overflow-y: auto;
    padding: 4px;
  }
  .picker-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: transparent;
    border: 0;
    color: var(--text);
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    border-radius: 4px;
  }
  .picker-item:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
  }
  .picker-item .hint {
    margin-left: auto;
    color: var(--tertiary, var(--muted-fg));
    font-size: 11px;
    font-family: ui-monospace, monospace;
  }
  .picker-item.create .prefix {
    color: var(--accent);
    font-weight: 600;
  }
  .picker-item:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }
  .picker-empty {
    padding: 12px 16px;
    color: var(--tertiary, var(--muted-fg));
    font-size: 12px;
  }
</style>
