<script lang="ts">
  import {
    notesSearchRebuild,
    notesRefsRebuildAll,
    notesQueryInvalidateCache,
    notesExportGraphJson,
    notesMarkdownImport,
  } from "$lib/notes-bridge";
  import OpLogViewer from "./OpLogViewer.svelte";

  type Props = {
    onToast: (kind: "ok" | "err", msg: string) => void;
  };

  let { onToast }: Props = $props();

  let busy = $state<string | null>(null);
  let importPreview = $state<{ name: string; markdown: string; lines: number } | null>(null);
  let importing = $state(false);
  let fileInput = $state<HTMLInputElement | null>(null);

  async function rebuildSearch() {
    busy = "search";
    try {
      const r = await notesSearchRebuild();
      onToast("ok", `Busca reindexada: ${r.indexed} blocos`);
    } catch (e) {
      onToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      busy = null;
    }
  }

  async function rebuildRefs() {
    busy = "refs";
    try {
      const r = await notesRefsRebuildAll();
      onToast("ok", `Backlinks reconstruídos: ${r.total_refs} refs`);
    } catch (e) {
      onToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      busy = null;
    }
  }

  async function clearQueryCache() {
    busy = "qcache";
    try {
      const r = await notesQueryInvalidateCache();
      onToast("ok", `Cache limpo (${r.size_after} entradas restantes)`);
    } catch (e) {
      onToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      busy = null;
    }
  }

  async function exportGraph() {
    busy = "graph";
    try {
      const r = await notesExportGraphJson();
      const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
      const blob = new Blob([r.json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `notes-graph-${stamp}.json`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      setTimeout(() => URL.revokeObjectURL(url), 1000);
      onToast("ok", "Grafo exportado");
    } catch (e) {
      onToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      busy = null;
    }
  }

  function pickImport() {
    fileInput?.click();
  }

  async function onFileChosen(e: Event) {
    const target = e.target as HTMLInputElement;
    const file = target.files?.[0];
    if (!file) return;
    try {
      const text = await file.text();
      const baseName = file.name.replace(/\.(md|markdown|txt)$/i, "");
      importPreview = {
        name: baseName,
        markdown: text,
        lines: text.split("\n").length,
      };
    } catch (err) {
      onToast("err", err instanceof Error ? err.message : String(err));
    } finally {
      target.value = "";
    }
  }

  function cancelImport() {
    importPreview = null;
  }

  async function confirmImport() {
    if (!importPreview) return;
    importing = true;
    try {
      const r = await notesMarkdownImport({
        name: importPreview.name,
        markdown: importPreview.markdown,
      });
      onToast("ok", `Importado: ${r.blocks_created} blocos em "${importPreview.name}"`);
      importPreview = null;
    } catch (e) {
      onToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      importing = false;
    }
  }
</script>

<article class="card">
  <h3>Manutenção</h3>
  <p class="hint">
    Tarefas de housekeeping. Reconstruir índices é seguro mas pode levar alguns segundos
    em databases grandes.
  </p>

  <div class="actions-grid">
    <button
      type="button"
      class="btn"
      onclick={rebuildSearch}
      disabled={busy !== null}
      title="Reindexa FTS5 (notes_v2). Use se a busca estiver retornando resultados desatualizados."
    >
      {busy === "search" ? "Reindexando…" : "Reconstruir busca"}
    </button>
    <button
      type="button"
      class="btn"
      onclick={rebuildRefs}
      disabled={busy !== null}
      title="Recalcula a tabela de refs/backlinks varrendo todo o conteúdo."
    >
      {busy === "refs" ? "Calculando…" : "Reconstruir backlinks"}
    </button>
    <button
      type="button"
      class="btn"
      onclick={clearQueryCache}
      disabled={busy !== null}
      title="Limpa cache de queries. Inofensivo; queries serão recalculadas."
    >
      {busy === "qcache" ? "Limpando…" : "Limpar cache de queries"}
    </button>
    <button
      type="button"
      class="btn"
      onclick={exportGraph}
      disabled={busy !== null}
      title="Baixa o grafo de notes (nodes + edges) como JSON pra inspeção/backup."
    >
      {busy === "graph" ? "Exportando…" : "Exportar grafo (JSON)"}
    </button>
    <button
      type="button"
      class="btn"
      onclick={pickImport}
      disabled={busy !== null || importing}
      title="Lê um .md do disco e cria uma página com o conteúdo."
    >
      Importar markdown
    </button>
    <input
      type="file"
      accept=".md,.markdown,.txt,text/markdown,text/plain"
      bind:this={fileInput}
      onchange={onFileChosen}
      style:display="none"
    />
  </div>

  <OpLogViewer {onToast} />
</article>

{#if importPreview}
  <div
    class="modal-bg"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) cancelImport();
    }}
  >
    <div class="modal" role="dialog" aria-label="Confirmar importação" aria-modal="true">
      <h3>Importar markdown?</h3>
      <p class="meta">
        <strong>{importPreview.name}</strong>
        <span class="muted">· {importPreview.lines} linhas</span>
      </p>
      <p class="hint">
        Vai criar uma página chamada <code>{importPreview.name}</code> e parsear o markdown
        em blocos hierárquicos.
      </p>

      <p class="warn-soft">
        Se já existir uma página com esse nome, o backend devolve erro e nada
        é importado.
      </p>

      <footer class="foot">
        <span class="spacer"></span>
        <button
          type="button"
          class="btn ghost"
          onclick={cancelImport}
          disabled={importing}
        >
          Cancelar
        </button>
        <button
          type="button"
          class="btn primary"
          onclick={confirmImport}
          disabled={importing}
        >
          {importing ? "Importando…" : "Importar"}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .card {
    padding: 14px 16px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .card h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--secondary);
  }
  .hint {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
    line-height: 1.5;
  }
  .actions-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 8px;
  }
  .btn {
    padding: 8px 12px;
    border-radius: var(--border-radius);
    border: none;
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    text-align: left;
    transition: background 120ms ease, border-color 120ms ease;
  }
  .btn:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border-color: color-mix(in oklab, var(--accent) 40%, var(--input-border));
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
    border-color: var(--accent);
  }
  .btn.ghost {
    background: transparent;
  }
  .modal-bg {
    position: fixed;
    inset: 0;
    z-index: 95;
    background: color-mix(in oklab, black 50%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }
  .modal {
    width: min(480px, calc(100vw - 48px));
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    padding: 18px 20px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .modal h3 {
    margin: 0;
    font-size: 15px;
  }
  .meta {
    margin: 0;
    font-size: 13px;
  }
  .muted {
    color: var(--tertiary);
    font-size: 11px;
    margin-left: 4px;
  }
  .warn-soft {
    margin: 0;
    padding: 8px 10px;
    background: color-mix(in oklab, var(--warning) 12%, transparent);
    border-radius: var(--border-radius);
    color: var(--warning);
    font-size: 12px;
  }
  code {
    padding: 1px 5px;
    background: var(--bg);
    border-radius: 3px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }
  .spacer {
    flex: 1 1 auto;
  }
</style>
