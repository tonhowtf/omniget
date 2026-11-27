<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { t } from "$lib/i18n";
  import * as api from "$lib/converter/api";
  import {
    EXTERNAL_TOOLS,
    ENGINE_NAMES,
    commonTargets,
    labeler,
    mergePaths,
    planRow,
    tally,
    tracesToSvg,
    type RowStatus,
  } from "$lib/converter/plan";
  import type { ConvertOptions, ConvertOutcome, FormatInfo, InspectResult, ProgressEvent, RunItem, ToolStatus, VectorPreset } from "$lib/converter/types";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import FileRow from "#components/converter/FileRow.svelte";
  import TargetPicker from "#components/converter/TargetPicker.svelte";
  import OptionsPanel, { type ConverterOpts } from "#components/converter/OptionsPanel.svelte";
  import StatusIcon from "#components/converter/StatusIcon.svelte";
  import VectorStyle from "#components/converter/VectorStyle.svelte";

  type Phase = "empty" | "inspecting" | "ready" | "converting" | "done";

  type Row = {
    file: InspectResult;
    override: string | null;
    status: RowStatus;
    percent: number;
    outcome: ConvertOutcome | null;
    runError: string | null;
  };

  let phase = $state<Phase>("empty");
  let rows = $state<Row[]>([]);
  let batchTarget = $state<string | null>(null);
  let formats = $state<FormatInfo[]>([]);
  let toolStatus = $state<ToolStatus | null>(null);
  let checkingTools = $state(false);
  let busyAdds = $state(0);
  let adding = $derived(busyAdds > 0 && phase !== "inspecting");
  const inFlight = new Set<string>();
  let dragging = $state(false);
  let stopping = $state(false);
  let notice = $state<string | null>(null);
  let batchId: string | null = null;
  let batchRows: Row[] = [];

  let opts = $state<ConverterOpts>({
    pretty: true,
    sortKeys: false,
    quality: 88,
    maxWidth: null,
    maxHeight: null,
    outDir: null,
    overwrite: false,
  });

  let labelOf = $derived(labeler(formats));
  let common = $derived(commonTargets(rows.map((r) => r.file)));
  let readableCount = $derived(rows.filter((r) => !r.file.error && r.file.format).length);
  let batchLabel = $derived(batchTarget ? labelOf(batchTarget) : null);
  let locked = $derived(phase === "converting");

  let plans = $derived(rows.map((r) => planRow(r.file, r.override ?? batchTarget)));
  let pending = $derived(rows.filter((r, i) => plans[i].kind === "convert" && r.status !== "done"));
  let counts = $derived(tally(rows.map((r) => r.status)));

  let targetCats = $derived.by(() => {
    const cats = new Set<string>();
    rows.forEach((r, i) => {
      const plan = plans[i];
      if (plan.kind === "convert") cats.add(plan.option.category);
      else if (r.file.format) cats.add(r.file.format.category);
    });
    return cats;
  });
  let vectorPreset = $state<VectorPreset>("colors");
  let showVector = $derived(plans.some((p) => p.kind === "convert" && tracesToSvg(p.option.route)));
  let showQuality = $derived(plans.some((p) => p.kind === "convert" && p.option.format === "jpeg") || (!batchTarget && targetCats.has("image")));

  let primaryLabel = $derived(
    pending.length === 0
      ? ($t("converter.convert") as string)
      : pending.length === 1
        ? ($t("converter.convert_one") as string)
        : ($t("converter.convert_many", { count: pending.length }) as string),
  );

  let liveSummary = $derived.by(() => {
    if (phase === "inspecting" || adding) return $t(phase === "inspecting" ? "converter.inspecting" : "converter.adding") as string;
    if (phase === "converting") {
      const total = counts.queued + counts.running + counts.done + counts.failed;
      const finished = counts.done + counts.failed;
      const base = $t("converter.progress", { done: finished, total }) as string;
      return counts.failed > 0 ? `${base} · ${$t("converter.progress_failed", { count: counts.failed })}` : base;
    }
    if (phase === "done") return doneTitle;
    return "";
  });

  let doneTitle = $derived.by(() => {
    const { done, failed } = counts;
    if (failed === 0 && done === 0) return $t("converter.done_skipped_only") as string;
    if (failed === 0) return (done === 1 ? $t("converter.done_one") : $t("converter.done_many", { count: done })) as string;
    if (done === 0) return $t("converter.done_none") as string;
    return $t("converter.done_partial", { done, total: done + failed }) as string;
  });

  let firstOutput = $derived(rows.find((r) => r.status === "done" && r.outcome?.output)?.outcome?.output ?? null);

  function newRow(file: InspectResult): Row {
    return { file, override: null, status: "idle", percent: 0, outcome: null, runError: null };
  }

  function resetStatuses() {
    for (const row of rows) {
      if (row.status !== "running" && row.status !== "queued") {
        row.status = "idle";
        row.percent = 0;
        row.outcome = null;
        row.runError = null;
      }
    }
  }

  function knownPaths(): string[] {
    return [...rows.map((r) => r.file.path), ...inFlight];
  }

  async function addPaths(paths: string[]) {
    if (locked || paths.length === 0) return;
    const known = knownPaths();
    const fresh = mergePaths(known, paths).slice(known.length);
    if (fresh.length === 0) return;
    fresh.forEach((path) => inFlight.add(path));
    notice = null;
    busyAdds += 1;
    if (rows.length === 0) phase = "inspecting";
    try {
      const results = await api.inspect(fresh);
      fresh.forEach((path) => inFlight.delete(path));
      const existing = new Set(rows.map((r) => r.file.path));
      const unique = results.filter((f) => !existing.has(f.path) && existing.add(f.path));
      if (phase === "done") resetStatuses();
      rows = [...rows, ...unique.map(newRow)];
      if (batchTarget && !commonTargets(rows.map((r) => r.file)).some((c) => c.format === batchTarget)) batchTarget = null;
    } catch (e) {
      notice = $t("converter.inspect_error", { error: api.errorText(e) }) as string;
    } finally {
      fresh.forEach((path) => inFlight.delete(path));
      busyAdds -= 1;
      if (phase === "inspecting" && busyAdds === 0) phase = rows.length ? "ready" : "empty";
      else if (phase === "done" && rows.some((r) => r.status === "idle")) phase = "ready";
    }
  }

  async function chooseFiles() {
    const picked = await openDialog({ multiple: true, directory: false }).catch(() => null);
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    await addPaths(paths);
  }

  async function chooseFolder() {
    const picked = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof picked === "string") opts.outDir = picked;
  }

  function pickBatch(format: string) {
    if (locked) return;
    batchTarget = batchTarget === format ? null : format;
    if (phase === "done") phase = "ready";
    resetStatuses();
  }

  function setOverride(row: Row, format: string | null) {
    row.override = format;
    row.status = "idle";
    row.percent = 0;
    row.outcome = null;
    row.runError = null;
    if (phase === "done") phase = "ready";
  }

  function removeRow(row: Row) {
    rows = rows.filter((r) => r !== row);
    if (rows.length === 0) reset();
  }

  function reset() {
    rows = [];
    batchTarget = null;
    notice = null;
    phase = "empty";
  }

  function buildOptions(): ConvertOptions {
    const out: ConvertOptions = { pretty: opts.pretty, sort_keys: opts.sortKeys, quality: opts.quality, overwrite: opts.overwrite, vector_preset: vectorPreset };
    if (opts.maxWidth && opts.maxWidth > 0) out.max_width = Math.round(opts.maxWidth);
    if (opts.maxHeight && opts.maxHeight > 0) out.max_height = Math.round(opts.maxHeight);
    return out;
  }

  function newBatchId(): string {
    return typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `cv-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  }

  function applyOutcome(row: Row, outcome: ConvertOutcome) {
    row.outcome = outcome;
    row.percent = 100;
    if (outcome.skipped) row.status = "skipped";
    else if (outcome.ok) row.status = "done";
    else row.status = stopping ? "stopped" : "failed";
  }

  async function convert(only?: Row[]) {
    if (locked || busyAdds > 0) return;
    const scope = only ?? rows;
    const items: RunItem[] = [];
    const sent: Row[] = [];
    for (const row of scope) {
      const plan = planRow(row.file, row.override ?? batchTarget);
      if (plan.kind === "skip") {
        row.status = "skipped";
        continue;
      }
      if (plan.kind !== "convert" || (row.status === "done" && !only)) continue;
      row.status = "queued";
      row.percent = 0;
      row.outcome = null;
      row.runError = null;
      items.push({ path: row.file.path, target: plan.option.format });
      sent.push(row);
    }
    if (items.length === 0) {
      phase = "done";
      return;
    }
    notice = null;
    stopping = false;
    batchId = newBatchId();
    batchRows = sent;
    phase = "converting";
    const id = batchId;
    try {
      const outcomes = await api.run(id, items, opts.outDir, buildOptions());
      outcomes.forEach((outcome, i) => {
        const row = sent[i];
        if (row) applyOutcome(row, outcome);
      });
    } catch (e) {
      const message = api.errorText(e);
      notice = $t("converter.run_error", { error: message }) as string;
      for (const row of rows) {
        if (row.status === "queued" || row.status === "running") {
          row.status = stopping ? "stopped" : "failed";
          row.runError = message;
        }
      }
    } finally {
      for (const row of rows) {
        if (row.status === "queued" || row.status === "running") row.status = "stopped";
      }
      if (batchId === id) {
        batchId = null;
        batchRows = [];
      }
      phase = stopping ? "ready" : "done";
      stopping = false;
    }
  }

  async function stop() {
    if (!batchId || stopping) return;
    stopping = true;
    await api.cancel(batchId).catch(() => {});
  }

  function onProgress(event: ProgressEvent) {
    if (!batchId || event.batch_id !== batchId) return;
    const row = batchRows[event.index];
    if (!row) return;
    if (event.outcome) {
      applyOutcome(row, event.outcome);
      return;
    }
    const percent = Number.isFinite(event.percent) ? Math.max(0, Math.min(100, event.percent)) : 0;
    if (event.state === "running") {
      row.status = "running";
      row.percent = percent;
    } else if (event.state === "queued") {
      row.status = "queued";
    } else if (event.state === "done") {
      row.status = "done";
      row.percent = 100;
    } else if (event.state === "skipped") {
      row.status = "skipped";
    } else if (event.state === "failed") {
      row.status = stopping ? "stopped" : "failed";
    }
  }

  async function recheck() {
    if (checkingTools) return;
    checkingTools = true;
    try {
      toolStatus = await api.tools();
      if (rows.length && !locked) {
        const fresh = await api.inspect(rows.map((r) => r.file.path));
        const byPath = new Map(fresh.map((f) => [f.path, f]));
        for (const row of rows) {
          const next = byPath.get(row.file.path);
          if (next) row.file = next;
        }
      }
    } catch (e) {
      notice = $t("converter.inspect_error", { error: api.errorText(e) }) as string;
    } finally {
      checkingTools = false;
    }
  }

  function retryFailed() {
    void convert(rows.filter((r) => r.status === "failed" || r.status === "stopped"));
  }

  function showInFolder() {
    if (firstOutput) void api.reveal(firstOutput).catch(() => {});
  }

  onMount(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];
    const keep = (fn: () => void) => {
      if (disposed) fn();
      else cleanups.push(fn);
    };

    void api.formats().then((list) => (formats = list)).catch(() => {});
    void api.tools().then((s) => (toolStatus = s)).catch(() => {});
    void api.onProgress(onProgress).then(keep).catch(() => {});

    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter" || p.type === "over") dragging = !locked;
          else if (p.type === "leave") dragging = false;
          else if (p.type === "drop") {
            dragging = false;
            if (p.paths?.length) void addPaths(p.paths);
          }
        })
        .then(keep)
        .catch(() => {});
    } catch {
      dragging = false;
    }

    return () => {
      disposed = true;
      cleanups.forEach((fn) => fn());
    };
  });
</script>

<section class="cv" class:dragging aria-busy={phase === "inspecting" || phase === "converting"}>
  <header class="cv-head">
    <NavIcon icon="converter" size={56} />
    <div class="cv-head-copy">
      <h1>{$t("converter.title")}</h1>
      <p class="cv-subtitle">{$t("converter.subtitle")}</p>
    </div>
  </header>

  <p class="cv-live" role="status" aria-live="polite">{liveSummary}</p>

  {#if notice}
    <div class="cv-notice" role="alert">
      <StatusIcon name="warning" />
      <p>{notice}</p>
    </div>
  {/if}

  {#if phase === "empty"}
    <div class="cv-drop">
      <svg class="cv-drop-art" viewBox="0 0 48 48" width="48" height="48" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M14 8h14l8 8v24a2 2 0 0 1-2 2H14a2 2 0 0 1-2-2V10a2 2 0 0 1 2-2z" />
        <path d="M28 8v8h8" />
        <path d="M18 28h12M26 24l4 4-4 4" />
      </svg>
      <h2 class="cv-drop-title">{$t("converter.empty_title")}</h2>
      <p class="cv-drop-body">{$t("converter.empty_body")}</p>
      <button type="button" class="btn btn-primary" onclick={chooseFiles}>{$t("converter.choose_files")}</button>
    </div>
  {:else if phase === "inspecting"}
    <div class="cv-drop cv-drop-busy">
      <span class="spinner" aria-hidden="true"></span>
      <p class="cv-drop-body">{$t("converter.inspecting")}</p>
    </div>
  {:else}
    {#if phase === "done"}
      <div class="cv-result" class:partial={counts.failed > 0}>
        <StatusIcon name={counts.failed > 0 ? (counts.done > 0 ? "warning" : "failed") : counts.done > 0 ? "done" : "skipped"} size={20} />
        <div class="cv-result-copy">
          <h2>{doneTitle}</h2>
          {#if counts.failed > 0}
            <p>{$t("converter.done_partial_body")}</p>
          {/if}
          {#if counts.skipped > 0 && (counts.done > 0 || counts.failed > 0)}
            <p>{$t("converter.skipped_count", { count: counts.skipped })}</p>
          {/if}
        </div>
      </div>
    {:else if readableCount > 0}
      <div class="cv-block">
        <h2 class="cv-block-title">{$t("converter.convert_to")}</h2>
        {#if common.length > 0}
          <TargetPicker targets={common} value={batchTarget} disabled={locked} onpick={pickBatch} />
          {#if !batchTarget && pending.length === 0}
            <p class="cv-hint">{$t("converter.pick_hint")}</p>
          {/if}
        {:else}
          <p class="cv-hint">{$t("converter.no_common")}</p>
        {/if}
      </div>
    {/if}

    {#if showVector && phase !== "done"}
      <VectorStyle bind:value={vectorPreset} disabled={locked} />
    {/if}

    <ul class="cv-list">
      {#each rows as row, i (row.file.path)}
        <FileRow
          file={row.file}
          status={row.status}
          percent={row.percent}
          outcome={row.outcome}
          runError={row.runError}
          plan={plans[i]}
          override={row.override}
          {batchLabel}
          {labelOf}
          {locked}
          onoverride={(format) => setOverride(row, format)}
          onremove={() => removeRow(row)}
        />
      {/each}
    </ul>

    {#if phase !== "done"}
      <OptionsPanel
        bind:opts
        showData={targetCats.has("data")}
        showImage={targetCats.has("image")}
        {showQuality}
        disabled={locked}
        onchoosefolder={chooseFolder}
      />
    {/if}

    <footer class="cv-bar">
      <div class="cv-bar-side">
        {#if phase === "done"}
          <button type="button" class="btn btn-secondary" onclick={reset}>{$t("converter.convert_more")}</button>
        {:else}
          <button type="button" class="btn btn-secondary" disabled={locked || adding} onclick={chooseFiles}>
            {#if adding}<span class="spinner cv-btn-spin" aria-hidden="true"></span>{/if}
            {$t("converter.add_files")}
          </button>
          <button type="button" class="btn btn-ghost" disabled={locked} onclick={reset}>{$t("converter.clear")}</button>
        {/if}
      </div>

      <div class="cv-bar-main">
        {#if phase === "converting"}
          <span class="cv-bar-progress" aria-hidden="true">{liveSummary}</span>
          <button type="button" class="btn btn-secondary" disabled={stopping} onclick={stop}>
            {stopping ? $t("converter.stopping") : $t("converter.stop")}
          </button>
        {:else if phase === "done"}
          {#if counts.failed > 0}
            {#if firstOutput}
              <button type="button" class="btn btn-secondary" onclick={showInFolder}>{$t("converter.show_in_folder")}</button>
            {/if}
            <button type="button" class="btn btn-primary" disabled={adding} onclick={retryFailed}>{$t("converter.retry_failed")}</button>
          {:else if firstOutput}
            <button type="button" class="btn btn-primary" onclick={showInFolder}>{$t("converter.show_in_folder")}</button>
          {/if}
        {:else}
          <button type="button" class="btn btn-primary" disabled={pending.length === 0 || adding} onclick={() => convert()}>{primaryLabel}</button>
        {/if}
      </div>
    </footer>
  {/if}

  <div class="cv-tools">
    <span class="cv-tools-label">{$t("converter.tools")}</span>
    <ul class="cv-tools-list">
      {#each EXTERNAL_TOOLS as tool (tool)}
        {@const found = toolStatus?.[tool] ?? false}
        <li class="cv-tool" class:found>
          {#if toolStatus}
            <StatusIcon name={found ? "done" : "idle"} size={13} />
          {/if}
          <span>{ENGINE_NAMES[tool]}</span>
          {#if toolStatus}
            <span class="cv-tool-state">{found ? $t("converter.tool_found") : $t("converter.tool_missing")}</span>
          {/if}
        </li>
      {/each}
    </ul>
    <button type="button" class="btn btn-ghost btn-sm" disabled={checkingTools} onclick={recheck}>
      {checkingTools ? $t("converter.tools_checking") : $t("converter.tools_recheck")}
    </button>
  </div>

  {#if dragging && phase !== "empty"}
    <div class="cv-dropping" aria-hidden="true">
      <p>{$t("converter.drop_here")}</p>
    </div>
  {/if}
</section>

<style>
  .cv {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .cv-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .cv-head-copy {
    min-width: 0;
  }

  .cv-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .cv-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .cv-live {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }

  .cv-notice {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cv-notice p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .cv-drop {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    min-height: 320px;
    padding: var(--space-7) var(--space-5);
    border: 1.5px dashed var(--content-border);
    border-radius: var(--radius-xl);
    background: var(--surface);
    text-align: center;
    transition: border-color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }

  .cv.dragging .cv-drop {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .cv-drop-busy {
    border-style: solid;
  }

  .cv-drop-art {
    color: var(--text-dim);
  }

  .cv-drop-title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .cv-drop-body {
    margin: 0;
    max-width: 420px;
    font-size: var(--text-base);
    color: var(--text-dim);
  }

  .cv-block {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .cv-block-title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .cv-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .cv-result {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cv-result-copy h2 {
    margin: 0 0 var(--space-1);
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .cv-result-copy p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .cv-list {
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cv-bar {
    position: sticky;
    bottom: 0;
    z-index: 1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) 0;
    background: var(--pane-bg);
  }

  .cv-bar-side,
  .cv-bar-main {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .cv-bar-progress {
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .cv-btn-spin {
    width: 12px;
    height: 12px;
  }

  .cv-tools {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .cv-tools-label {
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
  }

  .cv-tools-list {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .cv-tool {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .cv-tool.found {
    color: var(--text-muted);
  }

  .cv-tool-state {
    color: var(--text-dim);
  }

  .cv-dropping {
    position: absolute;
    inset: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px dashed var(--accent);
    border-radius: var(--radius-xl);
    background: color-mix(in srgb, var(--pane-bg) 82%, transparent);
    pointer-events: none;
  }

  .cv-dropping p {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  @media (prefers-reduced-motion: reduce) {
    .cv-drop {
      transition: none;
    }
  }
</style>
