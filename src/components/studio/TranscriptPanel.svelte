<script lang="ts" module>
  export type AsrState = "idle" | "downloading" | "transcribing" | "error";
</script>

<script lang="ts">
  import { t, locale } from "$lib/i18n";
  import { formatBytes } from "$lib/studio/api";
  import {
    editText,
    formatClock,
    insertAfter,
    mergeWithNext,
    parseTimestamp,
    removeAt,
    retime,
    setTiming,
    shiftAll,
    shiftSegment,
    splitAt,
  } from "$lib/studio/subtitles";
  import type { AsrStatus, Segment } from "$lib/studio/types";
  import StatusIcon from "$components/converter/StatusIcon.svelte";

  let {
    segments = $bindable(),
    burnCaptions = $bindable(),
    model = $bindable(),
    lang = $bindable(),
    asr,
    asrState,
    asrPercent,
    asrError,
    hasAudio,
    canBurn,
    durationMs,
    activeIndex,
    locked,
    ondownload,
    ontranscribe,
    onimport,
    onexport,
    onseek,
  }: {
    segments: Segment[];
    burnCaptions: boolean;
    model: string;
    lang: string;
    asr: AsrStatus | null;
    asrState: AsrState;
    asrPercent: number;
    asrError: string | null;
    hasAudio: boolean;
    canBurn: boolean;
    durationMs: number;
    activeIndex: number;
    locked: boolean;
    ondownload: () => void;
    ontranscribe: () => void;
    onimport: () => void;
    onexport: (format: "srt" | "vtt") => void;
    onseek: (ms: number) => void;
  } = $props();

  const LANGS = ["auto", "en", "pt", "es", "fr", "de", "it", "nl", "ru", "uk", "pl", "tr", "ar", "fa", "hi", "ja", "ko", "zh", "el", "lo"];

  const carets = new Map<string, number>();
  let drafts = $state<Record<string, string>>({});

  let selected = $derived(asr?.models.find((m) => m.id === model) ?? null);
  let runtimeBlocked = $derived(!!asr && !asr.runtime_installed && !asr.runtime_downloadable);
  let modelReady = $derived(!!asr && asr.runtime_installed && !!selected?.downloaded);
  let busy = $derived(asrState === "downloading" || asrState === "transcribing");
  let hasText = $derived(segments.some((s) => s.text.trim()));
  let estimated = $derived(segments.some((s) => s.estimated));

  function langName(code: string): string {
    if (code === "auto") return $t("studio.lang_auto") as string;
    try {
      return new Intl.DisplayNames([$locale || "en"], { type: "language" }).of(code) ?? code;
    } catch {
      return code;
    }
  }

  function update(index: number, seg: Segment) {
    segments = segments.map((s, i) => (i === index ? seg : s));
  }

  function commitTime(index: number, which: "start" | "end", raw: string) {
    const seg = segments[index];
    const key = `${seg.id}:${which}`;
    const ms = parseTimestamp(raw);
    const { [key]: _, ...rest } = drafts;
    drafts = rest;
    if (ms === null) return;
    segments = retime(segments, index, which === "start" ? setTiming(seg, ms, Math.max(seg.end_ms, ms + 200)) : setTiming(seg, seg.start_ms, ms));
  }

  function timeKey(e: KeyboardEvent, index: number, which: "start" | "end") {
    if (e.key === "Enter") commitTime(index, which, (e.currentTarget as HTMLInputElement).value);
  }
</script>

<div class="tp">
  {#if !hasAudio && segments.length === 0}
    <div class="tp-state">
      <StatusIcon name="warning" />
      <div>
        <p class="tp-state-title">{$t("studio.no_audio_title")}</p>
        <p class="tp-state-body">{$t("studio.no_audio_body")}</p>
      </div>
    </div>
  {:else if hasAudio}
    <section class="tp-asr" aria-labelledby="tp-asr-title">
      <h3 class="tp-h" id="tp-asr-title">{$t("studio.auto_title")}</h3>
      <div class="tp-asr-row">
        <label class="field tp-field">
          <span class="field-label">{$t("studio.model")}</span>
          <select class="input" bind:value={model} disabled={busy || locked || !asr}>
            {#each asr?.models ?? [] as m (m.id)}
              <option value={m.id}>{m.label}{m.downloaded ? "" : ` · ${$t("studio.model_not_downloaded")}`}</option>
            {/each}
          </select>
        </label>
        <label class="field tp-field">
          <span class="field-label">{$t("studio.language")}</span>
          <select class="input" bind:value={lang} disabled={busy || locked}>
            {#each LANGS as code (code)}
              <option value={code}>{langName(code)}</option>
            {/each}
          </select>
        </label>
      </div>

      {#if !asr}
        <p class="tp-hint"><span class="spinner tp-spin" aria-hidden="true"></span> {$t("studio.asr_checking")}</p>
      {:else if runtimeBlocked}
        <div class="tp-state">
          <StatusIcon name="failed" />
          <div>
            <p class="tp-state-title">{$t("studio.runtime_blocked_title")}</p>
            <p class="tp-state-body">{$t("studio.runtime_blocked_body")}</p>
          </div>
        </div>
      {:else if asrState === "downloading"}
        <div class="tp-progress" role="status" aria-live="polite">
          <span>{$t("studio.downloading_model", { percent: asrPercent })}</span>
          <progress max="100" value={asrPercent}></progress>
        </div>
      {:else if asrState === "transcribing"}
        <div class="tp-progress" role="status" aria-live="polite">
          <span>{$t("studio.transcribing", { percent: asrPercent })}</span>
          <progress max="100" value={asrPercent}></progress>
          <span class="tp-hint">{$t("studio.transcribing_hint")}</span>
        </div>
      {:else if !modelReady}
        <div class="tp-state">
          <StatusIcon name="idle" />
          <div class="tp-state-copy">
            <p class="tp-state-title">{$t("studio.model_missing_title")}</p>
            <p class="tp-state-body">{$t("studio.model_missing_body", { size: formatBytes(selected?.bytes ?? 0) })}</p>
            <button type="button" class="btn btn-tinted btn-sm" disabled={locked} onclick={ondownload}>
              {$t("studio.download_model", { size: formatBytes(selected?.bytes ?? 0) })}
            </button>
          </div>
        </div>
      {:else}
        <div class="tp-go">
          <button type="button" class="btn btn-tinted" disabled={locked} onclick={ontranscribe}>
            {segments.length ? $t("studio.transcribe_again") : $t("studio.transcribe")}
          </button>
          <span class="tp-hint">{$t("studio.transcribe_hint")}</span>
        </div>
      {/if}

      {#if asrError && asrState === "error"}
        <div class="tp-state tp-error" role="alert">
          <StatusIcon name="failed" />
          <div>
            <p class="tp-state-title">{$t("studio.asr_failed")}</p>
            <p class="tp-state-body">{asrError}</p>
          </div>
        </div>
      {/if}
    </section>
  {/if}

  <section class="tp-list-wrap" aria-labelledby="tp-list-title">
    <div class="tp-toolbar">
      <h3 class="tp-h" id="tp-list-title">{$t("studio.transcript")}</h3>
      <div class="tp-tools">
        <button type="button" class="btn btn-ghost btn-sm" disabled={locked} onclick={onimport}>{$t("studio.import_subs")}</button>
        <button type="button" class="btn btn-ghost btn-sm" disabled={!hasText} onclick={() => onexport("srt")}>{$t("studio.export_srt")}</button>
        <button type="button" class="btn btn-ghost btn-sm" disabled={!hasText} onclick={() => onexport("vtt")}>{$t("studio.export_vtt")}</button>
      </div>
    </div>

    {#if segments.length === 0}
      <div class="tp-empty">
        <p>{$t("studio.transcript_empty")}</p>
        <button type="button" class="btn btn-secondary btn-sm" disabled={locked} onclick={() => (segments = insertAfter([], -1, durationMs))}>{$t("studio.add_line")}</button>
      </div>
    {:else}
      <div class="tp-bulk">
        <label class="tp-check" class:disabled={!canBurn}>
          <input type="checkbox" class="checkbox" bind:checked={burnCaptions} disabled={!canBurn || locked} />
          <span>{$t("studio.burn_captions")}</span>
        </label>
        <span class="tp-shift" role="group" aria-label={$t("studio.shift_all")}>
          <span class="tp-hint">{$t("studio.shift_all")}</span>
          <button type="button" class="btn btn-ghost btn-sm" disabled={locked} onclick={() => (segments = shiftAll(segments, -100))}>{$t("studio.minus_tenth")}</button>
          <button type="button" class="btn btn-ghost btn-sm" disabled={locked} onclick={() => (segments = shiftAll(segments, 100))}>{$t("studio.plus_tenth")}</button>
        </span>
      </div>
      {#if !canBurn}
        <p class="tp-hint">{$t("studio.burn_unavailable")}</p>
      {/if}
      {#if estimated}
        <p class="tp-hint">{$t("studio.words_estimated")}</p>
      {/if}

      <ol class="tp-list">
        {#each segments as seg, i (seg.id)}
          <li class="tp-row" class:active={i === activeIndex}>
            <div class="tp-times">
              <button type="button" class="tp-jump" onclick={() => onseek(seg.start_ms)} aria-label={$t("studio.jump_to", { time: formatClock(seg.start_ms) })}>
                <svg viewBox="0 0 16 16" width="10" height="10" fill="currentColor" aria-hidden="true"><path d="M5 3.2v9.6a.6.6 0 0 0 .9.5l7.6-4.8a.6.6 0 0 0 0-1L5.9 2.7a.6.6 0 0 0-.9.5z" /></svg>
              </button>
              <input
                class="input tp-time"
                aria-label={$t("studio.start_time")}
                value={drafts[`${seg.id}:start`] ?? formatClock(seg.start_ms)}
                disabled={locked}
                oninput={(e) => (drafts = { ...drafts, [`${seg.id}:start`]: (e.currentTarget as HTMLInputElement).value })}
                onblur={(e) => commitTime(i, "start", (e.currentTarget as HTMLInputElement).value)}
                onkeydown={(e) => timeKey(e, i, "start")}
              />
              <span class="tp-dash" aria-hidden="true">–</span>
              <input
                class="input tp-time"
                aria-label={$t("studio.end_time")}
                value={drafts[`${seg.id}:end`] ?? formatClock(seg.end_ms)}
                disabled={locked}
                oninput={(e) => (drafts = { ...drafts, [`${seg.id}:end`]: (e.currentTarget as HTMLInputElement).value })}
                onblur={(e) => commitTime(i, "end", (e.currentTarget as HTMLInputElement).value)}
                onkeydown={(e) => timeKey(e, i, "end")}
              />
              <span class="tp-nudge">
                <button type="button" class="btn btn-ghost btn-sm" disabled={locked} aria-label={$t("studio.nudge_earlier")} onclick={() => (segments = retime(segments, i, shiftSegment(seg, -100)))}>{$t("studio.minus_tenth")}</button>
                <button type="button" class="btn btn-ghost btn-sm" disabled={locked} aria-label={$t("studio.nudge_later")} onclick={() => (segments = retime(segments, i, shiftSegment(seg, 100)))}>{$t("studio.plus_tenth")}</button>
              </span>
            </div>
            <textarea
              class="input tp-text"
              rows="2"
              aria-label={$t("studio.line_text", { n: i + 1 })}
              value={seg.text}
              disabled={locked}
              oninput={(e) => {
                const el = e.currentTarget as HTMLTextAreaElement;
                carets.set(seg.id, el.selectionStart);
                update(i, editText(seg, el.value));
              }}
              onselect={(e) => carets.set(seg.id, (e.currentTarget as HTMLTextAreaElement).selectionStart)}
              onclick={(e) => carets.set(seg.id, (e.currentTarget as HTMLTextAreaElement).selectionStart)}
              onkeyup={(e) => carets.set(seg.id, (e.currentTarget as HTMLTextAreaElement).selectionStart)}
            ></textarea>
            <div class="tp-actions">
              <button type="button" class="btn btn-ghost btn-sm" disabled={locked || seg.words.length < 2} title={seg.words.length < 2 ? $t("studio.split_needs_words") : undefined} onclick={() => (segments = splitAt(segments, i, carets.get(seg.id)))}>{$t("studio.split")}</button>
              <button type="button" class="btn btn-ghost btn-sm" disabled={locked || i === segments.length - 1} onclick={() => (segments = mergeWithNext(segments, i))}>{$t("studio.merge_next")}</button>
              <button type="button" class="btn btn-ghost btn-sm" disabled={locked} onclick={() => (segments = insertAfter(segments, i, durationMs))}>{$t("studio.add_after")}</button>
              <button type="button" class="btn btn-ghost btn-sm tp-del" disabled={locked} onclick={() => (segments = removeAt(segments, i))}>{$t("studio.delete_line")}</button>
            </div>
          </li>
        {/each}
      </ol>
    {/if}
  </section>
</div>

<style>
  .tp {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .tp-h {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 650;
    color: var(--text);
  }

  .tp-asr {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .tp-asr-row {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
    gap: var(--space-3);
  }

  .tp-field select {
    width: 100%;
  }

  .tp-state {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .tp-state-copy {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
  }

  .tp-state-title {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .tp-state-body {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }

  .tp-error :global(.cv-icon) {
    color: var(--danger);
  }

  .tp-progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }

  .tp-progress progress {
    width: 100%;
    accent-color: var(--accent);
  }

  .tp-go {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .tp-hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .tp-spin {
    display: inline-block;
    width: 10px;
    height: 10px;
    vertical-align: -1px;
  }

  .tp-list-wrap {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .tp-toolbar,
  .tp-bulk {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .tp-tools,
  .tp-shift {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .tp-check {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .tp-check.disabled {
    color: var(--text-dim);
  }

  .tp-empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-4);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .tp-empty p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .tp-list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .tp-row {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-3);
    border-top: var(--hairline) solid var(--content-border);
  }

  .tp-row:first-child {
    border-top: none;
  }

  .tp-row.active {
    background: var(--accent-soft);
  }

  .tp-times {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .tp-jump {
    display: inline-grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: var(--radius-full);
    background: var(--control-bg);
    color: var(--text-muted);
    cursor: pointer;
  }

  .tp-jump:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 1px;
  }

  .tp-time {
    width: 64px;
    height: 24px;
    font-size: var(--text-xs);
    font-family: var(--font-mono);
    text-align: center;
  }

  .tp-dash {
    color: var(--text-dim);
  }

  .tp-nudge {
    display: inline-flex;
    margin-left: auto;
  }

  .tp-text {
    min-height: 44px;
    font-size: var(--text-sm);
  }

  .tp-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
  }

  .tp-del {
    margin-left: auto;
  }
</style>
