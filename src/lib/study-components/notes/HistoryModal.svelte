<script lang="ts">
  import {
    notesUndoHistory,
    notesUndoRestoreTo,
    notesUndoClear,
    notesBlocksGet,
    type ContentSnapshot,
  } from "$lib/notes-bridge";
  import DiffView from "./DiffView.svelte";

  type Props = {
    open: boolean;
    blockId: number | null;
    onRestored?: () => void;
    onClose: () => void;
  };

  let { open, blockId, onRestored, onClose }: Props = $props();

  let snapshots = $state<ContentSnapshot[]>([]);
  let currentContent = $state<string>("");
  let selectedId = $state<number | null>(null);
  let loading = $state(false);
  let error = $state<string>("");
  let restoring = $state(false);
  let confirmClearOpen = $state(false);
  let clearing = $state(false);

  let selected = $derived(
    snapshots.find((s) => s.id === selectedId) ?? null,
  );

  $effect(() => {
    if (open && blockId !== null) {
      void load();
    } else if (!open) {
      snapshots = [];
      selectedId = null;
      error = "";
    }
  });

  async function load() {
    if (blockId === null) return;
    loading = true;
    error = "";
    try {
      const [hist, block] = await Promise.all([
        notesUndoHistory({ blockId, limit: 50 }),
        notesBlocksGet(blockId),
      ]);
      snapshots = hist;
      currentContent = block.content ?? "";
      if (hist.length > 0) selectedId = hist[0].id;
      else selectedId = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function restoreSelected() {
    if (!selected || blockId === null) return;
    restoring = true;
    error = "";
    try {
      await notesUndoRestoreTo({ blockId, snapshotId: selected.id });
      onRestored?.();
      onClose();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      restoring = false;
    }
  }

  async function clearAll() {
    if (blockId === null) return;
    clearing = true;
    error = "";
    try {
      await notesUndoClear(blockId);
      confirmClearOpen = false;
      await load();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      clearing = false;
    }
  }

  function fmtTime(ts: number): string {
    try {
      const d = new Date(ts * 1000);
      return d.toLocaleString();
    } catch {
      return String(ts);
    }
  }

  function preview(text: string): string {
    const first = text.split("\n").find((l) => l.trim().length > 0) ?? "";
    return first.length > 80 ? first.slice(0, 80) + "…" : first || "(vazio)";
  }
</script>

{#if open}
  <div
    class="modal-bg"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
  >
    <div class="modal" role="dialog" aria-label="Histórico do bloco" aria-modal="true">
      <header class="head">
        <h3>Histórico do bloco</h3>
        <button type="button" class="btn ghost sm" onclick={onClose}>×</button>
      </header>

      {#if loading}
        <div class="state muted">Carregando…</div>
      {:else if blockId === null}
        <div class="state muted">Selecione um bloco para ver seu histórico.</div>
      {:else if error}
        <div class="state err">{error}</div>
      {:else if snapshots.length === 0}
        <div class="state muted">
          Nenhum snapshot ainda. O backend grava snapshots automaticamente
          em edições; abra esta página em sessões diferentes para acumular versões.
        </div>
      {:else}
        <div class="layout">
          <ul class="list" role="listbox" aria-label="Versões">
            {#each snapshots as s, i (s.id)}
              <li>
                <button
                  type="button"
                  class="item"
                  class:active={selectedId === s.id}
                  onclick={() => (selectedId = s.id)}
                >
                  <span class="when">{fmtTime(s.created_at)}</span>
                  <span class="prev">{preview(s.content)}</span>
                  {#if i === 0}
                    <span class="badge latest">mais recente</span>
                  {/if}
                </button>
              </li>
            {/each}
          </ul>

          <div class="detail">
            {#if selected}
              <div class="detail-meta">
                Diff: snapshot ({fmtTime(selected.created_at)}) → atual
              </div>
              <DiffView oldText={selected.content} newText={currentContent} />
            {:else}
              <div class="state muted">Escolha uma versão.</div>
            {/if}
          </div>
        </div>
      {/if}

      <footer class="foot">
        {#if snapshots.length > 0 && blockId !== null}
          <button
            type="button"
            class="btn ghost danger"
            onclick={() => (confirmClearOpen = true)}
            disabled={restoring}
          >
            Limpar histórico ({snapshots.length})
          </button>
        {/if}
        <span class="spacer"></span>
        <button type="button" class="btn ghost" onclick={onClose} disabled={restoring}>
          Fechar
        </button>
        <button
          type="button"
          class="btn primary"
          onclick={restoreSelected}
          disabled={!selected || restoring || blockId === null}
        >
          {restoring ? "Restaurando…" : "Restaurar esta versão"}
        </button>
      </footer>
    </div>
  </div>
{/if}

{#if confirmClearOpen}
  <div
    class="modal-bg confirm"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) confirmClearOpen = false;
    }}
  >
    <div class="modal small" role="dialog" aria-label="Limpar histórico" aria-modal="true">
      <h3>Limpar histórico?</h3>
      <p class="warn">
        {snapshots.length === 1
          ? "Isso apaga o snapshot deste bloco. Não dá pra desfazer."
          : `Isso apaga ${snapshots.length} snapshots deste bloco. Não dá pra desfazer.`}
      </p>
      <footer class="foot">
        <span class="spacer"></span>
        <button
          type="button"
          class="btn ghost"
          onclick={() => (confirmClearOpen = false)}
          disabled={clearing}
        >
          Cancelar
        </button>
        <button
          type="button"
          class="btn danger"
          onclick={clearAll}
          disabled={clearing}
        >
          {clearing ? "Apagando…" : "Apagar tudo"}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
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
  .modal-bg.confirm {
    z-index: 96;
  }
  .modal {
    width: min(900px, calc(100vw - 48px));
    max-height: calc(100vh - 80px);
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 18px;
    overflow: hidden;
  }
  .modal.small {
    width: min(480px, calc(100vw - 48px));
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .head h3 {
    margin: 0;
    font-size: 15px;
  }
  .layout {
    display: grid;
    grid-template-columns: 280px 1fr;
    gap: 12px;
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
  }
  @media (max-width: 720px) {
    .layout {
      grid-template-columns: 1fr;
    }
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
  }
  .list li {
    border-bottom: none;
  }
  .list li:last-child {
    border-bottom: 0;
  }
  .item {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: 10px 12px;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .item:hover {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .item.active {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
  }
  .when {
    font-size: 11px;
    color: var(--secondary);
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .prev {
    font-size: 13px;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    align-self: flex-start;
    padding: 1px 6px;
    border-radius: 999px;
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: 600;
  }
  .badge.latest {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .detail {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
    overflow: hidden;
  }
  .detail-meta {
    font-size: 11px;
    color: var(--tertiary);
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .state {
    padding: 24px;
    text-align: center;
    font-size: 13px;
  }
  .state.muted {
    color: var(--tertiary);
  }
  .state.err {
    color: var(--error);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .spacer {
    flex: 1 1 auto;
  }
  .warn {
    margin: 0;
    font-size: 13px;
    color: var(--text);
  }
  .btn {
    padding: 6px 14px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
  }
  .btn.sm {
    padding: 2px 8px;
    font-size: 14px;
    line-height: 1;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
  }
  .btn.primary:disabled,
  .btn.danger:disabled,
  .btn.ghost:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn.danger {
    background: var(--error);
    color: white;
    border-color: var(--error);
  }
  .btn.ghost.danger {
    background: transparent;
    color: var(--error);
    border-color: color-mix(in oklab, var(--error) 40%, var(--input-border));
  }
  .btn.ghost.danger:hover:not(:disabled) {
    background: color-mix(in oklab, var(--error) 10%, transparent);
  }
</style>
