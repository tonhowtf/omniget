<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t, locale } from "$lib/i18n";
  import * as api from "$lib/photos/api";
  import { filterPhotos, formatBytes, groupPhotos, rangeSelect, type GroupBy } from "$lib/photos/plan";
  import type { ExportOptions, ExportProgress, FileReport, Folders, IndexProgress, Photo, RemoveMode } from "$lib/photos/types";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import PhotoTile from "#components/photos/PhotoTile.svelte";
  import Editor from "#components/photos/Editor.svelte";
  import ExportDialog from "#components/photos/ExportDialog.svelte";
  import RemoveDialog from "#components/photos/RemoveDialog.svelte";
  import FoldersDialog from "#components/photos/FoldersDialog.svelte";
  import Duplicates from "#components/photos/Duplicates.svelte";

  type Tab = "library" | "duplicates";
  type ExportRun = { batchId: string; total: number; done: number; failed: number; output: string | null; finished: boolean; stopping: boolean };

  const RELOAD_EVERY = 1500;

  let tab = $state<Tab>("library");
  let photos = $state<Photo[]>([]);
  let folders = $state<Folders | null>(null);
  let progress = $state<IndexProgress | null>(null);
  let loaded = $state(false);
  let groupBy = $state<GroupBy>("month");
  let query = $state("");
  let selected = $state(new Set<string>());
  let anchor: string | null = null;
  let editor = $state<{ path: string; order: string[] } | null>(null);
  let exportFor = $state<string[] | null>(null);
  let exportOptions = $state<ExportOptions>({ format: "jpeg", quality: 88, max_size: null, out_dir: null, apply_edits: true });
  let run = $state<ExportRun | null>(null);
  let removeOpen = $state(false);
  let foldersOpen = $state(false);
  let busy = $state(false);
  let notice = $state<string | null>(null);
  let lastReload = 0;
  let reloadTimer: ReturnType<typeof setTimeout> | null = null;

  let byPath = $derived(new Map(photos.map((p) => [p.path, p])));
  let roots = $derived(folders?.folders.map((f) => f.path) ?? []);
  let shown = $derived(filterPhotos(photos, query));
  let sections = $derived(groupPhotos(shown, groupBy, $locale || "en", roots));
  let order = $derived(sections.flatMap((s) => s.photos.map((p) => p.path)));
  let selectedList = $derived(order.filter((p) => selected.has(p)));
  let selectedBytes = $derived(selectedList.reduce((n, p) => n + (byPath.get(p)?.size ?? 0), 0));
  let indexing = $derived(progress?.stage === "scanning" || progress?.stage === "indexing");
  let failedCount = $derived(photos.filter((p) => p.error).length);
  let editorIndex = $derived(editor ? editor.order.indexOf(editor.path) : -1);
  let editorPhoto = $derived(editor ? (byPath.get(editor.path) ?? null) : null);

  let statusLine = $derived.by(() => {
    if (!progress) return "";
    if (progress.stage === "scanning") return $t("photos.index.scanning", { count: progress.found }) as string;
    if (progress.stage === "indexing") return $t("photos.index.thumbs", { done: progress.done, total: progress.total }) as string;
    if (progress.stage === "failed") return $t("photos.index.failed", { error: progress.error ?? "" }) as string;
    if (progress.stage === "cancelled") return $t("photos.index.cancelled") as string;
    return "";
  });

  async function reload() {
    lastReload = Date.now();
    try {
      photos = await api.list();
      const known = new Set(photos.map((p) => p.path));
      if ([...selected].some((p) => !known.has(p))) selected = new Set([...selected].filter((p) => known.has(p)));
    } catch (e) {
      notice = api.errorText(e);
    } finally {
      loaded = true;
    }
  }

  function reloadSoon() {
    if (reloadTimer) return;
    const wait = Math.max(0, RELOAD_EVERY - (Date.now() - lastReload));
    reloadTimer = setTimeout(() => {
      reloadTimer = null;
      void reload();
    }, wait);
  }

  function onIndex(p: IndexProgress) {
    const before = progress;
    progress = p;
    if (p.stage === "indexing" && p.done !== before?.done) reloadSoon();
    if (p.stage === "done" || p.stage === "cancelled" || p.stage === "failed") void reload();
  }

  async function startIndex() {
    try {
      progress = await api.indexStart();
    } catch (e) {
      notice = api.errorText(e);
    }
  }

  async function saveFolders(paths: string[]) {
    foldersOpen = false;
    try {
      folders = await api.setFolders(paths);
      await startIndex();
    } catch (e) {
      notice = api.errorText(e);
    }
  }

  function clickTile(e: MouseEvent, path: string) {
    if (e.shiftKey) {
      selected = new Set([...selected, ...rangeSelect(order, anchor, path)]);
      return;
    }
    if (e.metaKey || e.ctrlKey || selected.size > 0) {
      toggle(path);
      return;
    }
    editor = { path, order };
  }

  function toggle(path: string) {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    selected = next;
    anchor = path;
  }

  function checkTile(e: MouseEvent, path: string) {
    if (e.shiftKey) selected = new Set([...selected, ...rangeSelect(order, anchor, path)]);
    else toggle(path);
  }

  function selectSection(paths: string[]) {
    const all = paths.every((p) => selected.has(p));
    const next = new Set(selected);
    for (const p of paths) {
      if (all) next.delete(p);
      else next.add(p);
    }
    selected = next;
  }

  function step(delta: number) {
    if (!editor) return;
    const next = editor.order[editorIndex + delta];
    if (next) editor = { ...editor, path: next };
  }

  function markEdited(path: string, edited: boolean) {
    const p = photos.find((x) => x.path === path);
    if (p) p.edited = edited;
  }

  function applyReport(report: FileReport) {
    const gone = new Set(report.done);
    if (report.moved_to.length) {
      void reload();
    } else {
      photos = photos.filter((p) => !gone.has(p.path));
    }
    selected = new Set([...selected].filter((p) => !gone.has(p)));
    if (report.failed.length) notice = $t("photos.remove.some_failed", { count: report.failed.length }) as string;
  }

  async function removeSelected(mode: RemoveMode) {
    removeOpen = false;
    busy = true;
    try {
      applyReport(await api.remove(selectedList, mode));
    } catch (e) {
      notice = api.errorText(e);
    } finally {
      busy = false;
    }
  }

  async function moveSelected() {
    const dest = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof dest !== "string") return;
    busy = true;
    try {
      const report = await api.move(selectedList, dest);
      applyReport(report);
      if (report.done.length) notice = $t("photos.move.done", { count: report.done.length }) as string;
    } catch (e) {
      notice = api.errorText(e);
    } finally {
      busy = false;
    }
  }

  function onExportProgress(e: ExportProgress) {
    if (!run || e.batch_id !== run.batchId) return;
    if (e.state === "done") {
      run.done += 1;
      run.output ??= e.outcome?.output ?? null;
    } else if (e.state === "failed") {
      run.failed += 1;
    }
  }

  async function startExport(paths: string[], options: ExportOptions) {
    exportFor = null;
    exportOptions = options;
    const batchId = `photos-${Date.now()}`;
    run = { batchId, total: paths.length, done: 0, failed: 0, output: null, finished: false, stopping: false };
    try {
      const outcomes = await api.exportPhotos(batchId, paths, options);
      if (run?.batchId === batchId) {
        run.done = outcomes.filter((o) => o.ok).length;
        run.failed = outcomes.length - run.done;
        run.output = outcomes.find((o) => o.ok && o.output)?.output ?? null;
        run.finished = true;
        const firstError = outcomes.find((o) => !o.ok)?.error;
        if (firstError && run.done === 0 && !run.stopping) notice = firstError;
      }
    } catch (e) {
      notice = api.errorText(e);
      run = null;
    }
  }

  function stopExport() {
    if (!run) return;
    run.stopping = true;
    void api.exportCancel(run.batchId).catch(() => {});
  }

  onMount(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];
    const keep = (fn: () => void) => {
      if (disposed) fn();
      else cleanups.push(fn);
    };
    void api.onIndex(onIndex).then(keep).catch(() => {});
    void api.onExport(onExportProgress).then(keep).catch(() => {});
    void (async () => {
      try {
        folders = await api.folders();
        progress = await api.indexStatus();
      } catch (e) {
        notice = api.errorText(e);
      }
      await reload();
      if (!disposed) await startIndex();
    })();
    return () => {
      disposed = true;
      if (reloadTimer) clearTimeout(reloadTimer);
      cleanups.forEach((fn) => fn());
    };
  });
</script>

<section class="ph" aria-busy={indexing}>
  <header class="ph-head">
    <NavIcon icon="photos" size={56} />
    <div class="ph-head-copy">
      <h1>{$t("photos.title")}</h1>
      <p class="ph-subtitle">{$t("photos.subtitle")}</p>
    </div>
  </header>

  <div class="ph-tabs">
    <div class="segmented" role="tablist" aria-label={$t("photos.title") as string}>
      <button type="button" class="segmented-btn" role="tab" aria-selected={tab === "library"} onclick={() => (tab = "library")}>{$t("photos.tab_library")}</button>
      <button type="button" class="segmented-btn" role="tab" aria-selected={tab === "duplicates"} onclick={() => (tab = "duplicates")}>{$t("photos.tab_duplicates")}</button>
    </div>
    <span class="ph-grow"></span>
    <button type="button" class="btn btn-secondary btn-sm" onclick={() => (foldersOpen = true)} disabled={!folders}>{$t("photos.folders.button")}</button>
    <button type="button" class="btn btn-ghost btn-sm" disabled={indexing} onclick={() => void startIndex()}>{$t("photos.index.refresh")}</button>
  </div>

  {#if indexing || statusLine}
    <div class="ph-status" role="status" aria-live="polite">
      {#if indexing}<span class="spinner ph-spin" aria-hidden="true"></span>{/if}
      <span>{statusLine}</span>
      {#if progress?.stage === "indexing" && progress.total > 0}
        <div class="progress ph-progress"><div class="progress-fill" style:width="{(progress.done / progress.total) * 100}%"></div></div>
      {/if}
      {#if indexing}
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => void api.indexCancel()}>{$t("photos.index.stop")}</button>
      {/if}
    </div>
  {/if}

  {#if notice}
    <div class="notice notice-warning ph-notice" role="alert">
      <p class="notice-text">{notice}</p>
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => (notice = null)}>{$t("photos.dismiss")}</button>
    </div>
  {/if}

  {#if run}
    <div class="notice ph-notice" class:notice-success={run.finished && run.failed === 0} role="status">
      <p class="notice-text">
        {#if !run.finished}
          {$t("photos.export.progress", { done: run.done + run.failed, total: run.total })}
        {:else if run.failed === 0}
          {run.done === 1 ? $t("photos.export.done_one") : $t("photos.export.done_many", { count: run.done })}
        {:else}
          {$t("photos.export.done_partial", { done: run.done, total: run.total })}
        {/if}
      </p>
      {#if !run.finished}
        <button type="button" class="btn btn-ghost btn-sm" disabled={run.stopping} onclick={stopExport}>{run.stopping ? $t("photos.export.stopping") : $t("photos.export.stop")}</button>
      {:else}
        {#if run.output}
          {@const out = run.output}
          <button type="button" class="btn btn-secondary btn-sm" onclick={() => void api.reveal(out).catch(() => {})}>{$t("photos.show_in_folder")}</button>
        {/if}
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => (run = null)}>{$t("photos.dismiss")}</button>
      {/if}
    </div>
  {/if}

  {#if folders && folders.folders.length === 0}
    <div class="ph-empty">
      <h2>{$t("photos.empty_folders_title")}</h2>
      <p>{$t("photos.empty_folders_body")}</p>
      <button type="button" class="btn btn-primary" onclick={() => (foldersOpen = true)}>{$t("photos.folders.add")}</button>
    </div>
  {:else if tab === "duplicates"}
    {#if loaded}
      <Duplicates photos={byPath} {indexing} onopen={(path, o) => (editor = { path, order: o })} onremoved={applyReport} />
    {/if}
  {:else}
    <div class="ph-tools">
      <input class="input input-search ph-search" type="search" placeholder={$t("photos.search") as string} aria-label={$t("photos.search") as string} bind:value={query} />
      <div class="segmented" role="radiogroup" aria-label={$t("photos.group_by") as string}>
        <button type="button" class="segmented-btn" role="radio" aria-checked={groupBy === "month"} onclick={() => (groupBy = "month")}>{$t("photos.group_month")}</button>
        <button type="button" class="segmented-btn" role="radio" aria-checked={groupBy === "day"} onclick={() => (groupBy = "day")}>{$t("photos.group_day")}</button>
        <button type="button" class="segmented-btn" role="radio" aria-checked={groupBy === "folder"} onclick={() => (groupBy = "folder")}>{$t("photos.group_folder")}</button>
      </div>
      <span class="ph-count">
        {$t("photos.count", { count: photos.length })}{#if failedCount > 0}&nbsp;· {$t("photos.count_failed", { count: failedCount })}{/if}
      </span>
    </div>

    {#if !loaded || (photos.length === 0 && indexing)}
      <div class="ph-empty"><span class="spinner" aria-hidden="true"></span><p>{$t("photos.looking")}</p></div>
    {:else if photos.length === 0}
      <div class="ph-empty">
        <h2>{$t("photos.empty_title")}</h2>
        <p>{$t("photos.empty_body")}</p>
        <button type="button" class="btn btn-secondary" onclick={() => (foldersOpen = true)}>{$t("photos.folders.button")}</button>
      </div>
    {:else if shown.length === 0}
      <div class="ph-empty"><p>{$t("photos.no_match", { query })}</p></div>
    {:else}
      {#each sections as section (section.key)}
        {@const paths = section.photos.map((p) => p.path)}
        <section class="ph-section">
          <header class="ph-section-head">
            <h2 title={section.key}>{section.title}</h2>
            <span>{section.photos.length}</span>
            <button type="button" class="btn btn-ghost btn-sm ph-select-all" onclick={() => selectSection(paths)}>
              {paths.every((p) => selected.has(p)) ? $t("photos.unselect_all") : $t("photos.select_all")}
            </button>
          </header>
          <div class="ph-grid">
            {#each section.photos as p (p.path)}
              <PhotoTile photo={p} selected={selected.has(p.path)} checkLabel={$t("photos.select") as string} onopen={(e) => clickTile(e, p.path)} ontoggle={(e) => checkTile(e, p.path)} />
            {/each}
          </div>
        </section>
      {/each}
    {/if}

    {#if selected.size > 0}
      <footer class="ph-bar">
        <span class="ph-bar-count">{$t("photos.selected", { count: selectedList.length, size: formatBytes(selectedBytes) })}</span>
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => (selected = new Set())}>{$t("photos.clear_selection")}</button>
        <span class="ph-grow"></span>
        <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => void moveSelected()}>{$t("photos.move.action")}</button>
        <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => (removeOpen = true)}>{$t("photos.remove.action")}</button>
        <button type="button" class="btn btn-primary btn-sm" disabled={busy || (run !== null && !run.finished)} onclick={() => (exportFor = selectedList)}>{$t("photos.export.action")}</button>
      </footer>
    {/if}
  {/if}
</section>

{#if editor && editorPhoto}
  <Editor
    photo={editorPhoto}
    hasPrev={editorIndex > 0}
    hasNext={editorIndex >= 0 && editorIndex < editor.order.length - 1}
    onprev={() => step(-1)}
    onnext={() => step(1)}
    onclose={() => (editor = null)}
    onsaved={markEdited}
    onexport={(path) => (exportFor = [path])}
  />
{/if}

{#if exportFor}
  {@const paths = exportFor}
  <ExportDialog
    count={paths.length}
    anyEdited={paths.some((p) => byPath.get(p)?.edited)}
    initial={exportOptions}
    onclose={() => (exportFor = null)}
    onexport={(options) => void startExport(paths, options)}
  />
{/if}

{#if removeOpen}
  <RemoveDialog count={selectedList.length} bytes={selectedBytes} onclose={() => (removeOpen = false)} onconfirm={(mode) => void removeSelected(mode)} />
{/if}

{#if foldersOpen && folders}
  <FoldersDialog folders={folders.folders} onclose={() => (foldersOpen = false)} onsave={(paths) => void saveFolders(paths)} />
{/if}

<style>
  .ph {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    max-width: 1280px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .ph-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .ph-head-copy {
    min-width: 0;
  }

  .ph-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .ph-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .ph-tabs,
  .ph-tools {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
  }

  .ph-grow {
    flex: 1;
  }

  .ph-search {
    flex: 1 1 220px;
    max-width: 320px;
  }

  .ph-count {
    margin-left: auto;
    color: var(--text-dim);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }

  .ph-status {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }

  .ph-spin {
    width: 12px;
    height: 12px;
  }

  .ph-progress {
    flex: 0 1 200px;
  }

  .ph-notice {
    align-items: center;
  }

  .ph-notice .notice-text {
    flex: 1;
    margin: 0;
    overflow-wrap: anywhere;
  }

  .ph-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    min-height: 280px;
    padding: var(--space-7) var(--space-5);
    border-radius: var(--radius-xl);
    background: var(--surface);
    color: var(--text-dim);
    text-align: center;
  }

  .ph-empty h2 {
    margin: 0;
    color: var(--text);
    font-size: var(--text-lg);
    font-weight: 650;
  }

  .ph-empty p {
    margin: 0;
    max-width: 440px;
  }

  .ph-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    content-visibility: auto;
    contain-intrinsic-size: auto 420px;
  }

  .ph-section-head {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .ph-section-head h2 {
    margin: 0;
    color: var(--text);
    font-size: var(--text-md);
    font-weight: 650;
  }

  .ph-select-all {
    margin-left: auto;
    opacity: 0;
  }

  .ph-section:hover .ph-select-all,
  .ph-select-all:focus-visible {
    opacity: 1;
  }

  .ph-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: var(--space-2);
  }

  .ph-bar {
    position: sticky;
    bottom: 0;
    z-index: 2;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow:
      inset 0 0 0 var(--hairline) var(--content-border),
      0 8px 24px rgba(0, 0, 0, 0.18);
  }

  .ph-bar-count {
    color: var(--text);
    font-size: var(--text-sm);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  @media (hover: none) {
    .ph-select-all {
      opacity: 1;
    }
  }
</style>
