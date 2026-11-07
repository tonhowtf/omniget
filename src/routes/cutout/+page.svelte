<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { t } from "$lib/i18n";
  import * as api from "$lib/cutout/api";
  import { mergePaths } from "$lib/converter/plan";
  import { formatBytes } from "$lib/download-format";
  import {
    DEFAULT_MODEL,
    EDGE_LIMIT,
    MODEL_CHOICES,
    REFINE_MODEL,
    SWATCHES,
    addClick,
    addMoment,
    durationOf,
    formatMoment,
    installPercent,
    isVideo,
    modelBytes,
    modelReady,
    moveFrame,
    newRows,
    pending,
    tally,
    toItems,
    undoClick,
    type Row,
  } from "$lib/cutout/plan";
  import type { Background, Click, Edge, OutputFormat, Preview, ProgressEvent, Status } from "$lib/cutout/types";
  import Compare from "$lib/cutout/Compare.svelte";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import StatusIcon from "$components/converter/StatusIcon.svelte";

  type Phase = "empty" | "inspecting" | "ready" | "cutting" | "done";
  type BgKind = Background["kind"];

  const MEDIA_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "tga", "avif", "heic", "heif", "mp4", "m4v", "mkv", "webm", "mov", "avi"];
  const IMAGE_EXTENSIONS = MEDIA_EXTENSIONS.slice(0, 12);

  let phase = $state<Phase>("empty");
  let rows = $state<Row[]>([]);
  let selectedId = $state<string | null>(null);
  let status = $state<Status | null>(null);
  let notice = $state<string | null>(null);
  let dragging = $state(false);
  let stopping = $state(false);

  let model = $state<string>(DEFAULT_MODEL);
  let edge = $state<Edge>({ feather: 1, shift: 0 });
  let bgKind = $state<BgKind>("none");
  let bgColor = $state("#ffffff");
  let bgImage = $state<string | null>(null);
  let format = $state<OutputFormat>("png");
  let outDir = $state<string | null>(null);
  let refining = $state(false);

  let installing = $state<string | null>(null);
  let installParts = $state<Record<string, { done: number; total: number | null }>>({});
  let installError = $state<string | null>(null);

  let shown = $state<Preview | null>(null);
  let previewLoading = $state(false);
  let previewError = $state<string | null>(null);
  let frameDraft = $state<number | null>(null);

  let batchId: string | null = null;
  let batchRows: Row[] = [];
  let previewSeq = 0;
  let shownFor: string | null = null;

  let selected = $derived(rows.find((r) => r.id === selectedId) ?? null);
  let background = $derived<Background>(
    bgKind === "color" ? { kind: "color", color: bgColor } : bgKind === "image" && bgImage ? { kind: "image", path: bgImage } : { kind: "none" },
  );
  let ready = $derived(modelReady(status, model));
  let refineReady = $derived(modelReady(status, REFINE_MODEL));
  let runtimeMissing = $derived(!!status && !status.runtime_installed);
  let noRuntime = $derived(!!status && !status.runtime_installed && !status.runtime_downloadable);
  let todo = $derived(pending(rows));
  let counts = $derived(tally(rows));
  let locked = $derived(phase === "cutting");
  let percent = $derived(installPercent(installParts));
  let firstOutput = $derived(rows.find((r) => r.status === "done" && r.outcome?.output)?.outcome?.output ?? null);
  let usesClicks = $derived(!!selected && selected.clicks.length > 0);

  let primaryLabel = $derived(
    todo.length <= 1 ? ($t("cutout.cut_one") as string) : ($t("cutout.cut_many", { count: todo.length }) as string),
  );

  let doneTitle = $derived.by(() => {
    const { done, failed } = counts;
    if (failed === 0) return (done === 1 ? $t("cutout.done_one") : $t("cutout.done_many", { count: done })) as string;
    if (done === 0) return $t("cutout.done_none") as string;
    return $t("cutout.done_partial", { done, total: done + failed }) as string;
  });

  let liveSummary = $derived.by(() => {
    if (phase === "inspecting") return $t("cutout.inspecting") as string;
    if (phase === "cutting") return $t("cutout.progress", { done: counts.done + counts.failed, total: batchRows.length }) as string;
    if (phase === "done") return doneTitle;
    return "";
  });

  function newBatchId(): string {
    return typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `co-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  }

  function resetStatuses() {
    for (const row of rows) {
      if (row.status !== "queued" && row.status !== "running") {
        row.status = "idle";
        row.outcome = null;
      }
    }
    if (phase === "done") phase = "ready";
  }

  async function addPaths(paths: string[]) {
    if (locked || paths.length === 0) return;
    const known = [...new Set(rows.map((r) => r.source.path))];
    const fresh = mergePaths(known, paths).slice(known.length);
    if (fresh.length === 0) return;
    notice = null;
    if (rows.length === 0) phase = "inspecting";
    try {
      const inspected = await api.inspect(fresh);
      const added = newRows(inspected, rows.map((r) => r.id));
      if (phase === "done") resetStatuses();
      rows = [...rows, ...added];
      if (!selectedId) selectedId = added.find((r) => r.source.info)?.id ?? null;
    } catch (e) {
      notice = $t("cutout.inspect_error", { error: api.errorText(e) }) as string;
    } finally {
      if (phase === "inspecting") phase = rows.length ? "ready" : "empty";
    }
  }

  async function chooseFiles() {
    const picked = await openDialog({ multiple: true, directory: false, filters: [{ name: $t("cutout.filter_media") as string, extensions: MEDIA_EXTENSIONS }] }).catch(() => null);
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    await addPaths(paths);
  }

  async function chooseFolder() {
    const picked = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof picked === "string") outDir = picked;
  }

  async function chooseBackground() {
    const picked = await openDialog({ multiple: false, directory: false, filters: [{ name: $t("cutout.filter_images") as string, extensions: IMAGE_EXTENSIONS }] }).catch(() => null);
    if (typeof picked === "string") {
      bgImage = picked;
      bgKind = "image";
      resetStatuses();
    }
  }

  function select(row: Row) {
    selectedId = row.id;
    frameDraft = null;
  }

  function removeRow(row: Row) {
    const at = rows.indexOf(row);
    rows = rows.filter((r) => r !== row);
    if (selectedId === row.id) selectedId = (rows[at] ?? rows[at - 1])?.id ?? null;
    if (rows.length === 0) reset();
  }

  function reset() {
    rows = [];
    selectedId = null;
    shown = null;
    notice = null;
    refining = false;
    phase = "empty";
  }

  function commitFrame(row: Row, ms: number) {
    frameDraft = null;
    const moved = moveFrame(rows, row, ms);
    if (!moved) {
      notice = $t("cutout.moment_taken") as string;
      return;
    }
    rows = rows.map((r) => (r === row ? moved : r));
    selectedId = moved.id;
  }

  function anotherMoment(row: Row) {
    const before = rows.length;
    rows = addMoment(rows, row);
    if (rows.length > before) {
      selectedId = rows[rows.findIndex((r) => r.id === row.id) + 1].id;
      if (phase === "done") phase = "ready";
    }
  }

  function placeClick(click: Click) {
    if (!selected) return;
    selected.clicks = addClick(selected.clicks, click);
    selected.status = "idle";
    if (phase === "done") phase = "ready";
  }

  function undo() {
    if (selected) selected.clicks = undoClick(selected.clicks);
  }

  function clearClicks() {
    if (selected) selected.clicks = [];
  }

  function pickModel(id: string) {
    if (locked || id === model) return;
    model = id;
    resetStatuses();
  }

  function setEdge(next: Partial<Edge>) {
    edge = { ...edge, ...next };
    resetStatuses();
  }

  function setBackground(kind: BgKind) {
    if (kind === "image" && !bgImage) {
      void chooseBackground();
      return;
    }
    bgKind = kind;
    resetStatuses();
  }

  function setColor(color: string) {
    bgColor = color;
    bgKind = "color";
    resetStatuses();
  }

  async function install(id: string) {
    if (installing) return;
    installing = id;
    installParts = {};
    installError = null;
    try {
      status = await api.install(id);
      if (id === REFINE_MODEL) refining = true;
    } catch (e) {
      installError = api.errorText(e);
    } finally {
      installing = null;
    }
  }

  $effect(() => {
    const row = selected;
    const canUse = row?.source.info && (row.clicks.length > 0 ? refineReady : ready);
    if (!row || !canUse) {
      previewSeq += 1;
      previewLoading = false;
      previewError = null;
      shown = null;
      shownFor = null;
      return;
    }
    const seq = ++previewSeq;
    const rowKey = row.id;
    if (shownFor !== rowKey) {
      shown = null;
      previewError = null;
    }
    const { path } = row.source;
    const { frameMs } = row;
    const clicks = $state.snapshot(row.clicks) as Click[];
    const edgeNow = { ...edge };
    const bg = $state.snapshot(background) as Background;
    const modelNow = model;
    previewLoading = true;
    const timer = setTimeout(() => {
      api
        .preview(path, frameMs, modelNow, clicks, edgeNow, bg)
        .then((p) => {
          if (seq !== previewSeq) return;
          shown = p;
          shownFor = rowKey;
          previewError = null;
        })
        .catch((e) => {
          if (seq !== previewSeq) return;
          shown = null;
          shownFor = null;
          previewError = api.errorText(e);
        })
        .finally(() => {
          if (seq === previewSeq) previewLoading = false;
        });
    }, 140);
    return () => clearTimeout(timer);
  });

  function applyOutcome(row: Row, event: ProgressEvent) {
    if (event.outcome) row.outcome = event.outcome;
    if (event.state === "running") row.status = "running";
    else if (event.state === "queued") row.status = "queued";
    else if (event.state === "done") row.status = "done";
    else if (event.state === "failed") row.status = stopping ? "stopped" : "failed";
  }

  function onProgress(event: ProgressEvent) {
    if (!batchId || event.batch_id !== batchId) return;
    const row = batchRows[event.index];
    if (row) applyOutcome(row, event);
  }

  async function cut(only?: Row[]) {
    if (locked || !ready) return;
    const scope = (only ?? rows).filter((r) => r.source.info && (only || r.status !== "done"));
    if (scope.length === 0) return;
    if (scope.some((r) => r.clicks.length > 0) && !refineReady) {
      notice = $t("cutout.refine_missing") as string;
      return;
    }
    for (const row of scope) {
      row.status = "queued";
      row.outcome = null;
    }
    notice = null;
    stopping = false;
    batchId = newBatchId();
    batchRows = scope;
    phase = "cutting";
    const id = batchId;
    try {
      const outcomes = await api.run(id, toItems(scope), { model, edge: { ...edge }, background: $state.snapshot(background) as Background, format, out_dir: outDir, overwrite: false });
      outcomes.forEach((outcome, i) => {
        const row = scope[i];
        if (!row) return;
        row.outcome = outcome;
        row.status = outcome.ok ? "done" : stopping ? "stopped" : "failed";
      });
    } catch (e) {
      notice = $t("cutout.run_error", { error: api.errorText(e) }) as string;
    } finally {
      for (const row of scope) {
        if (row.status === "queued" || row.status === "running") row.status = "stopped";
      }
      if (batchId === id) {
        batchId = null;
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

  function retryFailed() {
    void cut(rows.filter((r) => r.status === "failed" || r.status === "stopped"));
  }

  function showInFolder() {
    if (firstOutput) void api.reveal(firstOutput).catch(() => {});
  }

  function rowLabel(row: Row): string {
    return row.frameMs === null ? row.source.name : `${row.source.name} · ${formatMoment(row.frameMs)}`;
  }

  function signed(value: number): string {
    return value > 0 ? `+${value}` : value < 0 ? `−${Math.abs(value)}` : "0";
  }

  onMount(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];
    const keep = (fn: () => void) => {
      if (disposed) fn();
      else cleanups.push(fn);
    };

    void api.status().then((s) => (status = s)).catch(() => {});
    void api.onProgress(onProgress).then(keep).catch(() => {});
    void api
      .onInstall((p) => {
        if (p.model !== installing) return;
        installParts = { ...installParts, [p.id]: { done: p.done, total: p.total } };
      })
      .then(keep)
      .catch(() => {});

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
      if (!batchId) void api.release().catch(() => {});
    };
  });
</script>

<section class="co" class:dragging aria-busy={phase === "inspecting" || phase === "cutting"}>
  <header class="co-head">
    <NavIcon icon="cutout" size={56} />
    <div class="co-head-copy">
      <h1>{$t("cutout.title")}</h1>
      <p class="co-subtitle">{$t("cutout.subtitle")}</p>
    </div>
  </header>

  <p class="co-live" role="status" aria-live="polite">{liveSummary}</p>

  {#if notice}
    <div class="co-notice" role="alert">
      <StatusIcon name="warning" />
      <p>{notice}</p>
    </div>
  {/if}

  {#if status && !ready}
    <div class="co-setup">
      <div class="co-setup-copy">
        <h2>{$t("cutout.setup_title")}</h2>
        {#if noRuntime}
          <p>{$t("cutout.setup_no_runtime")}</p>
        {:else}
          <p>{$t("cutout.setup_body", { size: formatBytes(modelBytes(status, model)) })}</p>
          {#if runtimeMissing}
            <p class="co-hint">{$t("cutout.setup_runtime")}</p>
          {/if}
        {/if}
        {#if installError && installing === null}
          <p class="co-error">{installError}</p>
        {/if}
      </div>
      {#if !noRuntime}
        <div class="co-setup-action">
          {#if installing === model}
            <progress class="co-progress" max="100" value={percent ?? undefined}></progress>
            <span class="co-hint">{percent === null ? $t("cutout.setup_starting") : $t("cutout.setup_progress", { percent })}</span>
          {:else}
            <button type="button" class="btn btn-primary" disabled={installing !== null} onclick={() => install(model)}>{$t("cutout.setup_download")}</button>
          {/if}
        </div>
      {/if}
    </div>
  {/if}

  {#if phase === "empty"}
    <div class="co-drop">
      <svg class="co-drop-art" viewBox="0 0 48 48" width="48" height="48" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <rect x="8" y="10" width="32" height="28" rx="3" stroke-dasharray="4 3" />
        <circle cx="24" cy="22" r="5" />
        <path d="M15 34c2-5 5.5-7 9-7s7 2 9 7" />
      </svg>
      <h2 class="co-drop-title">{$t("cutout.empty_title")}</h2>
      <p class="co-drop-body">{$t("cutout.empty_body")}</p>
      <button type="button" class="btn btn-primary" onclick={chooseFiles}>{$t("cutout.choose_files")}</button>
    </div>
  {:else if phase === "inspecting"}
    <div class="co-drop co-drop-busy">
      <span class="spinner" aria-hidden="true"></span>
      <p class="co-drop-body">{$t("cutout.inspecting")}</p>
    </div>
  {:else}
    {#if phase === "done"}
      <div class="co-result">
        <StatusIcon name={counts.failed > 0 ? (counts.done > 0 ? "warning" : "failed") : "done"} size={20} />
        <div class="co-result-copy">
          <h2>{doneTitle}</h2>
          {#if counts.failed > 0}
            <p>{$t("cutout.done_partial_body")}</p>
          {/if}
        </div>
      </div>
    {/if}

    <div class="co-work">
      <ul class="co-list" aria-label={$t("cutout.files") as string}>
        {#each rows as row (row.id)}
          <li class="co-row" class:active={row.id === selectedId} class:broken={!row.source.info}>
            <button type="button" class="co-row-main" disabled={!row.source.info} onclick={() => select(row)}>
              <StatusIcon name={!row.source.info ? "failed" : row.status === "idle" ? "idle" : row.status} size={14} />
              <span class="co-row-text">
                <span class="co-row-name" title={row.source.path}>{rowLabel(row)}</span>
                {#if row.source.error}
                  <span class="co-row-sub co-error">{row.source.error}</span>
                {:else if row.status === "failed" && row.outcome?.error}
                  <span class="co-row-sub co-error">{row.outcome.error}</span>
                {:else if row.clicks.length > 0}
                  <span class="co-row-sub">{$t("cutout.row_clicks", { count: row.clicks.length })}</span>
                {/if}
              </span>
            </button>
            <button type="button" class="btn btn-ghost btn-sm co-row-remove" disabled={locked} aria-label={$t("cutout.remove_file", { name: row.source.name }) as string} onclick={() => removeRow(row)}>×</button>
          </li>
        {/each}
      </ul>

      <div class="co-editor">
        {#if selected?.source.info}
          {#if usesClicks ? refineReady : ready}
            <Compare preview={shown} loading={previewLoading} clicks={selected.clicks} clickable={refining && refineReady} onclick={placeClick} />
          {:else}
            <div class="co-wait">{$t("cutout.preview_needs_model")}</div>
          {/if}
          {#if previewError}
            <p class="co-error">{previewError}</p>
          {/if}

          {#if isVideo(selected)}
            {@const row = selected}
            <div class="co-moment">
              <label class="co-field">
                <span class="field-label">{$t("cutout.moment")}</span>
                <span class="co-range">
                  <input
                    type="range"
                    min="0"
                    max={Math.max(0, durationOf(row) - 100)}
                    step="100"
                    value={frameDraft ?? row.frameMs ?? 0}
                    disabled={locked}
                    oninput={(e) => (frameDraft = Number(e.currentTarget.value))}
                    onchange={(e) => commitFrame(row, Number(e.currentTarget.value))}
                  />
                  <output class="co-num">{formatMoment(frameDraft ?? row.frameMs ?? 0)}</output>
                </span>
              </label>
              <button type="button" class="btn btn-secondary btn-sm" disabled={locked} onclick={() => anotherMoment(row)}>{$t("cutout.moment_add")}</button>
            </div>
          {/if}

          <div class="co-refine">
            {#if refineReady}
              <label class="co-check">
                <input type="checkbox" class="checkbox" bind:checked={refining} disabled={locked} />
                <span>
                  <span class="field-label">{$t("cutout.refine")}</span>
                  <span class="field-hint">{$t("cutout.refine_hint")}</span>
                </span>
              </label>
              {#if refining || usesClicks}
                <div class="co-refine-actions">
                  {#if usesClicks}
                    <span class="co-hint">{$t("cutout.refine_active")}</span>
                  {/if}
                  <button type="button" class="btn btn-ghost btn-sm" disabled={!usesClicks || locked} onclick={undo}>{$t("cutout.refine_undo")}</button>
                  <button type="button" class="btn btn-ghost btn-sm" disabled={!usesClicks || locked} onclick={clearClicks}>{$t("cutout.refine_clear")}</button>
                </div>
              {/if}
            {:else if status && !noRuntime}
              <div class="co-refine-offer">
                <span>
                  <span class="field-label">{$t("cutout.refine")}</span>
                  <span class="field-hint">{$t("cutout.refine_offer", { size: formatBytes(status.refine.bytes) })}</span>
                </span>
                {#if installing === REFINE_MODEL}
                  <progress class="co-progress" max="100" value={percent ?? undefined}></progress>
                {:else}
                  <button type="button" class="btn btn-secondary btn-sm" disabled={installing !== null} onclick={() => install(REFINE_MODEL)}>{$t("cutout.refine_download")}</button>
                {/if}
              </div>
            {/if}
          </div>
        {:else}
          <div class="co-wait">{$t("cutout.pick_file")}</div>
        {/if}
      </div>
    </div>

    <fieldset class="co-opts" disabled={locked}>
      <section class="co-opts-section">
        <h3 class="co-opts-title">{$t("cutout.model")}</h3>
        <div class="segmented" role="radiogroup" aria-label={$t("cutout.model") as string}>
          {#each MODEL_CHOICES as choice (choice.id)}
            <button type="button" class="segmented-btn" role="radio" aria-checked={model === choice.id} onclick={() => pickModel(choice.id)}>{$t(choice.label)}</button>
          {/each}
        </div>
        <p class="field-hint">
          {$t(MODEL_CHOICES.find((c) => c.id === model)?.hint ?? "cutout.model_fast_hint")}
          {#if status}· {formatBytes(modelBytes(status, model))}{/if}
        </p>
      </section>

      <section class="co-opts-section">
        <h3 class="co-opts-title">{$t("cutout.edge")}</h3>
        <label class="co-field">
          <span class="field-label">{$t("cutout.edge_soften")}</span>
          <span class="co-range">
            <input type="range" min="0" max={EDGE_LIMIT} step="0.5" value={edge.feather} oninput={(e) => setEdge({ feather: Number(e.currentTarget.value) })} />
            <output class="co-num">{edge.feather}</output>
          </span>
        </label>
        <label class="co-field">
          <span class="field-label">{$t("cutout.edge_shift")}</span>
          <span class="co-range">
            <input type="range" min={-EDGE_LIMIT} max={EDGE_LIMIT} step="0.5" value={edge.shift} oninput={(e) => setEdge({ shift: Number(e.currentTarget.value) })} />
            <output class="co-num">{signed(edge.shift)}</output>
          </span>
        </label>
        <p class="field-hint">{$t("cutout.edge_hint")}</p>
        {#if edge.feather !== 1 || edge.shift !== 0}
          <button type="button" class="btn btn-ghost btn-sm co-reset" onclick={() => setEdge({ feather: 1, shift: 0 })}>{$t("cutout.edge_reset")}</button>
        {/if}
      </section>

      <section class="co-opts-section">
        <h3 class="co-opts-title">{$t("cutout.background")}</h3>
        <div class="segmented" role="radiogroup" aria-label={$t("cutout.background") as string}>
          <button type="button" class="segmented-btn" role="radio" aria-checked={bgKind === "none"} onclick={() => setBackground("none")}>{$t("cutout.bg_none")}</button>
          <button type="button" class="segmented-btn" role="radio" aria-checked={bgKind === "color"} onclick={() => setBackground("color")}>{$t("cutout.bg_color")}</button>
          <button type="button" class="segmented-btn" role="radio" aria-checked={bgKind === "image"} onclick={() => setBackground("image")}>{$t("cutout.bg_image")}</button>
        </div>
        {#if bgKind === "color"}
          <div class="co-swatches">
            {#each SWATCHES as swatch (swatch)}
              <button type="button" class="co-swatch" class:on={bgColor.toLowerCase() === swatch} style:background={swatch} aria-label={swatch} onclick={() => setColor(swatch)}></button>
            {/each}
            <label class="co-swatch co-swatch-pick" title={$t("cutout.bg_pick") as string}>
              <input type="color" value={bgColor} oninput={(e) => setColor(e.currentTarget.value)} aria-label={$t("cutout.bg_pick") as string} />
            </label>
          </div>
        {:else if bgKind === "image" && bgImage}
          <div class="co-folder">
            <span class="co-path" title={bgImage}>{bgImage.split(/[\\/]/).pop()}</span>
            <button type="button" class="btn btn-secondary btn-sm" onclick={chooseBackground}>{$t("cutout.bg_change")}</button>
          </div>
        {/if}
      </section>

      <section class="co-opts-section">
        <h3 class="co-opts-title">{$t("cutout.save")}</h3>
        <div class="segmented" role="radiogroup" aria-label={$t("cutout.format") as string}>
          <button type="button" class="segmented-btn" role="radio" aria-checked={format === "png"} onclick={() => ((format = "png"), resetStatuses())}>PNG</button>
          <button type="button" class="segmented-btn" role="radio" aria-checked={format === "webp"} onclick={() => ((format = "webp"), resetStatuses())}>WebP</button>
        </div>
        <div class="co-folder">
          <span class="co-path" title={outDir ?? undefined}>{outDir ?? $t("cutout.save_next_to")}</span>
          <button type="button" class="btn btn-secondary btn-sm" onclick={chooseFolder}>{$t("cutout.save_choose")}</button>
          {#if outDir}
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => (outDir = null)}>{$t("cutout.save_reset")}</button>
          {/if}
        </div>
      </section>
    </fieldset>

    <footer class="co-bar">
      <div class="co-bar-side">
        {#if phase === "done"}
          <button type="button" class="btn btn-secondary" onclick={reset}>{$t("cutout.cut_more")}</button>
        {:else}
          <button type="button" class="btn btn-secondary" disabled={locked} onclick={chooseFiles}>{$t("cutout.add_files")}</button>
          <button type="button" class="btn btn-ghost" disabled={locked} onclick={reset}>{$t("cutout.clear")}</button>
        {/if}
      </div>
      <div class="co-bar-main">
        {#if phase === "cutting"}
          <span class="co-bar-progress" aria-hidden="true">{liveSummary}</span>
          <button type="button" class="btn btn-secondary" disabled={stopping} onclick={stop}>{stopping ? $t("cutout.stopping") : $t("cutout.stop")}</button>
        {:else if phase === "done"}
          {#if firstOutput}
            <button type="button" class={counts.failed > 0 ? "btn btn-secondary" : "btn btn-primary"} onclick={showInFolder}>{$t("cutout.show_in_folder")}</button>
          {/if}
          {#if counts.failed > 0 || counts.stopped > 0}
            <button type="button" class="btn btn-primary" onclick={retryFailed}>{$t("cutout.retry_failed")}</button>
          {/if}
          {#if todo.length > 0}
            <button type="button" class="btn btn-secondary" disabled={!ready} onclick={() => cut()}>{primaryLabel}</button>
          {/if}
        {:else}
          <button type="button" class="btn btn-primary" disabled={!ready || todo.length === 0} onclick={() => cut()}>{primaryLabel}</button>
        {/if}
      </div>
    </footer>
  {/if}

  {#if dragging && phase !== "empty"}
    <div class="co-dropping" aria-hidden="true">
      <p>{$t("cutout.drop_here")}</p>
    </div>
  {/if}
</section>

<style>
  .co {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    width: 100%;
    max-width: 1100px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .co-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .co-head-copy {
    min-width: 0;
  }

  .co-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .co-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .co-live {
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

  .co-notice,
  .co-result,
  .co-setup {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .co-notice p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .co-setup {
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4);
  }

  .co-setup-copy {
    flex: 1 1 320px;
    min-width: 0;
  }

  .co-setup-copy h2,
  .co-result-copy h2 {
    margin: 0 0 var(--space-1);
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .co-setup-copy p,
  .co-result-copy p {
    margin: 0 0 var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .co-setup-action {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--space-1);
  }

  .co-progress {
    width: 180px;
    accent-color: var(--accent);
  }

  .co-hint {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .co-error {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--danger, #d70015);
    overflow-wrap: anywhere;
  }

  .co-drop {
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

  .co.dragging .co-drop {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .co-drop-busy {
    border-style: solid;
  }

  .co-drop-art {
    color: var(--text-dim);
  }

  .co-drop-title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .co-drop-body {
    margin: 0;
    max-width: 440px;
    font-size: var(--text-base);
    color: var(--text-dim);
  }

  .co-work {
    display: grid;
    grid-template-columns: minmax(200px, 260px) minmax(0, 1fr);
    gap: var(--space-4);
    align-items: start;
  }

  @media (max-width: 760px) {
    .co-work {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .co-list {
    margin: 0;
    padding: var(--space-1);
    list-style: none;
    max-height: 60vh;
    overflow: auto;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .co-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    border-radius: var(--radius-md);
  }

  .co-row.active {
    background: var(--accent-soft);
  }

  .co-row-main {
    display: flex;
    flex: 1;
    min-width: 0;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    border-radius: var(--radius-md);
  }

  .co-row-main:disabled {
    cursor: default;
  }

  .co-row-main:focus-visible {
    outline: var(--focus-ring);
  }

  .co-row-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    gap: 2px;
  }

  .co-row-name {
    overflow: hidden;
    font-size: var(--text-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .co-row-sub {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .co-row.broken .co-row-name {
    color: var(--text-dim);
  }

  .co-row-remove {
    flex-shrink: 0;
  }

  .co-editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .co-wait {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 280px;
    padding: var(--space-5);
    border-radius: var(--radius-lg);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--text-sm);
    text-align: center;
  }

  .co-moment {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-3);
  }

  .co-moment .co-field {
    flex: 1 1 260px;
  }

  .co-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .co-range {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .co-range input {
    flex: 1;
  }

  .co-num {
    min-width: 44px;
    font-size: var(--text-sm);
    color: var(--text-muted);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .co-refine {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .co-check,
  .co-refine-offer > span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .co-check {
    flex-direction: row;
    align-items: flex-start;
    gap: var(--space-2);
  }

  .co-check > span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .co-refine-offer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .co-refine-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .co-opts {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
    gap: var(--space-4);
    margin: 0;
    padding: var(--space-4);
    border: 0;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .co-opts-section {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    min-width: 0;
  }

  .co-opts-section .co-field {
    align-self: stretch;
  }

  .co-opts-title {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 650;
    color: var(--text);
  }

  .co-reset {
    align-self: flex-start;
  }

  .co-swatches {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .co-swatch {
    position: relative;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.18);
    cursor: pointer;
  }

  .co-swatch.on {
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.18), 0 0 0 2px var(--pane-bg), 0 0 0 4px var(--accent);
  }

  .co-swatch:focus-visible {
    outline: var(--focus-ring);
  }

  .co-swatch-pick {
    overflow: hidden;
    background: conic-gradient(#ff3b30, #ffcc00, #34c759, #1e6fe8, #af52de, #ff3b30);
  }

  .co-swatch-pick input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    cursor: pointer;
  }

  .co-folder {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    max-width: 100%;
  }

  .co-path {
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    font-size: var(--text-sm);
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .co-bar {
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

  .co-bar-side,
  .co-bar-main {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .co-bar-progress {
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .co-dropping {
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

  .co-dropping p {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  @media (prefers-reduced-motion: reduce) {
    .co-drop {
      transition: none;
    }
  }
</style>
