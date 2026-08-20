<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { writeText as writeClipboard } from "@tauri-apps/plugin-clipboard-manager";
  import { onDestroy, onMount } from "svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";

  type ModelSize = "tiny" | "base" | "small" | "medium" | "large-v3";
  type Language = "auto" | "id" | "en" | "es" | "pt" | "fr" | "it" | "ru" | "ja" | "zh" | "el";

  let videoPath = $state("");
  let modelSize = $state<ModelSize>("small");
  let language = $state<Language>("auto");
  let outputPath = $state("");

  let transcribing = $state(false);
  let progressStage = $state("");
  let progressPercent = $state(0);
  let progressCurrent = $state(0);
  let progressTotal = $state(0);
  let transcriptText = $state("");
  let detectedLanguage = $state("");
  let duration = $state(0);
  let errorMsg = $state("");

  let whisperAvailable = $state<boolean | null>(null);

  // Markdown converter
  let mdInput = $state("");
  let mdOutput = $state("");
  let mdTitle = $state("Transcript");
  let mdParagraphSize = $state(4);

  let jobId = "";
  let unlistenFn: UnlistenFn | null = null;

  const MODEL_OPTIONS: { value: ModelSize; label: string; sizeMB: string }[] = [
    { value: "tiny", label: "Tiny", sizeMB: "~40 MB" },
    { value: "base", label: "Base", sizeMB: "~75 MB" },
    { value: "small", label: "Small (recommended)", sizeMB: "~250 MB" },
    { value: "medium", label: "Medium", sizeMB: "~770 MB" },
    { value: "large-v3", label: "Large v3", sizeMB: "~1.5 GB" },
  ];

  const LANG_OPTIONS: { value: Language; label: string }[] = [
    { value: "auto", label: "Auto-detect" },
    { value: "id", label: "Bahasa Indonesia" },
    { value: "en", label: "English" },
    { value: "es", label: "Español" },
    { value: "pt", label: "Português" },
    { value: "fr", label: "Français" },
    { value: "it", label: "Italiano" },
    { value: "ru", label: "Русский" },
    { value: "ja", label: "日本語" },
    { value: "zh", label: "中文" },
    { value: "el", label: "Ελληνικά" },
  ];

  onMount(async () => {
    try {
      whisperAvailable = await invoke<boolean>("check_whisper_installed");
    } catch {
      whisperAvailable = false;
    }
  });

  onDestroy(() => {
    unlistenFn?.();
  });

  async function pickVideo() {
    const selected = await openDialog({
      multiple: false,
      title: $t("transcript.pick_video"),
      filters: [
        {
          name: "Media",
          extensions: ["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v", "mp3", "wav", "flac", "m4a", "ogg", "opus", "aac"],
        },
      ],
    });
    if (typeof selected === "string" && selected) {
      videoPath = selected;
      // Suggest default output path
      const dot = selected.lastIndexOf(".");
      outputPath = (dot > 0 ? selected.slice(0, dot) : selected) + ".txt";
    }
  }

  async function pickOutput() {
    const selected = await saveDialog({
      title: $t("transcript.save_transcript"),
      defaultPath: outputPath || undefined,
      filters: [{ name: "Text", extensions: ["txt"] }],
    });
    if (selected) outputPath = selected;
  }

  async function startTranscribe() {
    if (!videoPath || !outputPath) return;
    if (whisperAvailable === false) {
      errorMsg = $t("transcript.whisper_missing_error");
      return;
    }
    if (transcribing) return;
    transcribing = true;
    progressStage = "loading_model";
    progressPercent = 0;
    transcriptText = "";
    errorMsg = "";
    detectedLanguage = "";
    duration = 0;

    jobId = `job-${Date.now()}`;
    const eventName = `transcript:${jobId}`;

    unlistenFn?.();
    unlistenFn = await listen<any>(eventName, (event) => {
      const p = event.payload;
      if (!p || !p.stage) return;
      progressStage = p.stage;
      if (p.stage === "progress") {
        progressPercent = p.percent ?? 0;
        progressCurrent = p.current ?? 0;
        progressTotal = p.total ?? 0;
      } else if (p.stage === "error") {
        errorMsg = p.message ?? $t("transcript.unknown_error");
      }
    });

    try {
      const result = await invoke<{
        language: string;
        duration: number;
        text: string;
        output_path: string;
      }>("transcribe_video", {
        videoPath,
        outputPath,
        modelSize,
        language,
        jobId,
      });
      transcriptText = result.text;
      detectedLanguage = result.language;
      duration = result.duration;
      mdInput = result.text;
      showToast("success", $t("transcript.done_toast", { path: result.output_path }));
    } catch (e: any) {
      errorMsg = typeof e === "string" ? e : (e?.message ?? $t("transcript.unknown_error"));
    } finally {
      transcribing = false;
      unlistenFn?.();
      unlistenFn = null;
      progressStage = "";
    }
  }

  async function copyTranscript() {
    if (!transcriptText) return;
    try {
      await writeClipboard(transcriptText);
      showToast("success", $t("transcript.copied"));
    } catch {}
  }

  // === Markdown conversion ===
  function transcriptToMarkdown(text: string, title: string, paraSize: number): string {
    const trimmed = text.trim();
    if (!trimmed) return "";
    const lines = trimmed.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
    const parts: string[] = [`# ${title}`, ""];
    const step = Math.max(1, paraSize);
    for (let i = 0; i < lines.length; i += step) {
      const chunk = lines.slice(i, i + step).join(" ");
      parts.push(chunk);
      parts.push("");
    }
    return parts.join("\n").trim() + "\n";
  }

  function generateMarkdown() {
    if (!mdInput.trim()) {
      showToast("info", $t("transcript.md_input_empty"));
      return;
    }
    mdOutput = transcriptToMarkdown(mdInput, mdTitle || "Transcript", mdParagraphSize);
  }

  async function copyMarkdown() {
    if (!mdOutput) return;
    try {
      await writeClipboard(mdOutput);
      showToast("success", $t("transcript.md_copied"));
    } catch {}
  }

  async function saveMarkdown() {
    if (!mdOutput) return;
    const path = await saveDialog({
      title: $t("transcript.save_markdown"),
      defaultPath: `${mdTitle || "transcript"}.md`,
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    if (!path) return;
    try {
      await invoke("write_text_file", { path, content: mdOutput });
      showToast("success", $t("transcript.md_saved", { path }));
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? $t("transcript.unknown_error")));
    }
  }

  function useTranscriptForMd() {
    if (transcriptText) mdInput = transcriptText;
  }

  function formatTime(sec: number): string {
    if (!isFinite(sec)) return "--:--";
    const s = Math.floor(sec % 60);
    const m = Math.floor((sec / 60) % 60);
    const h = Math.floor(sec / 3600);
    if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
    return `${m}:${String(s).padStart(2, "0")}`;
  }
</script>

<div class="page">
  <header class="header">
    <h1 class="title">{$t("transcript.title")}</h1>
    <p class="subtitle">{$t("transcript.subtitle")}</p>
  </header>

  {#if whisperAvailable === false}
    <div class="banner-warn">
      <strong>{$t("transcript.whisper_missing_title")}</strong>
      <p>{$t("transcript.whisper_missing_desc")}</p>
      <code class="install-cmd">pip install --user faster-whisper</code>
    </div>
  {/if}

  <section class="card">
    <h2 class="section-title">{$t("transcript.section_video_to_text")}</h2>

    <div class="field-row">
      <label class="field-label" for="video-input">{$t("transcript.video_file")}</label>
      <div class="input-group">
        <input
          id="video-input"
          class="input"
          type="text"
          readonly
          value={videoPath}
          placeholder={$t("transcript.no_video_picked")}
        />
        <button class="button" onclick={pickVideo} disabled={transcribing}>
          {$t("transcript.browse")}
        </button>
      </div>
    </div>

    <div class="field-row-2col">
      <div>
        <label class="field-label" for="model-select">{$t("transcript.model_size")}</label>
        <select id="model-select" class="input" bind:value={modelSize} disabled={transcribing}>
          {#each MODEL_OPTIONS as opt}
            <option value={opt.value}>{opt.label} — {opt.sizeMB}</option>
          {/each}
        </select>
      </div>
      <div>
        <label class="field-label" for="lang-select">{$t("transcript.language")}</label>
        <select id="lang-select" class="input" bind:value={language} disabled={transcribing}>
          {#each LANG_OPTIONS as opt}
            <option value={opt.value}>{opt.label}</option>
          {/each}
        </select>
      </div>
    </div>

    <div class="field-row">
      <label class="field-label" for="output-input">{$t("transcript.output_path")}</label>
      <div class="input-group">
        <input
          id="output-input"
          class="input"
          type="text"
          bind:value={outputPath}
          placeholder={$t("transcript.no_output_picked")}
        />
        <button class="button" onclick={pickOutput} disabled={transcribing}>
          {$t("transcript.browse")}
        </button>
      </div>
    </div>

    <div class="action-row">
      <button
        class="button primary"
        onclick={startTranscribe}
        disabled={transcribing || !videoPath || !outputPath || whisperAvailable === false}
      >
        {#if transcribing}
          <span class="spinner small"></span>
          {$t("transcript.transcribing")}
        {:else}
          {$t("transcript.start")}
        {/if}
      </button>
    </div>

    {#if transcribing || progressStage}
      <div class="progress-section">
        <div class="progress-label">
          {#if progressStage === "loading_model"}
            {$t("transcript.stage_loading_model")}
          {:else if progressStage === "transcribing"}
            {$t("transcript.stage_starting")}
          {:else if progressStage === "progress"}
            {progressPercent}% · {formatTime(progressCurrent)} / {formatTime(progressTotal)}
          {:else if progressStage === "done"}
            {$t("transcript.stage_done")}
          {/if}
        </div>
        <div class="progress-bar-outer">
          <div
            class="progress-bar-inner"
            class:indeterminate={progressStage !== "progress"}
            style:width={progressStage === "progress" ? `${progressPercent}%` : "40%"}
          ></div>
        </div>
      </div>
    {/if}

    {#if errorMsg}
      <div class="error-box">
        {errorMsg}
      </div>
    {/if}

    {#if transcriptText}
      <div class="result-meta">
        <span class="subtext">
          {$t("transcript.detected_lang", { lang: detectedLanguage })} · {formatTime(duration)}
        </span>
        <div class="result-actions">
          <button class="button" onclick={copyTranscript}>
            {$t("transcript.copy")}
          </button>
          <button class="button" onclick={useTranscriptForMd}>
            {$t("transcript.convert_to_md")}
          </button>
        </div>
      </div>
      <textarea class="transcript-textarea" bind:value={transcriptText} rows="12"></textarea>
    {/if}
  </section>

  <section class="card">
    <h2 class="section-title">{$t("transcript.section_txt_to_md")}</h2>
    <p class="section-desc">{$t("transcript.md_desc")}</p>

    <div class="field-row">
      <label class="field-label" for="md-title">{$t("transcript.md_title_label")}</label>
      <input id="md-title" class="input" type="text" bind:value={mdTitle} placeholder="Transcript" />
    </div>

    <div class="field-row">
      <label class="field-label" for="md-para">{$t("transcript.md_paragraph_size")}</label>
      <input
        id="md-para"
        class="input"
        type="number"
        min="1"
        max="20"
        bind:value={mdParagraphSize}
      />
    </div>

    <div class="field-row">
      <label class="field-label" for="md-input">{$t("transcript.md_input")}</label>
      <textarea
        id="md-input"
        class="transcript-textarea"
        bind:value={mdInput}
        rows="8"
        placeholder={$t("transcript.md_input_placeholder")}
      ></textarea>
    </div>

    <div class="action-row">
      <button class="button primary" onclick={generateMarkdown} disabled={!mdInput.trim()}>
        {$t("transcript.md_generate")}
      </button>
    </div>

    {#if mdOutput}
      <div class="result-meta">
        <span class="subtext">{$t("transcript.md_output_label")}</span>
        <div class="result-actions">
          <button class="button" onclick={copyMarkdown}>{$t("transcript.copy")}</button>
          <button class="button primary" onclick={saveMarkdown}>{$t("transcript.md_save")}</button>
        </div>
      </div>
      <textarea class="transcript-textarea" bind:value={mdOutput} rows="12"></textarea>
    {/if}
  </section>
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: var(--padding);
    max-width: 900px;
    margin: 0 auto;
    width: 100%;
  }

  .header {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .title {
    font-size: var(--text-lg);
    font-weight: 600;
    margin: 0;
  }

  .subtitle {
    color: var(--gray);
    font-size: var(--text-sm);
    margin: 0;
  }

  .banner-warn {
    background: color-mix(in oklab, var(--gold, #f59e0b) 15%, transparent);
    border: 1px solid color-mix(in oklab, var(--gold, #f59e0b) 40%, transparent);
    padding: 12px;
    border-radius: var(--radius);
    color: var(--primary);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .banner-warn strong { font-size: 14px; }
  .banner-warn p { margin: 0; color: var(--secondary); font-size: 13px; }

  .install-cmd {
    display: inline-block;
    background: var(--button-elevated);
    padding: 4px 8px;
    border-radius: 4px;
    font-family: var(--font-mono, monospace);
    font-size: 12px;
    color: var(--primary);
    user-select: all;
    width: fit-content;
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: var(--padding);
    background: var(--card);
    border-radius: var(--radius);
  }

  .section-title {
    font-size: 16px;
    font-weight: 600;
    margin: 0;
  }

  .section-desc {
    margin: 0;
    color: var(--gray);
    font-size: 13px;
  }

  .field-row, .field-row-2col {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .field-row-2col {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .field-label {
    font-size: 12px;
    color: var(--secondary);
    font-weight: 500;
  }

  .input-group {
    display: flex;
    gap: 8px;
  }

  .input-group .input { flex: 1; }

  .action-row {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }

  .progress-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .progress-label {
    font-size: 12px;
    color: var(--secondary);
    font-variant-numeric: tabular-nums;
  }

  .progress-bar-outer {
    position: relative;
    height: 4px;
    background: var(--button-elevated);
    border-radius: 100px;
    overflow: hidden;
  }

  .progress-bar-inner {
    height: 100%;
    background: var(--blue);
    border-radius: 100px;
    transition: width 200ms ease-out;
  }

  .progress-bar-inner.indeterminate {
    position: absolute;
    left: 0;
    animation: prog-slide 1.2s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }

  @keyframes prog-slide {
    0% { transform: translateX(-100%); }
    100% { transform: translateX(250%); }
  }

  .error-box {
    background: color-mix(in oklab, var(--red) 12%, transparent);
    color: var(--red);
    padding: 10px 12px;
    border-radius: 6px;
    font-size: 13px;
    white-space: pre-wrap;
  }

  .result-meta {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .subtext {
    color: var(--gray);
    font-size: 13px;
  }

  .result-actions {
    display: flex;
    gap: 6px;
  }

  .transcript-textarea {
    width: 100%;
    min-height: 200px;
    padding: 10px 12px;
    background: var(--input-bg);
    color: var(--primary);
    border: 1px solid var(--input-border);
    border-radius: 6px;
    font-family: var(--font-mono, monospace);
    font-size: 12.5px;
    line-height: 1.5;
    resize: vertical;
  }

  .spinner.small {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 1.5px solid var(--button-elevated);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 800ms linear infinite;
    vertical-align: -2px;
    margin-right: 4px;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
