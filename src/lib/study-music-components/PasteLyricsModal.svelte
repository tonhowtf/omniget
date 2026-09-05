<script lang="ts">
  import { lyricsStore } from "$lib/study-music/lyrics-store.svelte";
  import { t } from "$lib/i18n";

  type Props = {
    open: boolean;
    trackId: number | null;
    onclose: () => void;
    onsaved?: () => void;
  };

  let { open, trackId, onclose, onsaved }: Props = $props();

  let text = $state("");
  let saving = $state(false);
  let invalid = $state(false);
  let textareaEl = $state<HTMLTextAreaElement | null>(null);

  const TIMESTAMP_RE = /\[\d{1,2}:\d{2}(?:[.:]\d{1,3})?\]/;

  $effect(() => {
    if (open) {
      text = "";
      invalid = false;
      saving = false;
      requestAnimationFrame(() => {
        textareaEl?.focus();
      });
    }
  });

  function validateClientSide(value: string): boolean {
    if (!value || value.trim().length === 0) return false;
    return TIMESTAMP_RE.test(value);
  }

  async function save() {
    if (trackId === null) return;
    if (!validateClientSide(text)) {
      invalid = true;
      return;
    }
    invalid = false;
    saving = true;
    const ok = await lyricsStore.setLocal(trackId, text);
    saving = false;
    if (ok) {
      onsaved?.();
      onclose();
    } else {
      invalid = true;
    }
  }

  function cancel() {
    if (saving) return;
    onclose();
  }

  function handleKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      cancel();
    }
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void save();
    }
  }
</script>

{#if open}
  <div
    class="overlay"
    role="presentation"
    onclick={cancel}
  >
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="paste-lyrics-title"
      onkeydown={handleKey}
      onclick={(e) => e.stopPropagation()}
    >
      <header class="head">
        <h3 id="paste-lyrics-title">{$t("study.music.lyrics_paste_title")}</h3>
        <button
          type="button"
          class="ico"
          onclick={cancel}
          aria-label={$t("study.common.close") as string}
        >×</button>
      </header>

      <p class="hint">{$t("study.music.lyrics_paste_hint")}</p>

      <textarea
        bind:this={textareaEl}
        bind:value={text}
        placeholder={$t("study.music.lyrics_paste_placeholder") as string}
        rows="14"
        spellcheck="false"
        autocomplete="off"
        class:invalid
      ></textarea>

      {#if invalid}
        <p class="error">{$t("study.music.lyrics_paste_invalid")}</p>
      {/if}

      <footer class="foot">
        <button type="button" class="ghost" onclick={cancel} disabled={saving}>
          {$t("study.music.lyrics_paste_cancel")}
        </button>
        <button
          type="button"
          class="primary"
          onclick={save}
          disabled={saving || text.trim().length === 0}
        >
          {saving ? "…" : $t("study.music.lyrics_paste_save")}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    backdrop-filter: blur(4px);
    z-index: 200;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }
  .modal {
    width: min(560px, 100%);
    background: rgb(24, 24, 24);
    color: rgba(255, 255, 255, 0.92);
    border: none;
    border-radius: 16px;
    box-shadow: 0 32px 80px rgba(0, 0, 0, 0.7);
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px 20px 16px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .head h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 700;
  }
  .ico {
    width: 28px;
    height: 28px;
    padding: 0;
    background: transparent;
    border: 0;
    border-radius: 50%;
    color: rgba(255, 255, 255, 0.5);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
  }
  .ico:hover { color: white; background: rgba(255, 255, 255, 0.08); }
  .hint {
    margin: 0;
    font-size: 12px;
    color: rgba(255, 255, 255, 0.55);
    line-height: 1.5;
  }
  textarea {
    width: 100%;
    resize: vertical;
    min-height: 220px;
    padding: 12px 14px;
    background: rgb(14, 14, 14);
    color: rgba(255, 255, 255, 0.95);
    border: none;
    border-radius: 10px;
    font-family: ui-monospace, "Cascadia Mono", Menlo, Consolas, monospace;
    font-size: 13px;
    line-height: 1.55;
    box-sizing: border-box;
    transition: border-color 160ms ease, box-shadow 160ms ease;
  }
  textarea:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px rgba(124, 92, 255, 0.18);
  }
  textarea.invalid {
    border-color: rgb(255, 107, 107);
    box-shadow: 0 0 0 3px rgba(255, 107, 107, 0.18);
  }
  .error {
    margin: 0;
    font-size: 12px;
    color: rgb(255, 138, 138);
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding-top: 6px;
  }
  .ghost,
  .primary {
    padding: 9px 16px;
    border-radius: 999px;
    border: 0;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: filter 160ms ease, transform 160ms ease, opacity 160ms ease;
  }
  .ghost {
    background: transparent;
    color: rgba(255, 255, 255, 0.7);
    border: none;
  }
  .ghost:hover:not(:disabled) {
    color: white;
    border-color: rgba(255, 255, 255, 0.28);
  }
  .primary {
    background: var(--accent);
    color: var(--on-accent);
  }
  .primary:hover:not(:disabled) { filter: brightness(1.1); }
  .primary:active:not(:disabled) { transform: scale(0.98); }
  .primary:disabled,
  .ghost:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
