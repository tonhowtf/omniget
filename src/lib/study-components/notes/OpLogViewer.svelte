<script lang="ts">
  import { notesUndoListOps, notesUndoLastOp, type OpSummary } from "$lib/notes-bridge";
import { t } from "$lib/i18n";

  type Props = {
    onToast: (kind: "ok" | "err", msg: string) => void;
  };

  let { onToast }: Props = $props();

  let ops = $state<OpSummary[]>([]);
  let loading = $state(false);
  let busy = $state<string | null>(null);
  let loaded = $state(false);

  async function refresh() {
    loading = true;
    try {
      ops = await notesUndoListOps(50);
      loaded = true;
    } catch (e) {
      onToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  async function undoOp(opId: string) {
    busy = opId;
    try {
      const r = await notesUndoLastOp(opId);
      onToast("ok", `Desfeito: ${r.kind} (${r.blocks_affected} blocos)`);
      await refresh();
    } catch (e) {
      onToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      busy = null;
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

  function onToggle(e: Event) {
    const open = (e.currentTarget as HTMLDetailsElement).open;
    if (open && !loaded) void refresh();
  }
</script>

<details class="op-log" ontoggle={onToggle}>
  <summary>
    <span>Histórico de operações (op-log)</span>
    <span class="caret" aria-hidden="true">▸</span>
  </summary>

  <div class="body">
    <div class="actions">
      <button type="button" class="btn ghost sm" onclick={refresh} disabled={loading}>
        {loading ? "Carregando…" : "Atualizar"}
      </button>
      <span class="hint">Últimas 50 operações. Você pode desfazer ops antigas, não só a última.</span>
    </div>

    {#if !loaded && !loading}
      <p class="muted">Expanda para carregar.</p>
    {:else if ops.length === 0}
      <p class="muted">Sem operações registradas ainda.</p>
    {:else}
      <ul class="list">
        {#each ops as op (op.op_id)}
          <li class="row" class:undone={op.undone}>
            <span class="kind">{op.kind}</span>
            <span class="when">{fmtTime(op.created_at)}</span>
            <span class="rows-count">{op.row_count} {op.row_count === 1 ? "row" : "rows"}</span>
            {#if op.undone}
              <span class="badge undone">desfeito</span>
            {/if}
            <button
              type="button"
              class="btn ghost sm"
              onclick={() => undoOp(op.op_id)}
              disabled={op.undone || busy === op.op_id}
              title={op.undone ? $t("study.notes.nb.already_undone") : $t("study.notes.nb.undo_op")}
            >
              {busy === op.op_id ? "…" : "Desfazer"}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</details>

<style>
  .op-log {
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
  }
  summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    list-style: none;
  }
  summary::-webkit-details-marker {
    display: none;
  }
  .caret {
    font-size: 11px;
    color: var(--tertiary);
    transition: transform 120ms ease;
  }
  .op-log[open] .caret {
    transform: rotate(90deg);
  }
  .body {
    padding: 0 14px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .hint {
    color: var(--tertiary);
    font-size: 11px;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 320px;
    overflow-y: auto;
  }
  .row {
    display: grid;
    grid-template-columns: 100px 1fr auto auto auto;
    gap: 10px;
    align-items: center;
    padding: 6px 10px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    font-size: 12px;
  }
  .row.undone {
    opacity: 0.55;
  }
  .kind {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-weight: 600;
    color: var(--accent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    color: var(--secondary);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
  }
  .rows-count {
    color: var(--tertiary);
    font-size: 11px;
  }
  .badge.undone {
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--tertiary) 20%, transparent);
    color: var(--tertiary);
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .muted {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
    font-style: italic;
  }
  .btn {
    padding: 4px 10px;
    border-radius: var(--border-radius);
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  .btn.sm {
    font-size: 11px;
    padding: 3px 8px;
  }
  .btn.ghost:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
