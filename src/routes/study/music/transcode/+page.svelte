<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";

  type TranscodeFormat = "m4a" | "opus" | "mp3";

  let inputDir = $state<string>("");
  let outputDir = $state<string>("");
  let format = $state<TranscodeFormat>("m4a");
  let quality = $state<string>("192k");
  let starting = $state(false);
  let jobId = $state<number | null>(null);
  let total = $state(0);
  let current = $state(0);
  let currentFile = $state<string | null>(null);
  let etaSecs = $state<number | null>(null);
  let finished = $state(false);
  let cancelled = $state(false);
  const unlisteners: UnlistenFn[] = [];

  function pct() {
    if (total === 0) return 0;
    return Math.round((current / total) * 100);
  }

  function fmtEta(secs: number | null) {
    if (secs === null || secs <= 0 || !Number.isFinite(secs)) return "—";
    if (secs < 60) return `${Math.round(secs)}s`;
    const m = Math.floor(secs / 60);
    const s = Math.round(secs % 60);
    return `${m}m ${s}s`;
  }

  async function pickInput() {
    const sel = (await open({
      directory: true,
      title: $t("study.music.transcode_pick_input") as string,
    })) as string | null;
    if (sel) inputDir = sel;
  }

  async function pickOutput() {
    const sel = (await open({
      directory: true,
      title: $t("study.music.transcode_pick_output") as string,
    })) as string | null;
    if (sel) outputDir = sel;
  }

  async function startJob() {
    if (!inputDir.trim()) {
      showToast("error", $t("study.music.transcode_missing_input") as string);
      return;
    }
    starting = true;
    finished = false;
    cancelled = false;
    current = 0;
    total = 0;
    currentFile = null;
    etaSecs = null;
    try {
      const res = await pluginInvoke<{ job_id: number; total: number }>(
        "study",
        "study:music:transcode:start",
        {
          inputDir,
          outputFormat: format,
          quality: quality.trim() || "192k",
          outputDir: outputDir.trim() || undefined,
        },
      );
      jobId = res.job_id;
      total = res.total;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    } finally {
      starting = false;
    }
  }

  async function cancelJob() {
    if (jobId === null) return;
    try {
      await pluginInvoke("study", "study:music:transcode:cancel", { jobId });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    }
  }

  onMount(() => {
    void (async () => {
      const onStart = await listen<{ job_id: number; total: number }>(
        "study:music:transcode:start",
        (e) => {
          if (jobId !== null && e.payload?.job_id !== jobId) return;
          if (jobId === null) jobId = e.payload.job_id;
          total = e.payload.total;
        },
      );
      const onProgress = await listen<{
        job_id: number;
        current: number;
        total: number;
        current_file: string;
        eta_secs: number | null;
        ok: boolean;
        error: string | null;
      }>("study:music:transcode:progress", (e) => {
        if (jobId !== null && e.payload?.job_id !== jobId) return;
        current = e.payload.current;
        total = e.payload.total;
        currentFile = e.payload.current_file;
        etaSecs = e.payload.eta_secs;
      });
      const onComplete = await listen<{
        job_id: number;
        cancelled: boolean;
        current: number;
        total: number;
      }>("study:music:transcode:complete", (e) => {
        if (jobId !== null && e.payload?.job_id !== jobId) return;
        finished = true;
        cancelled = e.payload.cancelled;
        const msg = cancelled
          ? ($t("study.music.transcode_cancelled") as string)
          : ($t("study.music.transcode_done", {
              current: e.payload.current,
              total: e.payload.total,
            }) as string);
        showToast(cancelled ? "info" : "success", msg);
      });
      unlisteners.push(onStart, onProgress, onComplete);
    })();
  });

  onDestroy(() => {
    for (const u of unlisteners) u();
  });
</script>

<section class="transcode-page">
  <header class="hero">
    <span class="eyebrow">{$t("study.music.transcode_eyebrow")}</span>
    <h1>{$t("study.music.transcode_title")}</h1>
    <p class="lead">{$t("study.music.transcode_lead")}</p>
  </header>

  <div class="card">
    <div class="row">
      <label class="field">
        <span>{$t("study.music.transcode_input_label")}</span>
        <div class="path-input">
          <input
            type="text"
            bind:value={inputDir}
            placeholder={$t("study.music.transcode_input_placeholder") as string}
            disabled={starting || (jobId !== null && !finished)}
          />
          <button
            type="button"
            class="ghost"
            onclick={pickInput}
            disabled={starting || (jobId !== null && !finished)}
          >
            {$t("study.music.transcode_browse")}
          </button>
        </div>
      </label>
    </div>

    <div class="row">
      <label class="field">
        <span>{$t("study.music.transcode_output_label")}</span>
        <div class="path-input">
          <input
            type="text"
            bind:value={outputDir}
            placeholder={$t("study.music.transcode_output_placeholder") as string}
            disabled={starting || (jobId !== null && !finished)}
          />
          <button
            type="button"
            class="ghost"
            onclick={pickOutput}
            disabled={starting || (jobId !== null && !finished)}
          >
            {$t("study.music.transcode_browse")}
          </button>
        </div>
      </label>
    </div>

    <div class="row two">
      <label class="field">
        <span>{$t("study.music.transcode_format_label")}</span>
        <select
          bind:value={format}
          disabled={starting || (jobId !== null && !finished)}
        >
          <option value="m4a">m4a (AAC)</option>
          <option value="opus">opus</option>
          <option value="mp3">mp3</option>
        </select>
      </label>
      <label class="field">
        <span>{$t("study.music.transcode_quality_label")}</span>
        <input
          type="text"
          bind:value={quality}
          placeholder="192k"
          disabled={starting || (jobId !== null && !finished)}
        />
      </label>
    </div>

    <div class="actions">
      {#if jobId === null || finished}
        <button
          type="button"
          class="primary"
          onclick={startJob}
          disabled={starting || !inputDir.trim()}
        >
          {starting
            ? $t("study.music.transcode_starting")
            : $t("study.music.transcode_start")}
        </button>
      {:else}
        <button type="button" class="danger" onclick={cancelJob}>
          {$t("study.music.transcode_cancel")}
        </button>
      {/if}
    </div>
  </div>

  {#if jobId !== null}
    <div class="progress-card" role="status" aria-live="polite">
      <div class="progress-header">
        <span class="state">
          {#if finished && cancelled}
            {$t("study.music.transcode_state_cancelled")}
          {:else if finished}
            {$t("study.music.transcode_state_done")}
          {:else}
            {$t("study.music.transcode_state_running")}
          {/if}
        </span>
        <span class="count">{current} / {total}</span>
      </div>
      <div class="progress-track">
        <div class="progress-fill" style:width="{pct()}%"></div>
      </div>
      <div class="progress-meta">
        {#if currentFile && !finished}
          <span class="current-file" title={currentFile}>{currentFile}</span>
        {/if}
        {#if !finished}
          <span class="eta">{$t("study.music.transcode_eta")}: {fmtEta(etaSecs)}</span>
        {/if}
      </div>
    </div>
  {/if}
</section>

<style>
  .transcode-page {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 24px 4px;
    max-width: 720px;
  }
  .hero { display: flex; flex-direction: column; gap: 6px; }
  .eyebrow {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--tertiary);
  }
  .hero h1 {
    margin: 0;
    font-size: clamp(22px, 3vw, 32px);
    font-weight: 800;
    color: var(--secondary);
    letter-spacing: -0.01em;
  }
  .hero .lead {
    margin: 0;
    color: var(--tertiary);
    font-size: 13px;
    max-width: 56ch;
  }
  .card {
    background: color-mix(in oklab, var(--surface, var(--button-elevated)) 80%, transparent);
    border: none;
    border-radius: 12px;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .row.two { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .field > span {
    font-size: 12px;
    color: var(--tertiary);
    font-weight: 500;
  }
  .field input[type="text"],
  .field select {
    padding: 8px 10px;
    background: var(--surface, var(--button-elevated));
    border: none;
    border-radius: 6px;
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
  }
  .field input:disabled,
  .field select:disabled { opacity: 0.5; cursor: not-allowed; }
  .path-input { display: grid; grid-template-columns: 1fr auto; gap: 6px; }
  .ghost {
    padding: 8px 14px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .ghost:disabled { opacity: 0.5; cursor: not-allowed; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  .primary {
    padding: 9px 18px;
    background: var(--accent);
    color: var(--on-accent, white);
    border: 0;
    border-radius: 8px;
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }
  .primary:disabled { opacity: 0.4; cursor: not-allowed; }
  .danger {
    padding: 9px 18px;
    background: var(--error);
    color: white;
    border: 0;
    border-radius: 8px;
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }
  .progress-card {
    background: color-mix(in oklab, var(--accent) 5%, transparent);
    border: 1px solid color-mix(in oklab, var(--accent) 35%, transparent);
    border-radius: 12px;
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .progress-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 13px;
  }
  .state { color: var(--secondary); font-weight: 600; }
  .count { color: var(--tertiary); font-variant-numeric: tabular-nums; }
  .progress-track {
    height: 6px;
    border-radius: 6px;
    background: color-mix(in oklab, var(--content-border) 60%, transparent);
    overflow: hidden;
  }
  .progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width 200ms ease;
  }
  .progress-meta {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: 12px;
    color: var(--tertiary);
  }
  .current-file {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .eta { font-variant-numeric: tabular-nums; flex-shrink: 0; }
  @media (prefers-reduced-motion: reduce) {
    .progress-fill { transition: none; }
  }
</style>
