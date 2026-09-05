<script lang="ts">
  import { t } from "$lib/i18n";
  import ContextHint from "$components/hints/ContextHint.svelte";
  import { shortcut } from "$lib/platform";

  let { url = $bindable(""), onInput, prominent = false }: { url?: string; onInput?: () => void; prominent?: boolean } = $props();

  let dragOver = $state(false);
  let inputEl = $state<HTMLInputElement | null>(null);

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    if (e.dataTransfer) {
      e.dataTransfer.dropEffect = "copy";
    }
    dragOver = true;
  }

  function handleDragLeave() {
    dragOver = false;
  }

  function handleDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;

    if (e.dataTransfer?.files?.length) {
      const file = e.dataTransfer.files[0];
      if (file.name.endsWith(".torrent")) {
        url = (file as any).path || file.name;
        onInput?.();
        return;
      }
    }

    const text = e.dataTransfer?.getData("text/plain");
    if (text) {
      url = text.trim();
      onInput?.();
    }
  }

  function focusInput() {
    inputEl?.focus();
  }
</script>

<!-- A <label> so a click anywhere on the field focuses the input, like a native search field. -->
<label
  class="omnibox-wrapper"
  class:drag-over={dragOver}
  class:prominent
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
>
  <svg class="omnibox-glyph" viewBox="0 0 20 20" width={prominent ? 17 : 14} height={prominent ? 17 : 14} fill="currentColor" aria-hidden="true">
    <path d="M8.5 2.5a6 6 0 1 0 3.67 10.74l3.8 3.79a1 1 0 0 0 1.41-1.41l-3.79-3.8A6 6 0 0 0 8.5 2.5zM4.5 8.5a4 4 0 1 1 8 0 4 4 0 0 1-8 0z" />
  </svg>
  <input
    bind:this={inputEl}
    class="omnibox"
    type="text"
    placeholder={$t('omnibox.placeholder')}
    bind:value={url}
    oninput={onInput}
    autocomplete="off"
    autocorrect="off"
    autocapitalize="off"
    spellcheck="false"
    enterkeyhint="go"
  />
  {#if url.length > 0}
    <button type="button" class="clear-btn" onclick={() => { url = ""; onInput?.(); focusInput(); }} aria-label={$t('common.clear')}>
      <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true">
        <path d="M8 1.4a6.6 6.6 0 1 0 0 13.2A6.6 6.6 0 0 0 8 1.4zm2.35 3.5a.75.75 0 0 1 1.06 1.06L9.06 8l2.35 2.35a.75.75 0 1 1-1.06 1.06L8 9.06 5.65 11.4a.75.75 0 1 1-1.06-1.06L6.94 8 4.6 5.65A.75.75 0 0 1 5.65 4.6L8 6.94l2.35-2.04z" />
      </svg>
    </button>
  {:else if prominent}
    <span class="omnibox-kbd" aria-hidden="true"><span class="kbd">{shortcut("V")}</span></span>
  {/if}
  <ContextHint text={$t('hints.omnibox')} dismissKey="omnibox" />
</label>

<style>
  /* A macOS search field: control fill + hairline, glowing accent ring on focus.
     The prominent (stage) variant is the page's hero and the only elevated thing. */
  .omnibox-wrapper {
    width: 100%;
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-h-lg);
    padding: 0 var(--space-2) 0 var(--space-3);
    background: var(--control-bg);
    border-radius: var(--radius-md);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    cursor: text;
    transition: box-shadow var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }

  .omnibox-wrapper.prominent {
    height: 52px;
    padding: 0 var(--space-3) 0 var(--space-4);
    border-radius: var(--radius-xl);
    background: var(--surface);
    box-shadow:
      inset 0 0 0 var(--hairline) var(--content-border),
      0 10px 30px rgba(var(--shadow-ink), var(--elev-alpha-2)),
      0 2px 6px rgba(var(--shadow-ink), var(--elev-alpha-1));
  }

  .omnibox-wrapper.prominent .omnibox {
    font-size: var(--text-lg);
    font-weight: 400;
    letter-spacing: var(--track-snug);
  }

  .omnibox-wrapper.drag-over {
    background: var(--accent-soft);
    box-shadow:
      inset 0 0 0 2px var(--accent),
      0 0 0 4px var(--accent-soft);
  }

  .omnibox-wrapper:focus-within {
    box-shadow:
      inset 0 0 0 var(--hairline) var(--accent),
      0 0 0 3px var(--accent-soft);
  }

  .omnibox-wrapper.prominent:focus-within {
    box-shadow:
      inset 0 0 0 var(--hairline) var(--accent),
      0 0 0 4px var(--accent-soft),
      0 10px 30px rgba(var(--shadow-ink), var(--elev-alpha-2));
  }

  .omnibox-glyph {
    flex-shrink: 0;
    color: var(--text-dim);
    pointer-events: none;
  }

  .omnibox-wrapper:focus-within .omnibox-glyph {
    color: var(--accent-hi);
  }

  .omnibox {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    font-size: var(--text-base);
    background: transparent;
    color: var(--text);
    border: none;
  }

  .omnibox::placeholder {
    color: var(--text-dim);
  }

  .omnibox:focus {
    outline: none;
  }

  .omnibox-kbd {
    display: inline-flex;
    flex-shrink: 0;
    opacity: 0.8;
  }

  .omnibox-wrapper:focus-within .omnibox-kbd {
    opacity: 0;
  }

  .clear-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border: none;
    background: transparent;
    border-radius: var(--radius-full);
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
  }

  .clear-btn :global(svg) {
    pointer-events: none;
  }

  @media (hover: hover) {
    .clear-btn:hover {
      color: var(--text);
      background: var(--fill-2);
    }
  }

  .clear-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
</style>
