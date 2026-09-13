<script lang="ts">
  import { onMount } from "svelte";
  import { page as routePage } from "$app/stores";
  import { t } from "$lib/i18n";
  import { awardXp } from "$lib/study-gamification";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";
  import CreatePageDialog from "$lib/study-components/notes/CreatePageDialog.svelte";
  import RenamePageDialog from "$lib/study-components/notes/RenamePageDialog.svelte";
  import Editor from "$lib/study-components/notes/Editor.svelte";
  import Breadcrumb from "$lib/study-components/notes/Breadcrumb.svelte";
  import PageHero from "$lib/study-components/notes/PageHero.svelte";
  import CoverManager from "$lib/study-components/notes/CoverManager.svelte";
  import HistoryModal from "$lib/study-components/notes/HistoryModal.svelte";
  import {
    notesPagesList,
    notesPagesGet,
    notesPagesCreate,
    notesPagesDelete,
    notesPagesRenameCascade,
    notesPagesResolve,
    notesPagesEnsure,
    notesPagesSetAliases,
    notesPagesSetTags,
    notesJournalToday,
    notesBlocksPageTree,
    notesUndoLastOp,
    notesUndoRedoLast,
    notesAssetsListForPage,
    type NotePage as Page,
    type BlockNode,
  } from "$lib/notes-bridge";
  import { notesShell } from "$lib/study-notes/shell-store.svelte";
  import { tabsStore } from "$lib/study-notes/tabs-store.svelte";

  let currentPage = $state<Page | null>(null);
  let blockTree = $state<BlockNode[]>([]);

  let loading = $state(true);
  let error = $state("");
  let notesTableMissing = $state(false);
  let toast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);

  let renameOpen = $state(false);
  let renameValue = $state("");
  let confirmDeletePageOpen = $state(false);

  let aliasesEditor = $state("");
  let tagsEditor = $state("");
  let aliasesDirty = $state(false);
  let tagsDirty = $state(false);

  let createPageOpen = $state(false);

  let coverUrl = $state<string | null>(null);
  let coverManagerOpen = $state(false);
  let historyOpen = $state(false);
  let firstBlockId = $derived(blockTree.length > 0 ? blockTree[0].id : null);

  async function loadCover(pageId: number) {
    try {
      const assets = await notesAssetsListForPage(pageId);
      const cover = assets.find((a) => a.kind === "cover");
      coverUrl = cover?.path ?? null;
    } catch (e) {
      console.error("loadCover failed", e);
      coverUrl = null;
    }
  }

  function showToast(kind: "ok" | "err", msg: string) {
    toast = { kind, msg };
    setTimeout(() => (toast = null), 2400);
  }

  function notifyDocksDirty() {
    try {
      window.dispatchEvent(new CustomEvent("study:notes:dirty"));
    } catch {
      /* ignore */
    }
  }

  async function probeNotesAvailability() {
    try {
      await notesPagesList();
      error = "";
      notesTableMissing = false;
    } catch (e) {
      const raw = e instanceof Error ? e.message : String(e);
      console.error("[notes] probe failed:", raw);
      if (/no such table/i.test(raw) || /database\s+disk/i.test(raw)) {
        notesTableMissing = true;
        error = "";
      } else {
        notesTableMissing = false;
        error = raw;
      }
    }
  }

  async function openPage(id: number, registerTab: boolean = true) {
    loading = true;
    error = "";
    try {
      const p = await notesPagesGet(id);
      currentPage = p;
      aliasesEditor = p.aliases.join(", ");
      tagsEditor = p.tags.join(", ");
      aliasesDirty = false;
      tagsDirty = false;
      blockTree = await notesBlocksPageTree(id);
      notesShell.setActivePage(p.name, p.title ?? null);
      if (registerTab) {
        const wnd = tabsStore.activeWndId;
        if (wnd != null) {
          void tabsStore
            .openTab({ wndId: wnd, pageId: id, viewKind: "editor", activate: true })
            .catch(() => {});
        }
      }
      const initial = blockTree.length > 0 ? blockTree[0].content ?? "" : "";
      const trimmed = initial.trim();
      const words = trimmed
        ? trimmed.replace(/[`*_>#~\-\[\]\(\)\{\}!]+/g, " ").split(/\s+/).filter((w) => w.length > 0).length
        : 0;
      notesShell.setCounts(words, initial.length);
      notesShell.setSaving(false);
      await loadCover(id);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function reloadTree() {
    if (!currentPage) return;
    blockTree = await notesBlocksPageTree(currentPage.id);
  }

  async function createPage(name: string) {
    try {
      const r = await notesPagesCreate({ name });
      createPageOpen = false;
      notifyDocksDirty();
      await openPage(r.id);
      void awardXp("page_created", 15, { page_id: r.id, name });
      showToast("ok", $t("study.notes.page_created"));
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  async function deletePage() {
    if (!currentPage) return;
    try {
      await notesPagesDelete(currentPage.id);
      currentPage = null;
      blockTree = [];
      notesShell.setActivePage(null, null);
      notesShell.setCounts(0, 0);
      notesShell.setSaving(false);
      notifyDocksDirty();
      showToast("ok", $t("study.notes.page_removed"));
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  async function renamePage(newName: string) {
    if (!currentPage) return;
    if (newName === currentPage.name) {
      renameOpen = false;
      return;
    }
    try {
      const r = await notesPagesRenameCascade({ id: currentPage.id, newName });
      renameOpen = false;
      notifyDocksDirty();
      await openPage(currentPage.id);
      showToast(
        "ok",
        r.blocks_updated === 0
          ? $t("study.notes.page_renamed")
          : r.blocks_updated === 1
            ? $t("study.notes.page_renamed_block")
            : $t("study.notes.page_renamed_blocks", { n: r.blocks_updated }),
      );
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  async function saveAliases() {
    if (!currentPage || !aliasesDirty) return;
    const arr = aliasesEditor
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
    try {
      await notesPagesSetAliases({ id: currentPage.id, aliases: arr });
      aliasesDirty = false;
      showToast("ok", "Aliases salvos");
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  async function saveTags() {
    if (!currentPage || !tagsDirty) return;
    const arr = tagsEditor
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
    try {
      await notesPagesSetTags({ id: currentPage.id, tags: arr });
      tagsDirty = false;
      currentPage = { ...currentPage, tags: arr };
      showToast("ok", "Tags salvas");
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  async function gotoPageByName(name: string) {
    try {
      const p = await notesPagesResolve(name);
      if (p) {
        await openPage(p.id);
        notifyDocksDirty();
      } else {
        const r = await notesPagesEnsure({ name });
        await openPage(r.id);
        notifyDocksDirty();
      }
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  async function openJournalToday() {
    try {
      const r = await notesJournalToday();
      await openPage(r.page_id);
      notifyDocksDirty();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  async function undoLastOp() {
    try {
      await notesUndoLastOp();
      await reloadTree();
      showToast("ok", "Desfeito");
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (!msg.includes("no op to undo")) {
        showToast("err", msg);
      }
    }
  }

  async function redoLastOp() {
    try {
      await notesUndoRedoLast();
      await reloadTree();
      showToast("ok", "Refeito");
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (!msg.includes("no op to redo")) {
        showToast("err", msg);
      }
    }
  }

  onMount(async () => {
    await probeNotesAvailability();
    const target = $routePage.url.searchParams.get("page");
    if (target) {
      await gotoPageByName(target);
      return;
    }
    if (notesTableMissing || error) {
      loading = false;
      return;
    }
    try {
      const all = await notesPagesList();
      if (all.length > 0) {
        await openPage(all[0].id);
      } else {
        loading = false;
      }
    } catch {
      loading = false;
    }
  });

  $effect(() => {
    const t = tabsStore.activeTab;
    if (!t || t.view_kind !== "editor" || t.page_id == null) return;
    if (currentPage?.id === t.page_id) return;
    void openPage(t.page_id, false);
  });
</script>

<div class="notes-shell" data-surface="notes">
  <main class="editor">
    {#if toast}
      <div class="toast" class:err={toast.kind === "err"} role="status">
        {toast.msg}
      </div>
    {/if}

    {#if loading}
      <div class="state">Carregando…</div>
    {:else if notesTableMissing}
      <div class="state err">
        <p>{$t("study.library.error_loading_notes")}</p>
        <button class="btn" onclick={() => probeNotesAvailability()}>{$t("common.retry")}</button>
      </div>
    {:else if error}
      <div class="state err">
        <p>{error}</p>
        <button class="btn" onclick={() => probeNotesAvailability()}>{$t("common.retry")}</button>
      </div>
    {:else if !currentPage}
      <div class="state">
        {$t("study.library.notes_no_open_page")}
      </div>
    {:else}
      <PageHero
        coverUrl={coverUrl}
        title={currentPage.title ?? currentPage.name}
        subtitle={currentPage.name}
        onTitleClick={() => {
          renameValue = currentPage?.name ?? "";
          renameOpen = true;
        }}
      />
      <header class="editor-head">
        {#if !coverUrl}
          <button
            type="button"
            class="title"
            onclick={() => {
              renameValue = currentPage?.name ?? "";
              renameOpen = true;
            }}
            title="Click para renomear"
          >
            {currentPage.title ?? currentPage.name}
          </button>
        {/if}
        <Breadcrumb page={currentPage} onSegmentClick={(path) => gotoPageByName(path)} />
        <div class="head-actions">
          {#if currentPage.journal_day}
            <span class="badge">journal</span>
          {/if}
          <button
            class="btn ghost sm"
            onclick={() => (coverManagerOpen = true)}
            title={$t("study.notes.set_cover")}
          >
            {coverUrl ? $t("study.notes.cover_done") : $t("study.notes.add_cover")}
          </button>
          <button
            class="btn ghost sm"
            onclick={() => (historyOpen = true)}
            disabled={firstBlockId === null}
            title={$t("study.notes.view_root_history")}
          >
            🕐 {$t("study.notes.history")}
          </button>
          <button
            class="btn ghost sm"
            onclick={undoLastOp}
            title={$t("study.notes.undo_structural_hint")}
          >
            ↶
          </button>
          <button
            class="btn ghost sm"
            onclick={redoLastOp}
            title="Refazer (Cmd+Shift+Z)"
          >
            ↷
          </button>
          <a
            class="btn ghost sm"
            href="/study/notes/shortcuts"
            title="Atalhos do editor"
          >
            ?
          </a>
          <button
            class="btn ghost sm"
            onclick={() => window.print()}
            title="Imprimir / Salvar PDF (Ctrl+P)"
          >
            Imprimir/PDF
          </button>
          <button
            class="btn ghost sm danger"
            onclick={() => (confirmDeletePageOpen = true)}
          >
            Excluir página
          </button>
        </div>
      </header>

      <div class="page-meta-bar">
        <label class="meta-field">
          <span>Tags</span>
          <input
            type="text"
            placeholder="comma, separated"
            bind:value={tagsEditor}
            oninput={() => (tagsDirty = true)}
            onblur={saveTags}
          />
        </label>
        <label class="meta-field">
          <span>Aliases</span>
          <input
            type="text"
            placeholder="comma, separated"
            bind:value={aliasesEditor}
            oninput={() => (aliasesDirty = true)}
            onblur={saveAliases}
          />
        </label>
      </div>

      {@const firstBlock = blockTree.length > 0 ? blockTree[0] : null}
      {#if blockTree.length > 1}
        <p class="editor-banner">
          Esta página tem múltiplos blocos antigos. C1 edita só o primeiro;
          os demais ficam preservados no DB e voltam visíveis em C1.5.
        </p>
      {/if}
      <Editor
        pageId={currentPage.id}
        aggregateBlockId={firstBlock?.id ?? null}
        initialMarkdown={firstBlock?.content ?? ""}
        onSaved={() => { void reloadTree(); }}
        onError={(msg) => showToast("err", msg)}
        onStats={(s) => notesShell.setCounts(s.words, s.chars)}
        onSavingChange={(saving) => notesShell.setSaving(saving)}
      />
    {/if}
  </main>

</div>

<CoverManager
  open={coverManagerOpen}
  pageId={currentPage?.id ?? null}
  currentCoverUrl={coverUrl}
  onSaved={() => {
    if (currentPage) void loadCover(currentPage.id);
  }}
  onClose={() => (coverManagerOpen = false)}
/>

<HistoryModal
  open={historyOpen}
  blockId={firstBlockId}
  onRestored={() => {
    void reloadTree();
    showToast("ok", $t("study.notes.version_restored"));
  }}
  onClose={() => (historyOpen = false)}
/>

<CreatePageDialog
  open={createPageOpen}
  onConfirm={createPage}
  onClose={() => (createPageOpen = false)}
/>

<RenamePageDialog
  open={renameOpen}
  initialValue={renameValue}
  onConfirm={renamePage}
  onClose={() => (renameOpen = false)}
/>

<ConfirmDialog
  bind:open={confirmDeletePageOpen}
  title={$t("study.notes.delete_page")}
  message={currentPage
    ? $t("study.notes.delete_confirm", {
        name: currentPage.title ?? currentPage.name,
      })
    : ""}
  confirmLabel={$t("study.common.delete")}
  variant="danger"
  onConfirm={deletePage}
/>

<style>
  .notes-shell {
    height: 100%;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .editor-banner {
    margin: 0;
    padding: 8px 12px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border-left: 3px solid var(--accent);
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-size: 12px;
    line-height: 1.5;
  }

  .editor {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 20px;
    overflow-y: auto;
  }
  .state {
    padding: calc(var(--padding) * 2);
    text-align: center;
    color: var(--secondary);
    font-size: 14px;
  }
  .state.err {
    color: var(--error, var(--accent));
  }

  .editor-head {
    display: flex;
    align-items: baseline;
    gap: 12px;
    flex-wrap: wrap;
  }
  .title {
    background: transparent;
    border: 0;
    padding: 0;
    font: inherit;
    font-size: 22px;
    font-weight: 600;
    color: var(--text);
    cursor: text;
  }
  .title:hover {
    color: var(--accent);
  }
  .head-actions {
    margin-left: auto;
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .badge {
    padding: 2px 8px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .page-meta-bar {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .meta-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    color: var(--tertiary);
  }
  .meta-field input {
    padding: 6px 10px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .meta-field input:focus {
    outline: none;
    border-color: var(--accent);
  }

</style>
