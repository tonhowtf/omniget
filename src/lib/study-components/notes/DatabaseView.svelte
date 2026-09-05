<script lang="ts">
  import { onMount } from "svelte";
  import { notesQueryRun, type QuerySort } from "$lib/notes-bridge";

  type Props = {
    expr: string;
    initialSort?: QuerySort;
    onEdit?: (newExpr: string) => void;
  };

  let { expr, initialSort = "updated-desc", onEdit }: Props = $props();

  type BlockResultRow = {
    id: number;
    page_id: number;
    content: string;
    properties: Record<string, unknown>;
    updated_at: number;
  };

  type State =
    | { kind: "idle" }
    | { kind: "loading" }
    | { kind: "error"; message: string }
    | {
        kind: "ok";
        rows: BlockResultRow[];
        total: number;
        has_more: boolean;
        offset: number;
      };

  let qState = $state<State>({ kind: "idle" });
  let sort = $state<QuerySort>("updated-desc");
  let limit = $state(50);
  let offset = $state(0);
  let editingExpr = $state(false);
  let exprDraft = $state("");

  $effect(() => {
    sort = initialSort;
  });

  function snippetOf(content: string, max = 80): string {
    const trimmed = content.trim().replace(/\s+/g, " ");
    return trimmed.length > max ? trimmed.slice(0, max - 1) + "…" : trimmed;
  }

  function fmtDate(secs: number): string {
    if (!secs) return "—";
    return new Date(secs * 1000).toLocaleDateString("pt-BR");
  }

  function statusOf(props: Record<string, unknown>): string {
    return typeof props.status === "string" ? props.status : "";
  }

  function deadlineOf(props: Record<string, unknown>): string {
    return typeof props.deadline === "string" ? props.deadline : "";
  }

  function statusColor(status: string): string {
    switch (status) {
      case "TODO":
      case "NOW":
        return "var(--accent)";
      case "DOING":
        return "var(--warning)";
      case "DONE":
        return "var(--success)";
      case "WAITING":
      case "LATER":
        return "var(--tertiary)";
      case "CANCELED":
        return "var(--error)";
      default:
        return "var(--tertiary)";
    }
  }

  async function load() {
    if (!expr.trim()) {
      qState = { kind: "ok", rows: [], total: 0, has_more: false, offset: 0 };
      return;
    }
    qState = { kind: "loading" };
    try {
      const result = await notesQueryRun({
        expr: expr.trim(),
        sort,
        limit,
        offset,
      });
      const rows = result.blocks.map((b) => ({
        id: b.id,
        page_id: b.page_id,
        content: b.content,
        properties: b.properties as Record<string, unknown>,
        updated_at: b.updated_at,
      }));
      qState = {
        kind: "ok",
        rows,
        total: result.total,
        has_more: result.has_more,
        offset,
      };
    } catch (e) {
      qState = {
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
      };
    }
  }

  function changeSort(next: QuerySort) {
    if (sort === next) return;
    sort = next;
    offset = 0;
    void load();
  }

  function nextPage() {
    if (qState.kind !== "ok" || !qState.has_more) return;
    offset = offset + limit;
    void load();
  }

  function prevPage() {
    if (offset === 0) return;
    offset = Math.max(0, offset - limit);
    void load();
  }

  function commitExpr() {
    const next = exprDraft.trim();
    editingExpr = false;
    if (!next || next === expr) return;
    onEdit?.(next);
  }

  $effect(() => {
    void expr;
    void load();
  });

  onMount(() => {
    void load();
  });
</script>

<div class="db-view" data-view="table">
  <header class="db-head">
    <div class="expr-row">
      <span class="expr-label">{`{{query`}}</span>
      {#if editingExpr}
        <input
          class="expr-input"
          bind:value={exprDraft}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commitExpr();
            } else if (e.key === "Escape") {
              editingExpr = false;
              exprDraft = expr;
            }
          }}
          onblur={commitExpr}
        />
      {:else}
        <button
          type="button"
          class="expr-display"
          onclick={() => {
            if (!onEdit) return;
            editingExpr = true;
            exprDraft = expr;
          }}
          disabled={!onEdit}
        >{expr || "(vazio)"}</button>
      {/if}
      <span class="expr-label">{`}}`}</span>
    </div>
    <div class="meta">
      {#if qState.kind === "ok"}
        {qState.offset + 1}–{Math.min(qState.offset + qState.rows.length, qState.total)} de {qState.total}
      {/if}
    </div>
  </header>

  {#if qState.kind === "loading"}
    <div class="db-qState">executando query…</div>
  {:else if qState.kind === "error"}
    <div class="db-qState err">erro: {qState.message}</div>
  {:else if qState.kind === "ok"}
    {#if qState.rows.length === 0}
      <div class="db-qState">sem resultados</div>
    {:else}
      <table class="db-table">
        <thead>
          <tr>
            <th>conteúdo</th>
            <th class="th-narrow">
              <button
                type="button"
                class="sort-btn"
                class:active={sort === "status"}
                onclick={() => changeSort("status")}
              >status {sort === "status" ? "↓" : ""}</button>
            </th>
            <th class="th-narrow">deadline</th>
            <th class="th-narrow">
              <button
                type="button"
                class="sort-btn"
                class:active={sort === "updated-desc"}
                onclick={() =>
                  changeSort(sort === "updated-desc" ? "updated-asc" : "updated-desc")}
              >
                atualizado
                {sort === "updated-desc" ? "↓" : sort === "updated-asc" ? "↑" : ""}
              </button>
            </th>
          </tr>
        </thead>
        <tbody>
          {#each qState.rows as row (row.id)}
            {@const status = statusOf(row.properties)}
            {@const deadline = deadlineOf(row.properties)}
            <tr>
              <td class="td-content">
                <a href={`/study/notes?page=${row.page_id}`}>
                  {snippetOf(row.content)}
                </a>
              </td>
              <td class="th-narrow">
                {#if status}
                  <span
                    class="status-pill"
                    style:color={statusColor(status)}
                    style:border-color={statusColor(status)}
                  >{status}</span>
                {/if}
              </td>
              <td class="th-narrow mono">{deadline}</td>
              <td class="th-narrow mono">{fmtDate(row.updated_at)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
      <footer class="db-foot">
        <button
          type="button"
          class="page-btn"
          onclick={prevPage}
          disabled={qState.offset === 0}
        >‹ anterior</button>
        <button
          type="button"
          class="page-btn"
          onclick={nextPage}
          disabled={!qState.has_more}
        >próximo ›</button>
      </footer>
    {/if}
  {/if}
</div>

<style>
  .db-view {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px;
    background: color-mix(in oklab, var(--input-border) 12%, transparent);
    border: none;
    border-radius: var(--border-radius);
    font-size: 12px;
    contain: layout;
  }
  .db-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-wrap: wrap;
  }
  .expr-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1 1 auto;
    min-width: 0;
  }
  .expr-label {
    font-family: var(--font-mono, ui-monospace, monospace);
    color: var(--tertiary);
    font-size: 11px;
  }
  .expr-display {
    flex: 1 1 auto;
    text-align: left;
    background: transparent;
    border: 0;
    color: var(--accent);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
    cursor: pointer;
    padding: 2px 4px;
    border-radius: 3px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .expr-display:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .expr-display:disabled {
    cursor: default;
  }
  .expr-input {
    flex: 1 1 auto;
    background: var(--bg);
    border: 1px solid var(--accent);
    color: var(--text);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
    padding: 2px 6px;
    border-radius: 3px;
    outline: none;
    min-width: 0;
  }
  .meta {
    color: var(--tertiary);
    font-size: 11px;
  }
  .db-qState {
    padding: 10px;
    color: var(--tertiary);
    font-size: 12px;
    text-align: center;
  }
  .db-qState.err {
    color: var(--error, var(--accent));
  }
  .db-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  .db-table thead th {
    padding: 4px 6px;
    text-align: left;
    font-size: 10px;
    text-transform: uppercase;
    color: var(--tertiary);
    letter-spacing: 0.05em;
    font-weight: 600;
    border-bottom: none;
  }
  .db-table tbody td {
    padding: 4px 6px;
    border-bottom: none;
    vertical-align: top;
  }
  .db-table tbody tr:last-child td {
    border-bottom: 0;
  }
  .td-content a {
    color: var(--text);
    text-decoration: none;
  }
  .td-content a:hover {
    color: var(--accent);
    text-decoration: underline;
  }
  .th-narrow {
    width: 1%;
    white-space: nowrap;
  }
  .mono {
    font-family: var(--font-mono, ui-monospace, monospace);
    color: var(--tertiary);
    font-size: 11px;
  }
  .sort-btn {
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
    padding: 0;
  }
  .sort-btn.active {
    color: var(--accent);
  }
  .status-pill {
    display: inline-block;
    padding: 1px 6px;
    border: 1px solid;
    border-radius: 4px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.04em;
    background: transparent;
  }
  .db-foot {
    display: flex;
    justify-content: flex-end;
    gap: 4px;
    margin-top: 4px;
  }
  .page-btn {
    padding: 2px 8px;
    border: none;
    border-radius: var(--border-radius);
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  .page-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .page-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
