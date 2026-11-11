<script lang="ts">
  import { open as shellOpen } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { sortRows } from "$lib/research/workflows";
  import type { Cell, Table } from "$lib/research/types";

  let { table }: { table: Table } = $props();

  let sortKey = $state<string | null>(null);
  let descending = $state(true);

  let sorted = $derived(sortKey ? sortRows(table.rows, sortKey, descending) : table.rows);

  function sortBy(key: string) {
    if (sortKey === key) descending = !descending;
    else {
      sortKey = key;
      descending = true;
    }
  }

  function number(cell: Cell): string {
    return typeof cell === "number" ? cell.toLocaleString(undefined, { maximumFractionDigits: 2 }) : "—";
  }

  function confidence(value: string): string {
    return ["low", "medium", "high"].includes(value) ? ($t(`research.confidence.${value}`) as string) : value;
  }
</script>

<section class="dv-panel" aria-label={$t(`research.table.${table.id}`) as string}>
  <div class="dv-bar">
    <h3 class="dv-title">{$t(`research.table.${table.id}`)}</h3>
    <span class="dv-meta">{table.rows.length.toLocaleString()}</span>
  </div>
  {#if table.rows.length === 0}
    <p class="dv-meta">{$t("research.empty_table")}</p>
  {:else}
    <div class="dv-scroll">
      <table class="dv-table rs-table">
        <thead>
          <tr>
            {#each table.columns as column (column.key)}
              <th class:num={column.kind === "number"} aria-sort={sortKey === column.key ? (descending ? "descending" : "ascending") : "none"}>
                <button type="button" class="rs-sort" title={$t("research.sort_by", { column: $t(`research.col.${column.key}`) }) as string} onclick={() => sortBy(column.key)}>
                  {$t(`research.col.${column.key}`)}
                  {#if sortKey === column.key}<span aria-hidden="true">{descending ? "↓" : "↑"}</span>{/if}
                </button>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each sorted as row, i (i)}
            <tr>
              {#each table.columns as column (column.key)}
                {@const cell = row[column.key] ?? null}
                <td class:num={column.kind === "number"} class:flags={column.kind === "flags"}>
                  {#if Array.isArray(cell)}
                    {#each cell as flag (flag)}<span class="tag tag-warning rs-flag">{flag}</span>{/each}
                  {:else if cell !== null && typeof cell === "object"}
                    <a href={cell.href} class="rs-link" onclick={(e) => { e.preventDefault(); void shellOpen(cell.href).catch(() => {}); }}>{cell.label}</a>
                  {:else if column.kind === "number"}
                    {number(cell)}
                  {:else if cell === null}
                    <span class="dim">—</span>
                  {:else if column.key === "confidence"}
                    <span class="tag" class:tag-warning={cell === "low"} class:tag-info={cell === "medium"} class:tag-success={cell === "high"}>{confidence(String(cell))}</span>
                  {:else}
                    {cell}
                  {/if}
                </td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</section>

<style>
  .rs-table th {
    padding: 0;
    white-space: nowrap;
  }

  .rs-table td {
    max-width: 340px;
    vertical-align: top;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .rs-table .flags {
    min-width: 200px;
  }

  .rs-table .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .rs-sort {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    width: 100%;
    padding: var(--space-2);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .num .rs-sort {
    justify-content: flex-end;
  }

  @media (hover: hover) {
    .rs-sort:hover {
      background: var(--fill-1);
    }
  }

  .rs-sort:focus-visible,
  .rs-link:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .rs-link {
    color: var(--accent-hi);
    text-decoration: none;
  }

  @media (hover: hover) {
    .rs-link:hover {
      text-decoration: underline;
    }
  }

  .rs-flag {
    display: inline-block;
    margin: 0 var(--space-1) var(--space-1) 0;
    white-space: normal;
  }

  .dim {
    color: var(--text-dim);
  }
</style>
