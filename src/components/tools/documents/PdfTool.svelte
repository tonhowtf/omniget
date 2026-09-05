<script lang="ts">
  /**
   * Ferramentas de PDF (estudo 3, Stirling-PDF; 35, Dangerzone). Um componente
   * para seis tools: `mode` decide qual formulário aparece. Tudo roda no
   * PDFium gerido pelo app; Ghostscript, LibreOffice e Tesseract só entram
   * quando já existem na máquina.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, dirName, errText, fmtBytes, onToolProgress, pct, pickDir, pickFiles, reveal, saveAs, FILTERS, type ToolProgress } from "$lib/tools/rt";

  let { mode = "merge" }: { mode?: "merge" | "split" | "compress" | "convert" | "ocr" | "sanitize" } = $props();

  type Status = { pdfium: boolean; pdfium_version: string | null; ghostscript: string | null; libreoffice: string | null; tesseract: boolean; tesseract_langs: string[] };
  type Info = { path: string; pages: number; bytes: number; width_pt: number; height_pt: number; title: string | null; author: string | null; has_text: boolean };
  type PdfOut = { output: string; pages: number; bytes: number };
  type CompressResult = { output: string; before: number; after: number; method: string; pages: number };

  const PDF = [{ name: "PDF", extensions: ["pdf"] }];
  const OFFICE = [{ name: "Documents", extensions: ["pdf", "docx", "doc", "odt", "pptx", "ppt", "odp", "xlsx", "xls", "ods", "rtf", "txt", "html"] }];

  let status = $state<Status | null>(null);
  let files = $state<string[]>([]);
  let infos = $state<Record<string, Info>>({});
  let outDir = $state("");
  let busy = $state<string | null>(null);
  let progress = $state<ToolProgress | null>(null);
  let outputs = $state<string[]>([]);
  let summary = $state("");
  let text = $state("");

  // split
  let splitMode = $state<"each" | "every" | "ranges" | "extract">("extract");
  let every = $state(10);
  let ranges = $state("1-3");
  // compress
  let compressMode = $state<"auto" | "gs" | "raster">("auto");
  let preset = $state<"screen" | "ebook" | "printer">("ebook");
  let dpi = $state(110);
  let quality = $state(60);
  // convert
  let convertTo = $state<"png" | "jpg" | "txt" | "docx" | "pdf-from-images" | "pdf-from-office">("png");
  let pages = $state("");
  let renderDpi = $state(150);
  // ocr
  let langs = $state("por+eng");
  // sanitize
  let sanDpi = $state(150);

  let unlisten: (() => void) | null = null;
  onMount(async () => {
    await refresh();
    unlisten = await onToolProgress((p) => {
      if (p.id === "pdf") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function refresh() {
    try {
      status = await invoke<Status>("tool_pdf_status");
      if (status.tesseract_langs.length && !status.tesseract_langs.includes("por")) langs = status.tesseract_langs.includes("eng") ? "eng" : status.tesseract_langs[0];
    } catch (e) { showToast("error", errText(e)); }
  }

  async function installPdfium() {
    busy = "install";
    try { await invoke("install_dependency", { name: "PDFium" }); await refresh(); showToast("success", $t("tools.common.done") as string); }
    catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }

  async function addFiles() {
    const filters = mode === "convert" ? (convertTo === "pdf-from-images" ? FILTERS.images : convertTo === "pdf-from-office" ? OFFICE : PDF) : PDF;
    const picked = await pickFiles(filters);
    files = [...files, ...picked.filter((p) => !files.includes(p))];
    for (const p of picked) if (p.toLowerCase().endsWith(".pdf") && !infos[p]) {
      try { infos = { ...infos, [p]: await invoke<Info>("tool_pdf_info", { path: p }) }; } catch { /* sem PDFium ou arquivo ruim: a tool avisa ao rodar */ }
    }
  }
  function move(i: number, d: number) {
    const j = i + d; if (j < 0 || j >= files.length) return;
    const next = [...files]; [next[i], next[j]] = [next[j], next[i]]; files = next;
  }
  function remove(i: number) { files = files.filter((_, k) => k !== i); }
  let totalPages = $derived(files.reduce((n, f) => n + (infos[f]?.pages ?? 0), 0));
  let needsPdfium = $derived(!(mode === "convert" && (convertTo === "pdf-from-images" || convertTo === "docx" || convertTo === "pdf-from-office")));

  async function run() {
    if (!files.length || busy) return;
    busy = "run"; progress = null; outputs = []; summary = ""; text = "";
    try {
      if (mode === "merge") {
        const def = `${dirName(files[0])}/${baseName(files[0]).replace(/\.pdf$/i, "")} (${$t("tools.pdf.merged")}).pdf`;
        const out = await saveAs(def, PDF); if (!out) return;
        const r = await invoke<PdfOut>("tool_pdf_merge", { opts: { inputs: files, output: out } });
        outputs = [r.output]; summary = `${r.pages} ${$t("tools.pdf.pages")} · ${fmtBytes(r.bytes)}`;
      } else if (mode === "split") {
        const r = await invoke<{ outputs: string[]; pages: number }>("tool_pdf_split", { opts: { input: files[0], mode: splitMode, every, ranges, output_dir: outDir } });
        outputs = r.outputs; summary = `${r.outputs.length} ${$t("tools.common.files")}`;
      } else if (mode === "compress") {
        let before = 0, after = 0;
        for (const f of files) {
          const r = await invoke<CompressResult>("tool_pdf_compress", { opts: { input: f, output_dir: outDir, mode: compressMode, preset, dpi, quality } });
          outputs = [...outputs, r.output]; before += r.before; after += r.after; summary = `${fmtBytes(before)} → ${fmtBytes(after)} (${Math.round((1 - after / Math.max(1, before)) * 100)}%) · ${r.method}`;
        }
      } else if (mode === "convert") {
        if (convertTo === "png" || convertTo === "jpg") {
          for (const f of files) outputs = [...outputs, ...(await invoke<string[]>("tool_pdf_render", { opts: { input: f, pages, dpi: renderDpi, format: convertTo, quality: 90, output_dir: outDir } }))];
          summary = `${outputs.length} ${$t("tools.common.files")}`;
        } else if (convertTo === "txt") {
          for (const f of files) { const r = await invoke<{ text: string; output: string | null; pages: number }>("tool_pdf_text", { input: f, pages, save: true, outputDir: outDir }); if (r.output) outputs = [...outputs, r.output]; text = files.length === 1 ? r.text : ""; }
          summary = `${outputs.length} ${$t("tools.common.files")}`;
        } else if (convertTo === "pdf-from-images") {
          const def = `${dirName(files[0])}/${baseName(files[0]).replace(/\.[^.]+$/, "")}.pdf`;
          const out = await saveAs(def, PDF); if (!out) return;
          const r = await invoke<PdfOut>("tool_pdf_from_images", { inputs: files, output: out, quality: 90 });
          outputs = [r.output]; summary = `${r.pages} ${$t("tools.pdf.pages")} · ${fmtBytes(r.bytes)}`;
        } else {
          const target = convertTo === "docx" ? "docx" : "pdf";
          outputs = await invoke<string[]>("tool_pdf_office", { inputs: files, target, outputDir: outDir });
          summary = `${outputs.length} ${$t("tools.common.files")}`;
        }
      } else if (mode === "ocr") {
        for (const f of files) { const r = await invoke<PdfOut>("tool_pdf_ocr", { input: f, langs, outputDir: outDir, dpi: 300 }); outputs = [...outputs, r.output]; }
        summary = `${outputs.length} ${$t("tools.common.files")}`;
      } else if (mode === "sanitize") {
        for (const f of files) { const r = await invoke<PdfOut>("tool_pdf_sanitize", { input: f, outputDir: outDir, dpi: sanDpi, quality: 85 }); outputs = [...outputs, r.output]; }
        summary = `${outputs.length} ${$t("tools.common.files")}`;
      }
      if (outputs.length) showToast("success", $t("tools.common.done") as string);
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; progress = null; }
  }
  async function copyText() { await navigator.clipboard.writeText(text); showToast("success", $t("tools.common.copied") as string); }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">PDFium</div>
          <div class="group-row-sub">{#if !status}…{:else if status.pdfium}{status.pdfium_version ?? $t("tools.common.installed")}{:else}{$t("tools.pdf.pdfium_missing")}{/if}</div>
        </div>
        {#if status && !status.pdfium}
          <div class="group-row-trailing"><button class="btn btn-primary btn-sm" type="button" disabled={busy !== null} onclick={installPdfium}>{busy === "install" ? $t("tools.common.installing") : $t("tools.common.install")}</button></div>
        {/if}
      </div>
      {#if mode === "compress"}
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">Ghostscript <span class="dim">· {$t("tools.common.optional")}</span></div><div class="group-row-sub mono">{status?.ghostscript ?? $t("tools.pdf.gs_missing")}</div></div></div>
      {/if}
      {#if mode === "convert" && (convertTo === "docx" || convertTo === "pdf-from-office")}
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">LibreOffice</div><div class="group-row-sub mono">{status?.libreoffice ?? $t("tools.pdf.soffice_missing")}</div></div></div>
      {/if}
      {#if mode === "ocr"}
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">Tesseract</div><div class="group-row-sub">{#if status?.tesseract}{status.tesseract_langs.join(", ")}{:else}{$t("tools.common.not_installed")} · <span class="mono">{$t("tools.pdf.tesseract_hint")}</span>{/if}</div></div></div>
      {/if}
    </div>
  </section>

  <section>
    <div class="group">
      {#if mode === "convert"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.pdf.convert_to")}</div></div>
          <div class="group-row-trailing">
            <select class="input" bind:value={convertTo} onchange={() => { files = []; outputs = []; }}>
              <option value="png">PDF → PNG</option>
              <option value="jpg">PDF → JPG</option>
              <option value="txt">PDF → {$t("tools.pdf.text")}</option>
              <option value="docx">PDF → Word (docx)</option>
              <option value="pdf-from-images">{$t("tools.pdf.images")} → PDF</option>
              <option value="pdf-from-office">Word / PowerPoint / Excel → PDF</option>
            </select>
          </div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{files.length} {$t("tools.common.files")}{#if totalPages} <span class="dim">· {totalPages} {$t("tools.pdf.pages")}</span>{/if}</div>
          {#if !files.length}<div class="group-row-sub">{$t(`tools.pdf.hint_${mode}`)}</div>{/if}
        </div>
        <div class="group-row-trailing btn-row">
          {#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => { files = []; outputs = []; }}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={addFiles}>{$t("tools.common.add")}</button>
        </div>
      </div>
      {#each files as f, i (f)}
        <div class="group-row file-row">
          <div class="group-row-content">
            <div class="group-row-title mono">{baseName(f)}</div>
            {#if infos[f]}<div class="group-row-sub">{infos[f].pages} {$t("tools.pdf.pages")} · {fmtBytes(infos[f].bytes)}{#if infos[f].title} · {infos[f].title}{/if}{#if !infos[f].has_text} · <span class="tag tag-warning">{$t("tools.pdf.no_text")}</span>{/if}</div>{/if}
          </div>
          <div class="group-row-trailing btn-row">
            {#if mode === "merge" || convertTo === "pdf-from-images"}
              <button class="btn btn-ghost btn-sm" type="button" disabled={i === 0} onclick={() => move(i, -1)} aria-label="↑">↑</button>
              <button class="btn btn-ghost btn-sm" type="button" disabled={i === files.length - 1} onclick={() => move(i, 1)} aria-label="↓">↓</button>
            {/if}
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => remove(i)} aria-label="×">×</button>
          </div>
        </div>
      {/each}
    </div>
  </section>

  <section>
    <div class="group">
      {#if mode === "split"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.pdf.split_mode")}</div></div>
          <div class="group-row-trailing btn-row">
            <select class="input" bind:value={splitMode}>
              <option value="extract">{$t("tools.pdf.split_extract")}</option>
              <option value="ranges">{$t("tools.pdf.split_ranges")}</option>
              <option value="every">{$t("tools.pdf.split_every")}</option>
              <option value="each">{$t("tools.pdf.split_each")}</option>
            </select>
            {#if splitMode === "every"}<input class="input" type="number" min="1" bind:value={every} style:width="5em" />{/if}
            {#if splitMode === "extract" || splitMode === "ranges"}<input class="input mono" type="text" bind:value={ranges} placeholder={splitMode === "ranges" ? "1-3; 4-10; 11-" : "1-3, 5, 8-"} style:width="12em" />{/if}
          </div>
        </div>
        <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{$t("tools.pdf.split_hint")}</div></div></div>
      {:else if mode === "compress"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.pdf.method")}</div><div class="group-row-sub">{compressMode === "raster" ? $t("tools.pdf.raster_hint") : $t("tools.pdf.gs_hint")}</div></div>
          <div class="group-row-trailing btn-row">
            <select class="input" bind:value={compressMode}>
              <option value="auto">{$t("tools.pdf.auto")}</option>
              <option value="gs">Ghostscript</option>
              <option value="raster">{$t("tools.pdf.raster")}</option>
            </select>
            {#if compressMode !== "raster"}
              <select class="input" bind:value={preset}><option value="screen">screen · 72 dpi</option><option value="ebook">ebook · 150 dpi</option><option value="printer">printer · 300 dpi</option></select>
            {/if}
          </div>
        </div>
        {#if compressMode !== "gs"}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pdf.raster")} <span class="dim">· {dpi} dpi · {$t("tools.resize.quality")} {quality}</span></div></div>
            <div class="group-row-trailing btn-row"><input type="range" min="50" max="200" step="10" bind:value={dpi} /><input type="range" min="20" max="95" step="5" bind:value={quality} /></div>
          </div>
        {/if}
      {:else if mode === "convert" && (convertTo === "png" || convertTo === "jpg" || convertTo === "txt")}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.pdf.pages")}</div><div class="group-row-sub">{$t("tools.pdf.pages_hint")}</div></div>
          <div class="group-row-trailing btn-row">
            <input class="input mono" type="text" bind:value={pages} placeholder={$t("tools.pdf.all")} style:width="10em" />
            {#if convertTo !== "txt"}<input class="input" type="number" min="24" max="600" bind:value={renderDpi} style:width="6em" /><span class="dim">dpi</span>{/if}
          </div>
        </div>
      {:else if mode === "ocr"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ocr.languages")}</div><div class="group-row-sub">{$t("tools.ocr.languages_hint")}</div></div>
          <div class="group-row-trailing"><input class="input mono" type="text" bind:value={langs} style:width="10em" /></div>
        </div>
      {:else if mode === "sanitize"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.pdf.sanitize_title")} <span class="dim">· {sanDpi} dpi</span></div><div class="group-row-sub">{$t("tools.pdf.sanitize_hint")}</div></div>
          <div class="group-row-trailing"><input type="range" min="72" max="300" step="6" bind:value={sanDpi} /></div>
        </div>
      {/if}
      {#if mode !== "merge" && !(mode === "convert" && convertTo === "pdf-from-images")}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
          <div class="group-row-trailing btn-row">{#if outDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (outDir = "")}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content">
          {#if busy === "run"}
            <div class="group-row-sub">{progress?.message ?? "…"}{#if progress?.total} · {progress.done}/{progress.total}{/if}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy !== null || !files.length || (mode === "merge" && files.length < 2) || (needsPdfium && !status?.pdfium)} onclick={run}>{busy === "run" ? $t("tools.common.working") : $t(`tools.pdf.run_${mode}`)}</button></div>
      </div>
    </div>
  </section>

  {#if outputs.length}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.common.done")}</div><div class="group-row-sub">{summary}</div></div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(outputs[0])}>{$t("tools.common.reveal")}</button></div>
        </div>
        {#each outputs.slice(0, 12) as o (o)}
          <div class="group-row"><div class="group-row-content"><div class="group-row-sub mono">{o}</div></div></div>
        {/each}
        {#if outputs.length > 12}<div class="group-row"><div class="group-row-sub">… +{outputs.length - 12}</div></div>{/if}
        {#if text}
          <div class="group-row"><div class="group-row-content"><pre class="text">{text}</pre></div><div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={copyText}>{$t("tools.common.copy")}</button></div></div>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .file-row .group-row-title { font-weight: 500; }
  .text { margin: var(--space-1) 0 0; white-space: pre-wrap; max-height: 320px; overflow: auto; font-size: var(--text-sm); color: var(--text-muted); }
</style>
