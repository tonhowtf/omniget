<script lang="ts">
  /** Reparar PDF que não abre: Ghostscript quando existe, senão reconstrução da xref. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, pct, pickDir, pickFiles, reveal, type ToolProgress } from "$lib/tools/rt";

  type Item = {
    input: string;
    output: string | null;
    method: string | null;
    opened_before: boolean;
    pages_before: number | null;
    pages_after: number | null;
    objects_found: number;
    bytes_before: number;
    bytes_after: number;
    ok: boolean;
    error: string | null;
  };
  type Result = { items: Item[]; ghostscript: string | null };
  type Status = { ghostscript: string | null };

  const PDF_FILTER = [{ name: "PDF", extensions: ["pdf"] }];

  let status = $state<Status | null>(null);
  let files = $state<string[]>([]);
  let mode = $state("auto");
  let outDir = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    status = await invoke<Status>("tool_pdf_status");
    unlisten = await onToolProgress((p) => {
      if (p.id === "pdf-repair") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!files.length || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_pdf_repair", {
        opts: { inputs: files, mode, output_dir: outDir, suffix: "" },
      });
      const failed = result.items.filter((i) => !i.ok).length;
      showToast(failed ? "info" : "success", `${result.items.length - failed} ${$t("tools.common.done")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">Ghostscript</div>
          <div class="group-row-sub">{#if !status}…{:else if status.ghostscript}<span class="tag tag-success">{$t("tools.repair.gs_found")}</span> <span class="mono">{status.ghostscript}</span>{:else}{$t("tools.repair.gs_missing")}{/if}</div>
        </div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub mono">{files.map(baseName).slice(0, 5).join(", ")}{files.length > 5 ? "…" : ""}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(PDF_FILTER))]; }}>{$t("tools.common.add")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.repair.mode")}</div><div class="group-row-sub">{$t("tools.repair.note")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={mode}>
            <option value="auto">{$t("tools.repair.mode_auto")}</option>
            <option value="gs" disabled={!status?.ghostscript}>{$t("tools.repair.mode_gs")}</option>
            <option value="rebuild">{$t("tools.repair.mode_rebuild")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !files.length} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.repair.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section><div class="group">
      {#each result.items as item (item.input)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{baseName(item.input)}</div>
            {#if item.ok}
              <div class="group-row-sub">
                <span class="tag tag-success">{item.method === "ghostscript" ? $t("tools.repair.method_ghostscript") : $t("tools.repair.method_xref")}</span>
                {item.pages_after ?? 0} {$t("tools.repair.pages")}
                <span class="dim"> · {item.opened_before ? $t("tools.repair.opened_before") : $t("tools.repair.broken_before")}</span>
                {#if item.objects_found}<span class="dim"> · {item.objects_found} {$t("tools.repair.objects")}</span>{/if}
                <span class="dim"> · {fmtBytes(item.bytes_before)} → {fmtBytes(item.bytes_after)}</span>
              </div>
            {:else}
              <div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {item.error}</div>
            {/if}
          </div>
          {#if item.ok && item.output}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(item.output!)}>{$t("tools.common.reveal")}</button></div>{/if}
        </div>
      {/each}
    </div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
