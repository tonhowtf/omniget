<script lang="ts">
  import { onMount } from "svelte";
import { t } from "$lib/i18n";
  import { notesLessonsLink } from "$lib/notes-bridge";

  type Props = {
    lessonId: number;
    getCurrentTime: () => number;
    onClose: () => void;
  };

  let { lessonId, getCurrentTime, onClose }: Props = $props();

  let body = $state("");
  let saving = $state(false);
  let savedAt = $state<number | null>(null);
  let textareaEl = $state<HTMLTextAreaElement | undefined>(undefined);
  let pendingTimestamp = $state(0);

  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  function fmtTimestamp(secs: number): string {
    const m = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  function captureTimestamp() {
    pendingTimestamp = Math.max(0, Math.floor(getCurrentTime()));
  }

  async function flushSave() {
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
    }
    const text = body.trim();
    if (!text) return;
    saving = true;
    try {
      await notesLessonsLink({
        lessonId,
        timestampSeconds: pendingTimestamp,
        bodyMd: text,
      });
      savedAt = Date.now();
      body = "";
    } catch (e) {
      console.error("annotate save failed", e);
    } finally {
      saving = false;
    }
  }

  function scheduleSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      void flushSave();
    }, 500);
  }

  function onInput() {
    if (body.length === 1) captureTimestamp();
    scheduleSave();
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      void flushSave();
      onClose();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      captureTimestamp();
      void flushSave();
    }
  }

  onMount(() => {
    captureTimestamp();
    queueMicrotask(() => textareaEl?.focus());
  });
</script>

<div class="annotate-overlay" role="dialog" aria-label={$t("study.notes.nb.annotate_aria")}>
  <header class="head">
    <div class="title-block">
      <strong>Anotar momento</strong>
      <span class="ts">@ {fmtTimestamp(pendingTimestamp)}</span>
    </div>
    <button
      type="button"
      class="close-btn"
      aria-label="Fechar"
      title="Fechar (Esc)"
      onclick={() => {
        void flushSave();
        onClose();
      }}
    >×</button>
  </header>
  <textarea
    bind:this={textareaEl}
    class="body"
    placeholder="Anote algo sobre este momento da aula… (auto-salva, Esc fecha)"
    bind:value={body}
    oninput={onInput}
    onkeydown={onKeyDown}
    rows="10"
  ></textarea>
  <footer class="foot">
    {#if saving}
      <span class="state">salvando…</span>
    {:else if savedAt}
      <span class="state subtle">salvo</span>
    {:else}
      <span class="state subtle">timestamp captura no 1º caractere</span>
    {/if}
    <button
      type="button"
      class="capture-btn"
      onclick={captureTimestamp}
      title="Recaptura timestamp atual"
    >↻ timestamp</button>
  </footer>
</div>

<style>
  .annotate-overlay {
    position: fixed;
    top: 80px;
    right: 16px;
    width: 320px;
    max-height: 60vh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    box-shadow: 0 12px 32px color-mix(in oklab, black 32%, transparent);
    z-index: 95;
    overflow: hidden;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    border-bottom: none;
  }
  .title-block {
    display: flex;
    flex-direction: column;
  }
  .title-block strong {
    font-size: 13px;
    color: var(--text);
  }
  .ts {
    font-size: 11px;
    color: var(--accent);
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .close-btn {
    width: 24px;
    height: 24px;
    border: 0;
    border-radius: var(--border-radius);
    background: transparent;
    color: var(--tertiary);
    font: inherit;
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
  }
  .close-btn:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
    color: var(--text);
  }
  .body {
    flex: 1 1 auto;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 13px;
    line-height: 1.55;
    padding: 12px;
    resize: none;
    outline: none;
    min-height: 200px;
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 12px;
    border-top: none;
    font-size: 11px;
  }
  .state {
    color: var(--tertiary);
  }
  .state.subtle {
    opacity: 0.7;
  }
  .capture-btn {
    border: 0;
    background: transparent;
    color: var(--accent);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: var(--border-radius);
  }
  .capture-btn:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
</style>
