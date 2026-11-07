<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "$lib/pdf/api";
  import { formatBytes } from "$lib/download-format";
  import { joinPath, parentDir, pdfName, plural, samePath, stem } from "$lib/pdf/pages";
  import type { CompressOutcome, PdfInfo, Preset } from "$lib/pdf/types";
  import StatusIcon from "$components/converter/StatusIcon.svelte";

  let { onbusy }: { onbusy?: (busy: boolean) => void } = $props();

  type Row = {
    file: PdfInfo;
    state: "idle" | "running" | "done" | "same" | "failed" | "stopped";
    percent: number;
    outcome: CompressOutcome | null;
    error: string | null;
  };

  const PRESETS: Preset[] = ["light", "balanced", "strong"];

  let rows = $state<Row[]>([]);
  let preset = $state<Preset>("balanced");
  let running = $state(false);
  let stopping = $state(false);
  let adding = $state(false);
  let jobId: string | null = null;
  let current: Row | null = null;

  let pending = $derived(rows.filter((r) => !r.file.error && (r.state === "idle" || r.state === "failed" || r.state === "stopped")));
  let busy = $derived(running || adding);
  let saved = $derived(rows.reduce((sum, r) => sum + (r.state === "done" && r.outcome ? r.outcome.before_bytes - r.outcome.after_bytes : 0), 0));

  $effect(() => onbusy?.(busy));

  function errorText(code: string | null): string {
    return $t(`pdf.error_${code ?? "io"}`) as string;
  }

  export async function add(paths: string[]) {
    const fresh = paths.filter((p) => /\.pdf$/i.test(p) && !rows.some((r) => samePath(r.file.path, p)));
    if (fresh.length === 0 || busy) return;
    adding = true;
    try {
      const infos = await api.inspect(fresh);
      rows = [...rows, ...infos.map((file) => ({ file, state: "idle" as const, percent: 0, outcome: null, error: null }))];
    } finally {
      adding = false;
    }
  }

  async function chooseFiles() {
    const picked = await openDialog({ multiple: true, directory: false, filters: [{ name: "PDF", extensions: ["pdf"] }] }).catch(() => null);
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    await add(paths);
  }

  function pickPreset(next: Preset) {
    if (running) return;
    preset = next;
    for (const row of rows) {
      if (row.state !== "running") {
        row.state = "idle";
        row.outcome = null;
        row.error = null;
        row.percent = 0;
      }
    }
  }

  async function start() {
    if (busy || pending.length === 0) return;
    running = true;
    stopping = false;
    for (const row of pending) {
      if (stopping) break;
      current = row;
      row.state = "running";
      row.percent = 0;
      row.error = null;
      jobId = api.newJobId();
      const out = joinPath(parentDir(row.file.path), pdfName($t("pdf.name_compressed", { name: stem(row.file.path) }) as string));
      try {
        const outcome = await api.compress(jobId, row.file.path, preset, out);
        row.outcome = outcome;
        row.state = outcome.path ? "done" : "same";
      } catch (e) {
        const code = api.failureCode(e);
        row.state = code === "cancelled" ? "stopped" : "failed";
        row.error = code === "cancelled" ? null : errorText(code);
      }
    }
    current = null;
    jobId = null;
    running = false;
    stopping = false;
  }

  async function stop() {
    if (!jobId || stopping) return;
    stopping = true;
    await api.cancel(jobId).catch(() => {});
  }

  function remove(row: Row) {
    if (row.state !== "running") rows = rows.filter((r) => r !== row);
  }

  function percentSmaller(o: CompressOutcome): number {
    return o.before_bytes > 0 ? Math.round(((o.before_bytes - o.after_bytes) / o.before_bytes) * 100) : 0;
  }

  onDestroy(() => {
    stopping = true;
    if (jobId) void api.cancel(jobId).catch(() => {});
  });

  onMount(() => {
    let dispose: (() => void) | null = null;
    let gone = false;
    void api
      .onProgress((event) => {
        if (current && event.job_id === jobId && event.total > 0) current.percent = Math.round((event.done / event.total) * 100);
      })
      .then((fn) => (gone ? fn() : (dispose = fn)))
      .catch(() => {});
    return () => {
      gone = true;
      dispose?.();
    };
  });
</script>

{#if rows.length === 0}
  <div class="pv-drop">
    <svg class="pv-drop-art" viewBox="0 0 48 48" width="48" height="48" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M14 8h14l8 8v24a2 2 0 0 1-2 2H14a2 2 0 0 1-2-2V10a2 2 0 0 1 2-2z" />
      <path d="M28 8v8h8" />
      <path d="M24 22v14M19 31l5 5 5-5" />
    </svg>
    <h2 class="pv-drop-title">{$t("pdf.compress_empty_title")}</h2>
    <p class="pv-drop-body">{$t("pdf.compress_empty_body")}</p>
    <button type="button" class="btn btn-primary" disabled={adding} onclick={chooseFiles}>{$t("pdf.choose_pdfs")}</button>
  </div>
{:else}
  <div class="pv-block">
    <h2 class="pv-block-title">{$t("pdf.compress_level")}</h2>
    <div class="pv-presets" role="radiogroup" aria-label={$t("pdf.compress_level") as string}>
      {#each PRESETS as p (p)}
        <button type="button" class="pv-preset" class:active={preset === p} role="radio" aria-checked={preset === p} disabled={running} onclick={() => pickPreset(p)}>
          <span class="pv-preset-name">{$t(`pdf.preset_${p}`)}</span>
          <span class="pv-preset-body">{$t(`pdf.preset_${p}_body`)}</span>
        </button>
      {/each}
    </div>
    <p class="pv-hint">{$t("pdf.compress_hint")}</p>
  </div>

  <ul class="pv-list">
    {#each rows as row (row.file.path)}
      <li class="pv-row">
        <StatusIcon
          name={row.file.error || row.state === "failed" ? "warning" : row.state === "running" ? "running" : row.state === "done" ? "done" : row.state === "same" ? "skipped" : row.state === "stopped" ? "stopped" : "idle"}
        />
        <div class="pv-row-copy">
          <span class="pv-row-name" title={row.file.path}>{row.file.name}</span>
          <span class="pv-row-meta">
            {#if row.file.error}
              {errorText(row.file.error)}
            {:else if row.state === "running"}
              {$t("pdf.compressing", { percent: row.percent })}
            {:else if row.state === "done" && row.outcome}
              {$t("pdf.compress_result", { before: formatBytes(row.outcome.before_bytes), after: formatBytes(row.outcome.after_bytes), percent: percentSmaller(row.outcome) })}
            {:else if row.state === "same"}
              {$t("pdf.compress_same")}
            {:else if row.state === "failed"}
              {row.error}
            {:else if row.state === "stopped"}
              {$t("pdf.stopped")}
            {:else}
              {plural($t, "pdf.page_count", row.file.pages)} · {formatBytes(row.file.size_bytes)}
            {/if}
          </span>
        </div>
        {#if row.state === "done" && row.outcome?.path}
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => row.outcome?.path && void api.reveal(row.outcome.path).catch(() => {})}>{$t("pdf.show_in_folder")}</button>
        {:else if row.state !== "running"}
          <button type="button" class="pv-file-x" disabled={running} aria-label={$t("pdf.remove_file", { name: row.file.name }) as string} onclick={() => remove(row)}>
            <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="m4 4 8 8M12 4l-8 8" /></svg>
          </button>
        {/if}
      </li>
    {/each}
  </ul>

  <footer class="pv-bar">
    <div class="pv-bar-side">
      <button type="button" class="btn btn-secondary" disabled={busy} onclick={chooseFiles}>{$t("pdf.add_pdfs")}</button>
      <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => (rows = [])}>{$t("pdf.clear")}</button>
    </div>
    <div class="pv-bar-main">
      {#if saved > 0 && !running}
        <span class="pv-bar-note">{$t("pdf.compress_saved_total", { size: formatBytes(saved) })}</span>
      {/if}
      {#if running}
        <button type="button" class="btn btn-secondary" disabled={stopping} onclick={stop}>{stopping ? $t("pdf.stopping") : $t("pdf.stop")}</button>
      {:else}
        <button type="button" class="btn btn-primary" disabled={busy || pending.length === 0} onclick={start}>
          {pending.length > 1 ? $t("pdf.compress_many", { count: pending.length }) : $t("pdf.compress_one")}
        </button>
      {/if}
    </div>
  </footer>
{/if}
