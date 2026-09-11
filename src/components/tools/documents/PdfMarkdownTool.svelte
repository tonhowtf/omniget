<script lang="ts">
  /** PDF → Markdown limpo (títulos, listas, tabelas) para jogar num LLM. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, openPath, pct, pickFile, reveal, saveAs, type ToolProgress } from "$lib/tools/rt";

  type PageReport = { page: number; chars: number; columns: number; headings: number; tables: number; needs_ocr: boolean };
  type Result = {
    markdown: string;
    output: string | null;
    images_dir: string | null;
    images: number;
    pages: PageReport[];
    needs_ocr: number;
    headings: number;
    tables: number;
    words: number;
    bytes: number;
  };

  const PDF_FILTER = [{ name: "PDF", extensions: ["pdf"] }];
  const MD_FILTER = [{ name: "Markdown", extensions: ["md"] }];

  let input = $state("");
  let pages = $state("");
  let output = $state("");
  let marks = $state("comment");
  let tables = $state(true);
  let code = $state(true);
  let keepRunning = $state(false);
  let images = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const preview = $derived(result ? result.markdown.slice(0, 4000) : "");
  const twoColumn = $derived(result ? result.pages.some((p) => p.columns > 1) : false);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "pdf-markdown") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!input || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_pdf_markdown", {
        opts: {
          input,
          pages,
          output,
          save: true,
          page_marks: marks,
          extract_images: images,
          keep_running: keepRunning,
          tables,
          code,
        },
      });
      showToast(result.needs_ocr ? "info" : "success", $t("tools.common.done") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function copy() {
    if (!result) return;
    await navigator.clipboard.writeText(result.markdown);
    showToast("success", $t("tools.common.copied") as string);
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{input ? baseName(input) : $t("tools.pdfmd.file")}</div>
          <div class="group-row-sub">{$t("tools.pdfmd.file_sub")}</div>
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(PDF_FILTER); if (f) input = f; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.pdfmd.pages")}</div>
          <div class="group-row-sub">{$t("tools.pdfmd.pages_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input mono" type="text" placeholder="1-5, 8" bind:value={pages} style:width="9em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.pdfmd.marks")}</div>
          <div class="group-row-sub">{$t("tools.pdfmd.marks_sub")}</div>
        </div>
        <div class="group-row-trailing">
          <select class="input" bind:value={marks}>
            <option value="comment">{$t("tools.pdfmd.marks_comment")}</option>
            <option value="rule">{$t("tools.pdfmd.marks_rule")}</option>
            <option value="none">{$t("tools.pdfmd.marks_none")}</option>
          </select>
        </div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfmd.tables")}</div><div class="group-row-sub">{$t("tools.pdfmd.tables_sub")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={tables} type="button" role="switch" aria-checked={tables} aria-label={$t("tools.pdfmd.tables") as string} onclick={() => (tables = !tables)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfmd.code")}</div><div class="group-row-sub">{$t("tools.pdfmd.code_sub")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={code} type="button" role="switch" aria-checked={code} aria-label={$t("tools.pdfmd.code") as string} onclick={() => (code = !code)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfmd.running")}</div><div class="group-row-sub">{$t("tools.pdfmd.running_sub")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={keepRunning} type="button" role="switch" aria-checked={keepRunning} aria-label={$t("tools.pdfmd.running") as string} onclick={() => (keepRunning = !keepRunning)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfmd.images")}</div><div class="group-row-sub">{$t("tools.pdfmd.images_sub")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={images} type="button" role="switch" aria-checked={images} aria-label={$t("tools.pdfmd.images") as string} onclick={() => (images = !images)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfmd.output")}</div><div class="group-row-sub mono">{output || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing btn-row">
          {#if output}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (output = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await saveAs(input ? baseName(input).replace(/\.pdf$/i, ".md") : "documento.md", MD_FILTER); if (f) output = f; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !input} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.pdfmd.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.pages.length} {$t("tools.pdfmd.pages_done")}</div>
            <div class="group-row-sub">
              {result.words} {$t("tools.pdfmd.words")} · {result.headings} {$t("tools.pdfmd.headings")} · {result.tables} {$t("tools.pdfmd.tables_found")}
              {#if result.images}· {result.images} {$t("tools.pdfmd.images_found")}{/if}
              <span class="dim"> · {fmtBytes(result.bytes)}</span>
              {#if twoColumn}<span class="tag">{$t("tools.pdfmd.two_columns")}</span>{/if}
            </div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-ghost btn-sm" type="button" onclick={copy}>{$t("tools.common.copy")}</button>
            {#if result.output}
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => openPath(result!.output!)}>{$t("tools.common.open")}</button>
              <button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.output!)}>{$t("tools.common.reveal")}</button>
            {/if}
          </div>
        </div>
        {#if result.needs_ocr}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-sub"><span class="tag tag-danger">{$t("tools.pdfmd.needs_ocr")}</span> {result.needs_ocr} · {$t("tools.pdfmd.needs_ocr_sub")}</div>
            </div>
          </div>
        {/if}
        {#if result.images_dir}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfmd.images_dir")}</div><div class="group-row-sub mono">{result.images_dir}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result!.images_dir!)}>{$t("tools.common.reveal")}</button></div>
          </div>
        {/if}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.pdfmd.preview")}</div>
            <pre class="text">{preview}</pre>
          </div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .text { font-family: var(--font-mono); font-size: var(--text-xs); white-space: pre-wrap; overflow-wrap: anywhere; max-height: 22rem; overflow-y: auto; margin: var(--space-2) 0 0; }
</style>
