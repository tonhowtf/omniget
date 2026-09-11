<script lang="ts">
  /** Tabela de PDF → CSV e XLSX, por posição do texto (modo "stream"). */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openPath, pct, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Table = { page: number; pages: number[]; rows: number; cols: number; cells: string[][]; csv: string | null };
  type Result = { input: string; pages: number; tables: Table[]; outputs: string[]; xlsx: string | null };

  const PDF = [{ name: "PDF", extensions: ["pdf"] }];

  let input = $state("");
  let pages = $state("");
  let outputDir = $state("");
  let format = $state("both");
  let minGap = $state(5);
  let mergePages = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | undefined;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "pdf-table-extract") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function go(previewOnly: boolean) {
    if (!input || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_pdf_tables", {
        opts: {
          input,
          pages,
          output_dir: outputDir,
          format,
          min_gap: minGap,
          min_rows: 2,
          min_cols: 2,
          merge_pages: mergePages,
          preview_only: previewOnly,
        },
      });
      showToast(result.tables.length ? "success" : "info", `${result.tables.length} ${$t("tools.tables.found")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function copyCsv(table: Table) {
    await navigator.clipboard.writeText(table.csv ?? "");
    showToast("success", $t("tools.common.copied") as string);
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{input ? baseName(input) : $t("tools.tables.file")}</div>
          <div class="group-row-sub">{$t("tools.tables.file_sub")}</div>
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(PDF); if (f) { input = f; result = null; } }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.tables.pages")}</div><div class="group-row-sub">{$t("tools.tables.pages_sub")}</div></div>
        <div class="group-row-trailing"><input class="input mono" type="text" placeholder="1-5, 8" bind:value={pages} style:width="9em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.tables.format")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={format}>
            <option value="csv">CSV</option>
            <option value="xlsx">XLSX</option>
            <option value="both">{$t("tools.tables.both")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.tables.gap")} <span class="dim">· {minGap} pt</span></div><div class="group-row-sub">{$t("tools.tables.gap_sub")}</div></div>
        <div class="group-row-trailing"><input type="range" min="2" max="30" step="1" bind:value={minGap} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.tables.merge")}</div><div class="group-row-sub">{$t("tools.tables.merge_sub")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={mergePages} type="button" role="switch" aria-checked={mergePages} aria-label={$t("tools.tables.merge") as string} onclick={() => (mergePages = !mergePages)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outputDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing btn-row">
          {#if outputDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (outputDir = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outputDir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary" type="button" disabled={busy || !input} onclick={() => go(true)}>{$t("tools.tables.scan")}</button>
          <button class="btn btn-primary" type="button" disabled={busy || !input} onclick={() => go(false)}>{busy ? $t("tools.common.working") : $t("tools.tables.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.tables.length} {$t("tools.tables.found")}</div>
            <div class="group-row-sub">{result.pages} {$t("tools.tables.pages_done")}</div>
          </div>
          {#if result.outputs.length}
            <div class="group-row-trailing btn-row">
              {#if result.xlsx}<button class="btn btn-ghost btn-sm" type="button" onclick={() => openPath(result!.xlsx!)}>{$t("tools.common.open")}</button>{/if}
              <button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.outputs[0])}>{$t("tools.common.reveal")}</button>
            </div>
          {/if}
        </div>
        {#each result.tables as tb, i (i)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t("tools.tables.table")} {i + 1} <span class="dim">· {$t("tools.tables.page")} {tb.pages.join(", ")} · {tb.rows}×{tb.cols}</span></div>
              <div class="scroll">
                <table>
                  <tbody>
                    {#each tb.cells.slice(0, 8) as row, r (r)}
                      <tr>{#each row as cell, c (c)}<td>{cell}</td>{/each}</tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            </div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => copyCsv(tb)}>{$t("tools.common.copy")}</button></div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .scroll { overflow-x: auto; margin-top: var(--space-2); }
  table { border-collapse: collapse; font-size: var(--text-xs); }
  td { border: 1px solid var(--border); padding: 2px 6px; white-space: nowrap; max-width: 16rem; overflow: hidden; text-overflow: ellipsis; }
</style>
