<script lang="ts">
  import { onMount } from "svelte";
  import CreatePageDialog from "../CreatePageDialog.svelte";
  import NbNotebookCreateDialog from "./NbNotebookCreateDialog.svelte";
  import NbNotebookCoverDialog from "./NbNotebookCoverDialog.svelte";
  import { tabsStore } from "$lib/study-notes/tabs-store.svelte";
  import { notebooksStore } from "$lib/study-notes/notebooks-store.svelte";
  import {
    notesPagesList,
    notesPagesCreate,
    notesJournalToday,
    type PageSummary,
  } from "$lib/notes-bridge";

  let pages = $state<PageSummary[]>([]);
  let search = $state("");
  let createPageOpen = $state(false);
  let createPageInNotebookId = $state<number | null>(null);
  let createNotebookOpen = $state(false);
  let coverNotebookId = $state<number | null>(null);
  let coverDialogOpen = $state(false);
  let expandedClosed = $state(false);
  let expandedNotebooks = $state<Record<number, boolean>>({});
  let menuFor = $state<number | null>(null);
  let menuPos = $state({ x: 0, y: 0 });
  let renameTarget = $state<number | null>(null);
  let renameDraft = $state("");

  const selectedId = $derived(tabsStore.activeTab?.page_id ?? null);
  const query = $derived(search.trim().toLowerCase());

  function matchesSearch(p: PageSummary): boolean {
    if (!query) return true;
    return (
      p.name.toLowerCase().includes(query) ||
      (p.title ?? "").toLowerCase().includes(query)
    );
  }

  function pagesIn(notebookId: number): PageSummary[] {
    return pages
      .filter((p) => p.notebook_id === notebookId && matchesSearch(p))
      .sort((a, b) =>
        (a.title ?? a.name).localeCompare(b.title ?? b.name, "pt-BR", {
          sensitivity: "base",
        }),
      );
  }

  async function reloadPages() {
    try {
      pages = await notesPagesList();
    } catch {
      pages = [];
    }
  }

  async function reloadAll() {
    await Promise.all([reloadPages(), notebooksStore.refresh()]);
  }

  async function openPage(pageId: number) {
    const wnd = tabsStore.activeWndId ?? tabsStore.leafIds[0];
    if (wnd == null) return;
    await tabsStore.openTab({
      wndId: wnd,
      pageId,
      viewKind: "editor",
      activate: true,
    });
  }

  function startCreatePage(notebookId: number) {
    createPageInNotebookId = notebookId;
    createPageOpen = true;
  }

  async function confirmCreatePage(name: string) {
    const nbId = createPageInNotebookId ?? notebooksStore.activeId;
    try {
      const r = await notesPagesCreate({ name, notebookId: nbId });
      createPageOpen = false;
      createPageInNotebookId = null;
      await reloadAll();
      await openPage(r.id);
    } catch {
      /* ignore */
    }
  }

  async function openJournalToday() {
    try {
      const r = await notesJournalToday();
      await reloadAll();
      await openPage(r.page_id);
    } catch {
      /* ignore */
    }
  }

  async function confirmCreateNotebook(args: {
    name: string;
    color: string | null;
    iconLucide: string | null;
  }) {
    const id = await notebooksStore.create(args);
    createNotebookOpen = false;
    if (id != null) {
      expandedNotebooks = { ...expandedNotebooks, [id]: true };
      await notebooksStore.setActive(id);
    }
  }

  function toggleNotebook(id: number) {
    expandedNotebooks = {
      ...expandedNotebooks,
      [id]: !(expandedNotebooks[id] ?? true),
    };
  }

  function isExpanded(id: number): boolean {
    return expandedNotebooks[id] ?? true;
  }

  function openContext(e: MouseEvent, notebookId: number) {
    e.preventDefault();
    e.stopPropagation();
    menuFor = notebookId;
    menuPos = { x: e.clientX, y: e.clientY };
  }

  function closeContext() {
    menuFor = null;
  }

  function startRename(notebookId: number) {
    const nb = notebooksStore.byId(notebookId);
    if (!nb) return;
    renameTarget = notebookId;
    renameDraft = nb.name;
    menuFor = null;
  }

  async function commitRename() {
    if (renameTarget == null) return;
    const trimmed = renameDraft.trim();
    if (trimmed.length > 0) {
      await notebooksStore.rename(renameTarget, trimmed);
    }
    renameTarget = null;
    renameDraft = "";
  }

  async function doClose(notebookId: number) {
    await notebooksStore.close(notebookId);
    closeContext();
  }

  async function doReopen(notebookId: number) {
    await notebooksStore.reopen(notebookId);
    closeContext();
  }

  async function doDelete(notebookId: number) {
    closeContext();
    const nb = notebooksStore.byId(notebookId);
    if (!nb) return;
    if (nb.id === 1) {
      window.alert("Notebook 'Pessoal' não pode ser excluído.");
      return;
    }
    if (nb.page_count > 0) {
      const ok = window.confirm(
        `Excluir "${nb.name}" remove ${nb.page_count} página${nb.page_count === 1 ? "" : "s"} para sempre. Continuar?`,
      );
      if (!ok) return;
      const r = await notebooksStore.delete(notebookId, true);
      if (!r.deleted) {
        window.alert("Falha ao excluir notebook.");
      } else {
        await reloadPages();
      }
    } else {
      await notebooksStore.delete(notebookId, false);
    }
  }

  function openCover(notebookId: number) {
    coverNotebookId = notebookId;
    coverDialogOpen = true;
    closeContext();
  }

  async function pickColor(notebookId: number) {
    closeContext();
    const swatch = window.prompt(
      "Cor (hex ou var, ex: #f97316). Vazio = sem cor.",
      notebooksStore.byId(notebookId)?.color ?? "",
    );
    if (swatch == null) return;
    await notebooksStore.setColor(
      notebookId,
      swatch.trim().length > 0 ? swatch.trim() : null,
    );
  }

  async function pickIcon(notebookId: number) {
    closeContext();
    const icon = window.prompt(
      "Ícone (lucide name, ex: book, briefcase). Vazio = sem ícone.",
      notebooksStore.byId(notebookId)?.icon_lucide ?? "",
    );
    if (icon == null) return;
    await notebooksStore.setIcon(
      notebookId,
      icon.trim().length > 0 ? icon.trim() : null,
    );
  }

  async function activate(notebookId: number) {
    await notebooksStore.setActive(notebookId);
  }

  function fmtDay(secs: number): string {
    if (!secs) return "—";
    const d = new Date(secs * 1000);
    const today = new Date();
    const same =
      d.getFullYear() === today.getFullYear() &&
      d.getMonth() === today.getMonth() &&
      d.getDate() === today.getDate();
    if (same) return "hoje";
    return d.toLocaleDateString();
  }

  onMount(() => {
    if (!notebooksStore.hydrated) void notebooksStore.hydrate();
    void reloadPages();
    const handler = () => void reloadAll();
    const newNotebookHandler = () => (createNotebookOpen = true);
    window.addEventListener("study:notes:dirty", handler);
    window.addEventListener("study:notes:notebook-new", newNotebookHandler);
    const close = () => closeContext();
    window.addEventListener("mousedown", close);
    return () => {
      window.removeEventListener("study:notes:dirty", handler);
      window.removeEventListener("study:notes:notebook-new", newNotebookHandler);
      window.removeEventListener("mousedown", close);
    };
  });

  const visibleNotebooks = $derived(notebooksStore.visible);
  const closedNotebooks = $derived(notebooksStore.closed);
</script>

<div class="nb-dock-files">
  <header class="dock-head">
    <h2>Notebooks</h2>
    <button
      class="head-btn"
      type="button"
      onclick={() => void openJournalToday()}
      title="Journal de hoje"
      aria-label="Journal de hoje"
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <rect x="3" y="4" width="18" height="18" rx="2" />
        <path d="M3 10h18 M8 2v4 M16 2v4" />
      </svg>
    </button>
  </header>

  <input
    class="search"
    placeholder="Filtrar páginas…"
    bind:value={search}
  />

  <div class="scroll">
    {#each visibleNotebooks as nb (nb.id)}
      {@const expanded = isExpanded(nb.id)}
      {@const items = pagesIn(nb.id)}
      <section class="notebook" class:active={nb.id === notebooksStore.activeId}>
        <button
          class="nb-row"
          type="button"
          onclick={() => toggleNotebook(nb.id)}
          oncontextmenu={(e) => openContext(e, nb.id)}
          ondblclick={() => void activate(nb.id)}
        >
          <span
            class="chevron"
            class:open={expanded}
            aria-hidden="true"
          >▸</span>
          {#if nb.color}
            <span class="dot" style:background={nb.color}></span>
          {:else}
            <span class="dot dim"></span>
          {/if}
          <span class="nb-name" title={nb.icon_lucide ?? ""}>
            {nb.name}
          </span>
          <span class="count">{nb.page_count}</span>
        </button>

        {#if expanded}
          <ul class="page-list">
            {#each items as p (p.id)}
              <li>
                <button
                  type="button"
                  class="page-row"
                  class:active={selectedId === p.id}
                  onclick={() => void openPage(p.id)}
                  title={p.name}
                >
                  <span class="page-name">{p.title ?? p.name}</span>
                  <span class="page-meta">
                    {p.block_count} · {fmtDay(p.updated_at)}
                  </span>
                </button>
              </li>
            {:else}
              <li class="empty">— vazio —</li>
            {/each}
            <li>
              <button
                type="button"
                class="page-row add-page"
                onclick={() => startCreatePage(nb.id)}
                title="Nova página neste notebook"
              >
                <span class="page-name">+ Nova página</span>
              </button>
            </li>
          </ul>
        {/if}
      </section>
    {/each}

    {#if closedNotebooks.length > 0}
      <section class="closed-section">
        <button
          type="button"
          class="closed-toggle"
          onclick={() => (expandedClosed = !expandedClosed)}
        >
          <span class="chevron" class:open={expandedClosed} aria-hidden="true">▸</span>
          <span>Closed notebooks ({closedNotebooks.length})</span>
        </button>
        {#if expandedClosed}
          <ul class="closed-list">
            {#each closedNotebooks as nb (nb.id)}
              <li>
                <button
                  type="button"
                  class="closed-row"
                  oncontextmenu={(e) => openContext(e, nb.id)}
                  onclick={() => void doReopen(nb.id)}
                  title="Reabrir notebook"
                >
                  {#if nb.color}
                    <span class="dot" style:background={nb.color}></span>
                  {:else}
                    <span class="dot dim"></span>
                  {/if}
                  <span>{nb.name}</span>
                  <span class="count">{nb.page_count}</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/if}
  </div>

  <footer class="dock-foot">
    <button
      type="button"
      class="new-nb"
      onclick={() => (createNotebookOpen = true)}
      title="Novo notebook (Ctrl+Shift+N)"
    >
      <span aria-hidden="true">＋</span>
      <span>Novo notebook</span>
    </button>
  </footer>
</div>

{#if menuFor != null}
  {@const nb = notebooksStore.byId(menuFor)}
  {#if nb}
    <div
      class="ctx-menu"
      role="menu"
      tabindex="-1"
      data-modal="true"
      style:left={`${menuPos.x}px`}
      style:top={`${menuPos.y}px`}
      onmousedown={(e) => e.stopPropagation()}
    >
      <button class="ctx-item" onclick={() => void activate(nb.id)}>
        Tornar ativo
      </button>
      <button class="ctx-item" onclick={() => startRename(nb.id)}>
        Renomear
      </button>
      <button class="ctx-item" onclick={() => openCover(nb.id)}>
        Capa…
      </button>
      <button class="ctx-item" onclick={() => void pickColor(nb.id)}>
        Cor…
      </button>
      <button class="ctx-item" onclick={() => void pickIcon(nb.id)}>
        Ícone…
      </button>
      <hr />
      {#if nb.closed}
        <button class="ctx-item" onclick={() => void doReopen(nb.id)}>
          Reabrir
        </button>
      {:else}
        <button class="ctx-item" onclick={() => void doClose(nb.id)}>
          Fechar
        </button>
      {/if}
      <button class="ctx-item danger" onclick={() => void doDelete(nb.id)}>
        Excluir…
      </button>
    </div>
  {/if}
{/if}

{#if renameTarget != null}
  <div
    class="rename-overlay"
    role="presentation"
    data-modal="true"
    onmousedown={(e) => {
      if (e.target === e.currentTarget) {
        renameTarget = null;
      }
    }}
  >
    <div class="rename-card">
      <label>
        <span>Renomear notebook</span>
        <input
          type="text"
          bind:value={renameDraft}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void commitRename();
            } else if (e.key === "Escape") {
              e.preventDefault();
              renameTarget = null;
            }
          }}
        />
      </label>
      <div class="rename-actions">
        <button type="button" class="btn ghost" onclick={() => (renameTarget = null)}>
          Cancelar
        </button>
        <button type="button" class="btn primary" onclick={() => void commitRename()}>
          Salvar
        </button>
      </div>
    </div>
  </div>
{/if}

<CreatePageDialog
  open={createPageOpen}
  onConfirm={(name) => void confirmCreatePage(name)}
  onClose={() => {
    createPageOpen = false;
    createPageInNotebookId = null;
  }}
/>

<NbNotebookCreateDialog
  open={createNotebookOpen}
  onConfirm={(args) => void confirmCreateNotebook(args)}
  onClose={() => (createNotebookOpen = false)}
/>

<NbNotebookCoverDialog
  open={coverDialogOpen}
  notebookId={coverNotebookId}
  onClose={() => {
    coverDialogOpen = false;
    coverNotebookId = null;
  }}
/>

<style>
  .nb-dock-files {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 8px 8px;
    overflow: hidden;
  }
  .dock-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 0 4px;
  }
  .dock-head h2 {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--tertiary);
  }
  .head-btn {
    background: transparent;
    border: 0;
    padding: 4px;
    color: var(--tertiary);
    cursor: pointer;
    border-radius: 4px;
  }
  .head-btn:hover {
    color: var(--text);
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .search {
    padding: 6px 9px;
    border: none;
    border-radius: 6px;
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 12px;
  }
  .search:focus {
    outline: none;
    border-color: var(--accent);
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding-right: 2px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .notebook {
    display: flex;
    flex-direction: column;
  }
  .notebook.active > .nb-row {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .nb-row {
    display: grid;
    grid-template-columns: 14px auto 1fr auto;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    text-align: left;
    cursor: pointer;
    transition: background 120ms;
  }
  .nb-row:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .chevron {
    display: inline-block;
    color: var(--tertiary);
    transition: transform 120ms;
    font-size: 9px;
    line-height: 1;
    text-align: center;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--accent);
    flex-shrink: 0;
  }
  .dot.dim {
    background: color-mix(in oklab, var(--text) 22%, transparent);
  }
  .nb-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    color: var(--tertiary);
    font-size: 10.5px;
    font-weight: 500;
  }
  .page-list {
    list-style: none;
    margin: 0 0 4px 0;
    padding: 0 0 0 18px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .page-row {
    display: flex;
    flex-direction: column;
    gap: 1px;
    width: 100%;
    padding: 4px 6px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background 100ms;
  }
  .page-row:hover {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .page-row.active {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
  }
  .page-name {
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .page-meta {
    font-size: 10px;
    color: var(--tertiary);
  }
  .add-page {
    color: var(--tertiary);
    font-style: italic;
  }
  .add-page:hover {
    color: var(--accent);
  }
  .empty {
    color: var(--tertiary);
    font-size: 11px;
    padding: 4px 6px;
    list-style: none;
  }
  .closed-section {
    margin-top: 8px;
    border-top: none;
    padding-top: 6px;
  }
  .closed-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    background: transparent;
    color: var(--tertiary);
    font: inherit;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    cursor: pointer;
    width: 100%;
    text-align: left;
  }
  .closed-toggle:hover {
    color: var(--text);
  }
  .closed-list {
    list-style: none;
    margin: 4px 0 0 0;
    padding: 0 0 0 18px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .closed-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 6px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
    transition: background 100ms;
  }
  .closed-row:hover {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
    color: var(--text);
  }
  .dock-foot {
    border-top: none;
    padding-top: 6px;
  }
  .new-nb {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px 8px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--tertiary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    transition: background 120ms, color 120ms, border-color 120ms;
  }
  .new-nb:hover {
    color: var(--accent);
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .ctx-menu {
    position: fixed;
    z-index: 220;
    min-width: 180px;
    background: var(--bg);
    border: none;
    border-radius: 8px;
    padding: 4px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.34);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ctx-menu hr {
    border: 0;
    border-top: none;
    margin: 3px 0;
  }
  .ctx-item {
    text-align: left;
    padding: 6px 10px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .ctx-item:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .ctx-item.danger {
    color: #ef4444;
  }
  .rename-overlay {
    position: fixed;
    inset: 0;
    z-index: 220;
    display: grid;
    place-items: center;
    background: color-mix(in oklab, black 35%, transparent);
  }
  .rename-card {
    background: var(--bg);
    border: none;
    border-radius: 10px;
    padding: 14px;
    width: min(360px, 90vw);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .rename-card label {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 11px;
    color: var(--tertiary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .rename-card input {
    padding: 7px 9px;
    border: none;
    border-radius: 6px;
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .rename-card input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .rename-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .btn {
    padding: 6px 12px;
    border-radius: 6px;
    border: 1px solid transparent;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    transition: background 120ms;
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent);
  }
</style>
