<script lang="ts">
  // Transcript of a video, audio or voice message: timestamps that seek
  // (or open the player there) and copy buttons. Runs only when asked.
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "./TgIcon.svelte";
  import { tg3, formatDuration, isUnknownCommand, type ChatRef, type Transcript, type TranscribeProgress } from "./api";
  import { failText } from "./session.svelte";

  let {
    chat,
    messageId,
    current = -1,
    dark = false,
    auto = false,
    onseek,
  }: {
    chat: ChatRef;
    messageId: number;
    /** Playback position, to highlight the segment being spoken. */
    current?: number;
    dark?: boolean;
    /** Start right away instead of waiting for the button. */
    auto?: boolean;
    onseek?: (sec: number) => void;
  } = $props();

  const LANGS = ["", "en", "pt", "es", "fr", "de", "it", "ru", "zh", "ja", "ar", "fa"];

  let lang = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let nativeMissing = $state(false);
  let progress = $state<TranscribeProgress | null>(null);
  let result = $state<Transcript | null>(null);
  let list: HTMLOListElement | undefined = $state();

  let activeIdx = $derived.by(() => {
    if (!result || current < 0) return -1;
    return result.segments.findIndex((s) => current >= s.start && current < (s.end > s.start ? s.end : s.start + 4));
  });

  $effect(() => {
    if (activeIdx < 0 || !list) return;
    list.children[activeIdx]?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  });

  // New item in the player: show its cached transcript or the button again.
  let lastKey = "";
  $effect(() => {
    const key = `${chat.chat_type}:${chat.id}:${messageId}`;
    if (key === lastKey) return;
    lastKey = key;
    result = cached(chat, messageId);
    error = null;
    busy = false;
    progress = null;
    // One made earlier (here or by an agent) shows without asking again.
    if (!result) {
      const c = chat;
      const m = messageId;
      void tg3
        .transcriptGet(c, m)
        .then((r) => {
          if (r && key === lastKey && !result) {
            result = r;
            store.set(key, r);
          }
        })
        .catch(() => {});
    }
  });

  onMount(() => {
    if (auto && !result) {
      // Give the cached lookup a moment before starting real work.
      setTimeout(() => {
        if (!result && !busy) void run(false);
      }, 300);
    }
    let un: (() => void) | undefined;
    let disposed = false;
    listen<TranscribeProgress>("telegram-transcribe-progress", (ev) => {
      const p = ev.payload;
      if (p.chat_id === chat.id && p.message_id === messageId && busy) progress = p;
    })
      .then((f) => {
        if (disposed) f();
        else un = f;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      un?.();
    };
  });

  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

  async function run(native: boolean, force = false) {
    busy = true;
    error = null;
    progress = null;
    const key = `${chat.chat_type}:${chat.id}:${messageId}`;
    try {
      let r: Transcript;
      if (native) {
        // Telegram may still be working on it; ask again a few times.
        let n = await tg3.transcribeNative(chat, messageId);
        for (let i = 0; n.pending && i < 8 && key === lastKey; i++) {
          await wait(4000);
          n = await tg3.transcribeNative(chat, messageId);
        }
        if (n.pending) throw new Error($t("telegram.transcript.native_pending") as string);
        r = n;
      } else {
        r = await tg3.transcribe(chat, messageId, lang || undefined, force);
      }
      if (key !== lastKey) return;
      result = r;
      store.set(key, r);
    } catch (e) {
      if (key !== lastKey) return;
      if (native && isUnknownCommand(e)) nativeMissing = true;
      error = failText(e);
    } finally {
      if (key === lastKey) busy = false;
    }
  }

  async function copy(withTimes: boolean) {
    if (!result) return;
    const text = withTimes && result.segments.length
      ? result.segments.map((s) => `[${formatDuration(s.start)}] ${s.text}`).join("\n")
      : result.text;
    try {
      await writeText(text);
      showToast("success", $t("telegram.common.copied") as string);
    } catch {
      /* clipboard unavailable */
    }
  }
</script>

<script module lang="ts">
  import type { Transcript as T } from "./api";
  // Transcripts already fetched in this session (the backend caches on disk too).
  const store = new Map<string, T>();
  function cached(chat: { id: number; chat_type: string }, messageId: number): T | null {
    return store.get(`${chat.chat_type}:${chat.id}:${messageId}`) ?? null;
  }
</script>

<section class="tr" class:dark aria-label={$t("telegram.transcript.title")}>
  {#if result}
    <header class="tr-head">
      <span class="tr-meta">
        {#if result.lang}{result.lang.toUpperCase()} · {/if}{result.source === "telegram" ? $t("telegram.transcript.by_telegram") : $t("telegram.transcript.by_app")}
      </span>
      {#if result.source !== "telegram"}
        <button class="tr-btn" onclick={() => { result = null; void run(false, true); }} title={$t("telegram.transcript.redo_hint")}><TgIcon name="refresh" size={13} /> {$t("telegram.transcript.redo")}</button>
      {/if}
      <button class="tr-btn" onclick={() => copy(false)} title={$t("telegram.transcript.copy")}><TgIcon name="copy" size={13} /> {$t("telegram.transcript.copy")}</button>
      {#if result.segments.length}
        <button class="tr-btn" onclick={() => copy(true)} title={$t("telegram.transcript.copy_times")}><TgIcon name="clock" size={13} /> {$t("telegram.transcript.copy_times")}</button>
      {/if}
    </header>
    {#if result.segments.length}
      <ol class="tr-list" bind:this={list}>
        {#each result.segments as s, i (i)}
          <li class:cur={i === activeIdx}>
            <button class="tr-time" onclick={() => onseek?.(s.start)} disabled={!onseek} title={onseek ? $t("telegram.transcript.jump") : undefined}>{formatDuration(s.start)}</button>
            <span class="tr-text">{s.text}</span>
          </li>
        {/each}
      </ol>
    {:else if result.text}
      <p class="tr-plain">{result.text}</p>
    {:else}
      <p class="tr-dim">{$t("telegram.transcript.empty")}</p>
    {/if}
  {:else if busy}
    <div class="tr-wait">
      <span class="spinner"></span>
      <span>{$t("telegram.transcript.working")}</span>
      {#if progress && progress.total > 0 && progress.stage === "chunk"}
        <span class="tr-dim">{$t("telegram.transcript.progress", { done: progress.done, total: progress.total })}</span>
      {:else if progress?.stage === "audio"}
        <span class="tr-dim">{$t("telegram.transcript.stage_audio")}</span>
      {:else}
        <span class="tr-dim">{$t("telegram.transcript.working_hint")}</span>
      {/if}
    </div>
  {:else}
    <div class="tr-start">
      <p class="tr-dim">{$t("telegram.transcript.intro")}</p>
      <div class="tr-row">
        <select class="tr-select" bind:value={lang} aria-label={$t("telegram.transcript.lang")}>
          {#each LANGS as l (l)}
            <option value={l}>{l ? l.toUpperCase() : $t("telegram.transcript.lang_auto")}</option>
          {/each}
        </select>
        <button class="tr-btn primary" onclick={() => run(false)}><TgIcon name="captions" size={14} /> {$t("telegram.transcript.run")}</button>
      </div>
      {#if error}
        <p class="tr-err">{error}</p>
        {#if !nativeMissing}
          <button class="tr-btn" onclick={() => run(true)}>{$t("telegram.transcript.try_native")}</button>
          <p class="tr-dim small">{$t("telegram.transcript.native_hint")}</p>
        {/if}
      {/if}
    </div>
  {/if}
</section>

<style>
  .tr {
    --tr-text: var(--text);
    --tr-dim: var(--text-dim);
    --tr-fill: var(--fill-1);
    --tr-fill-2: var(--fill-2, var(--fill-1));
    --tr-cur: var(--accent-soft);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
    color: var(--tr-text);
    font-size: var(--text-sm);
  }

  .tr.dark {
    --tr-text: #f4f5f7;
    --tr-dim: rgba(244, 245, 247, 0.6);
    --tr-fill: rgba(255, 255, 255, 0.08);
    --tr-fill-2: rgba(255, 255, 255, 0.16);
    --tr-cur: rgba(255, 255, 255, 0.12);
  }

  .tr-head {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .tr-meta {
    flex: 1;
    min-width: 0;
    color: var(--tr-dim);
    font-size: var(--text-xs);
  }

  .tr-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radius-full);
    background: var(--tr-fill);
    color: inherit;
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 550;
    cursor: pointer;
    align-self: flex-start;
  }

  .tr-btn:hover {
    background: var(--tr-fill-2);
  }

  .tr-btn.primary {
    background: var(--accent);
    color: var(--on-accent, #fff);
  }

  .tr-btn:focus-visible,
  .tr-time:focus-visible,
  .tr-select:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .tr-list {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .tr-list li {
    display: flex;
    gap: var(--space-2);
    padding: 4px 6px;
    border-radius: var(--radius-sm);
    line-height: 1.45;
  }

  .tr-list li.cur {
    background: var(--tr-cur);
  }

  .tr-time {
    flex-shrink: 0;
    padding: 0;
    border: none;
    background: none;
    font: inherit;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--accent);
    cursor: pointer;
    min-width: 42px;
    text-align: left;
    padding-top: 1px;
  }

  .tr-time:disabled {
    color: var(--tr-dim);
    cursor: default;
  }

  .tr-text {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .tr-plain {
    margin: 0;
    white-space: pre-wrap;
    line-height: 1.5;
    overflow-y: auto;
  }

  .tr-wait,
  .tr-start {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .tr-wait {
    align-items: center;
    text-align: center;
    padding: var(--space-4) 0;
  }

  .tr-row {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }

  .tr-select {
    height: 28px;
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-full);
    background: var(--tr-fill);
    color: inherit;
    font: inherit;
    font-size: var(--text-xs);
  }

  .tr-dim {
    margin: 0;
    color: var(--tr-dim);
    line-height: 1.45;
  }

  .small {
    font-size: var(--text-xs);
  }

  .tr-err {
    margin: 0;
    color: var(--danger);
    font-size: var(--text-xs);
  }
</style>
