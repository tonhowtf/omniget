<script lang="ts">
  import { onDestroy } from "svelte";
  import { ask, open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "$lib/pdf/api";
  import { formatBytes } from "$lib/download-format";
  import {
    A4,
    insertBlank,
    isUntouched,
    joinPath,
    maybeSamePath,
    pagesOf,
    parentDir,
    pdfName,
    plural,
    rangeLabel,
    rotateItems,
    removeItems,
    samePath,
    sliceRange,
    sourcesOf,
    stem,
    toSpecs,
    type PageItem,
    type Range,
  } from "$lib/pdf/pages";
  import { Thumbs } from "$lib/pdf/thumbs";
  import type { OutputPlan, PdfInfo, SavedFile } from "$lib/pdf/types";
  import StatusIcon from "$components/converter/StatusIcon.svelte";
  import PageGrid from "./PageGrid.svelte";
  import SplitPanel from "./SplitPanel.svelte";

  let { active = true, onbusy }: { active?: boolean; onbusy?: (busy: boolean) => void } = $props();

  const HISTORY = 50;

  let files = $state<PdfInfo[]>([]);
  let tones = $state(new Map<string, number>());
  let items = $state<PageItem[]>([]);
  let selected = $state(new Set<string>());
  let history = $state<PageItem[][]>([]);
  let adding = $state(false);
  let saving = $state(false);
  let splitting = $state(false);
  let notice = $state<string | null>(null);
  let result = $state<{ files: SavedFile[]; title: string } | null>(null);
  const thumbs = new Thumbs();
  let toneSeq = 0;

  let readable = $derived(files.filter((f) => !f.error));
  let untouched = $derived(readable.length === 1 && isUntouched(items, readable[0].path, readable[0].pages));
  let busy = $derived(adding || saving);
  let selectedCount = $derived(items.filter((i) => selected.has(i.key)).length);

  $effect(() => onbusy?.(busy));

  onDestroy(() => thumbs.clear());

  function errorText(code: string | null): string {
    return $t(`pdf.error_${code ?? "io"}`) as string;
  }

  function commit(next: PageItem[]) {
    history = [...history.slice(-(HISTORY - 1)), items];
    items = next;
    result = null;
  }

  function undo() {
    const prev = history[history.length - 1];
    if (!prev || busy) return;
    history = history.slice(0, -1);
    items = prev;
    const keys = new Set(prev.map((i) => i.key));
    selected = new Set([...selected].filter((k) => keys.has(k)));
    result = null;
  }

  export async function add(paths: string[]) {
    const fresh = paths.filter((p) => /\.pdf$/i.test(p) && !files.some((f) => samePath(f.path, p)));
    if (fresh.length === 0 || busy) return;
    adding = true;
    notice = null;
    try {
      const infos = await api.inspect(fresh);
      const next = [...items];
      for (const info of infos) {
        if (!info.error) {
          tones.set(info.path, toneSeq++);
          next.push(...pagesOf(info.path, info.pages));
        }
      }
      tones = new Map(tones);
      files = [...files, ...infos];
      if (next.length !== items.length) commit(next);
    } catch (e) {
      notice = errorText(api.failureCode(e));
    } finally {
      adding = false;
    }
  }

  async function chooseFiles() {
    const picked = await openDialog({ multiple: true, directory: false, filters: [{ name: "PDF", extensions: ["pdf"] }] }).catch(() => null);
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    await add(paths);
  }

  function removeFile(file: PdfInfo) {
    if (busy) return;
    files = files.filter((f) => f !== file);
    if (!file.error) {
      const keep = (list: PageItem[]) => list.filter((i) => i.kind !== "page" || i.path !== file.path);
      history = history.map(keep);
      items = keep(items);
      result = null;
      const keys = new Set(items.map((i) => i.key));
      selected = new Set([...selected].filter((k) => keys.has(k)));
      thumbs.release(file.path);
      tones.delete(file.path);
      tones = new Map(tones);
    }
  }

  function clearAll() {
    if (busy) return;
    thumbs.clear();
    files = [];
    items = [];
    history = [];
    selected = new Set();
    tones = new Map();
    result = null;
    notice = null;
    splitting = false;
  }

  function selectedItems(): PageItem[] {
    return items.filter((i) => selected.has(i.key));
  }

  function rotateSelected(delta: number) {
    if (selected.size) commit(rotateItems(items, selected, delta));
  }

  function deleteSelected() {
    if (!selected.size) return;
    commit(removeItems(items, selected));
    selected = new Set();
  }

  function addBlank() {
    const last = items.reduce((at, item, i) => (selected.has(item.key) ? i : at), -1);
    const at = last >= 0 ? last + 1 : items.length;
    const near = items[at - 1] ?? items[at];
    const size = near?.kind === "page" ? thumbs.size(near.path, near.page) : near?.kind === "blank" ? { width: near.width, height: near.height } : null;
    const next = insertBlank(items, at, size ?? A4);
    commit(next);
    selected = new Set([next[at].key]);
  }

  function firstSource(list: PageItem[]): string {
    return sourcesOf(list)[0] ?? readable[0]?.path ?? "";
  }

  function plan(list: PageItem[], name: string, dir?: string): { sources: string[]; output: OutputPlan } {
    const sources = sourcesOf(list);
    const folder = dir ?? parentDir(firstSource(list));
    return { sources, output: { path: joinPath(folder, pdfName(name)), pages: toSpecs(list, sources) } };
  }

  function mainName(): string {
    const name = stem(firstSource(items));
    return (readable.length > 1 ? $t("pdf.name_merged", { name }) : $t("pdf.name_edited", { name })) as string;
  }

  async function run(title: string, sources: string[], outputs: OutputPlan[], overwrite: boolean) {
    saving = true;
    notice = null;
    result = null;
    try {
      const saved = await api.save(sources, outputs, overwrite);
      result = { files: saved, title };
    } catch (e) {
      notice = errorText(api.failureCode(e));
    } finally {
      saving = false;
    }
  }

  async function saveAll() {
    if (items.length === 0) return;
    const { sources, output } = plan(items, mainName());
    await run($t("pdf.saved_one") as string, sources, [output], false);
  }

  async function saveAs() {
    if (items.length === 0) return;
    const { sources, output } = plan(items, mainName());
    const picked = await saveDialog({ defaultPath: output.path, filters: [{ name: "PDF", extensions: ["pdf"] }] }).catch(() => null);
    if (!picked) return;
    const typed = /\.pdf$/i.test(picked);
    const path = typed ? picked : `${picked}.pdf`;
    const original = files.some((f) => maybeSamePath(f.path, path));
    if (original) {
      const yes = await ask($t("pdf.replace_original_body") as string, { title: $t("pdf.replace_original_title") as string, kind: "warning" }).catch(() => false);
      if (!yes) return;
    }
    await run($t("pdf.saved_one") as string, sources, [{ ...output, path }], typed || original);
  }

  async function extractSelected() {
    const list = selectedItems();
    if (list.length === 0) return;
    const positions = items.flatMap((item, i) => (selected.has(item.key) ? [i + 1] : []));
    const contiguous = positions.every((p, i) => i === 0 || p === positions[i - 1] + 1);
    const name = stem(firstSource(list));
    const fileName = contiguous
      ? ($t("pdf.name_pages", { name, range: rangeLabel([positions[0], positions[positions.length - 1]]) }) as string)
      : ($t("pdf.name_selection", { name }) as string);
    const { sources, output } = plan(list, fileName, parentDir(firstSource(items)));
    await run($t("pdf.saved_one") as string, sources, [output], false);
  }

  async function split(ranges: Range[]) {
    const name = stem(firstSource(items));
    const dir = parentDir(firstSource(items));
    const sources = sourcesOf(items);
    const outputs = ranges.map((range) => ({
      path: joinPath(dir, pdfName($t("pdf.name_pages", { name, range: rangeLabel(range) }) as string)),
      pages: toSpecs(sliceRange(items, range), sources),
    }));
    await run(plural($t, "pdf.saved_many", outputs.length), sources, outputs, false);
    if (result) splitting = false;
  }

  function onKeydown(event: KeyboardEvent) {
    if (active && (event.metaKey || event.ctrlKey) && !event.shiftKey && event.key.toLowerCase() === "z") {
      const target = event.target as HTMLElement | null;
      if (target?.closest("input, textarea, [contenteditable]")) return;
      event.preventDefault();
      undo();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if files.length === 0}
  <div class="pv-drop">
    <svg class="pv-drop-art" viewBox="0 0 48 48" width="48" height="48" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M14 8h14l8 8v24a2 2 0 0 1-2 2H14a2 2 0 0 1-2-2V10a2 2 0 0 1 2-2z" />
      <path d="M28 8v8h8" />
      <path d="M18 26h12M18 31h12M18 36h7" />
    </svg>
    <h2 class="pv-drop-title">{$t("pdf.organize_empty_title")}</h2>
    <p class="pv-drop-body">{$t("pdf.organize_empty_body")}</p>
    <button type="button" class="btn btn-primary" disabled={adding} onclick={chooseFiles}>
      {#if adding}<span class="spinner pv-spin" aria-hidden="true"></span>{/if}
      {$t("pdf.choose_pdfs")}
    </button>
  </div>
{:else}
  <ul class="pv-files" aria-label={$t("pdf.files_label") as string}>
    {#each files as file (file.path)}
      <li class="pv-file" class:bad={!!file.error}>
        {#if file.error}
          <StatusIcon name="warning" size={14} />
        {:else if tones.size > 1}
          <span class="pv-tone" style:--tone="var(--pdf-tone-{(tones.get(file.path) ?? 0) % 6})" aria-hidden="true"></span>
        {/if}
        <span class="pv-file-name" title={file.path}>{file.name}</span>
        <span class="pv-file-meta">
          {file.error ? errorText(file.error) : `${plural($t, "pdf.page_count", file.pages)} · ${formatBytes(file.size_bytes)}`}
        </span>
        <button type="button" class="pv-file-x" disabled={busy} aria-label={$t("pdf.remove_file", { name: file.name }) as string} onclick={() => removeFile(file)}>
          <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="m4 4 8 8M12 4l-8 8" /></svg>
        </button>
      </li>
    {/each}
  </ul>

  {#if notice}
    <div class="pv-notice" role="alert">
      <StatusIcon name="warning" />
      <p>{notice}</p>
    </div>
  {/if}

  {#if result}
    <div class="pv-result" role="status">
      <StatusIcon name="done" size={20} />
      <div class="pv-result-copy">
        <h2>{result.title}</h2>
        {#each result.files.slice(0, 4) as file (file.path)}
          <p title={file.path}>{file.path.split(/[\\/]/).pop()} · {formatBytes(file.size_bytes)}</p>
        {/each}
        {#if result.files.length > 4}
          <p>{plural($t, "pdf.saved_more", result.files.length - 4)}</p>
        {/if}
      </div>
      <div class="pv-result-actions">
        {#if result.files.length === 1}
          <button type="button" class="btn btn-secondary btn-sm" onclick={() => result && void api.openFile(result.files[0].path).catch(() => {})}>{$t("pdf.open")}</button>
        {/if}
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => result && void api.reveal(result.files[0].path).catch(() => {})}>{$t("pdf.show_in_folder")}</button>
      </div>
    </div>
  {/if}

  {#if items.length > 0}
    <div class="pv-tools" role="toolbar" aria-label={$t("pdf.tools_label") as string}>
      <span class="pv-count">
        {selectedCount > 0 ? plural($t, "pdf.selected_count", selectedCount) : plural($t, "pdf.page_count", items.length)}
      </span>
      <div class="pv-tools-group">
        <button type="button" class="btn btn-ghost btn-sm" disabled={busy || selectedCount === 0} onclick={() => rotateSelected(-90)}>{$t("pdf.rotate_left")}</button>
        <button type="button" class="btn btn-ghost btn-sm" disabled={busy || selectedCount === 0} onclick={() => rotateSelected(90)}>{$t("pdf.rotate_right")}</button>
        <button type="button" class="btn btn-ghost btn-sm" disabled={busy || selectedCount === 0} onclick={deleteSelected}>{$t("pdf.delete_selected")}</button>
        <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={addBlank}>{$t("pdf.insert_blank")}</button>
        <button type="button" class="btn btn-ghost btn-sm" disabled={busy || history.length === 0} onclick={undo}>{$t("pdf.undo")}</button>
      </div>
      <div class="pv-tools-group">
        <button type="button" class="btn btn-secondary btn-sm" disabled={busy || selectedCount === 0} onclick={extractSelected}>{$t("pdf.extract_selected")}</button>
        <button type="button" class="btn btn-secondary btn-sm" disabled={busy || items.length < 2} aria-expanded={splitting} onclick={() => (splitting = !splitting)}>{$t("pdf.split")}</button>
      </div>
    </div>
    <p class="pv-hint">{$t("pdf.organize_hint")}</p>

    {#if splitting}
      <SplitPanel total={items.length} busy={saving} onsplit={split} oncancel={() => (splitting = false)} />
    {/if}

    <PageGrid {items} bind:selected {thumbs} {tones} disabled={busy} onchange={commit} />
  {:else if readable.length > 0}
    <p class="pv-hint">{$t("pdf.all_pages_removed")}</p>
  {/if}

  <footer class="pv-bar">
    <div class="pv-bar-side">
      <button type="button" class="btn btn-secondary" disabled={busy} onclick={chooseFiles}>
        {#if adding}<span class="spinner pv-spin" aria-hidden="true"></span>{/if}
        {$t("pdf.add_pdfs")}
      </button>
      <button type="button" class="btn btn-ghost" disabled={busy} onclick={clearAll}>{$t("pdf.clear")}</button>
    </div>
    <div class="pv-bar-main">
      {#if untouched}
        <span class="pv-bar-note">{$t("pdf.nothing_changed")}</span>
      {/if}
      <button type="button" class="btn btn-secondary" disabled={busy || items.length === 0 || untouched} onclick={saveAs}>{$t("pdf.save_as")}</button>
      <button type="button" class="btn btn-primary" disabled={busy || items.length === 0 || untouched} onclick={saveAll}>
        {#if saving}<span class="spinner pv-spin" aria-hidden="true"></span>{/if}
        {readable.length > 1 ? $t("pdf.save_merged") : $t("pdf.save")}
      </button>
    </div>
  </footer>
{/if}
