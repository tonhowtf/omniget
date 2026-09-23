<script lang="ts">
  // Undo toast for pin / snooze / archive (bottom-left of the threads screen).
  import { t } from "$lib/i18n";
  import { threadsUi } from "$lib/central/threads-ui/ui-state.svelte";
  import Icon from "./Icon.svelte";

  function onKey(e: KeyboardEvent) {
    if (threadsUi.undo && (e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z" && !isEditable(e.target)) {
      e.preventDefault();
      void threadsUi.runUndo();
    }
  }

  function isEditable(el: EventTarget | null): boolean {
    const n = el as HTMLElement | null;
    return !!n && (n.isContentEditable || n.tagName === "INPUT" || n.tagName === "TEXTAREA" || n.tagName === "SELECT");
  }
</script>

<svelte:window onkeydown={onKey} />

{#if threadsUi.undo}
  {#key threadsUi.undo.id}
    <div class="undo" role="status" aria-live="polite">
      <span class="msg">{threadsUi.undo.message}</span>
      <button type="button" class="undo-btn" onclick={() => threadsUi.runUndo()}>
        <Icon name="arrow-counter-clockwise" size={14} />{$t("llm.central.threads.undo")}
      </button>
      <button type="button" class="close" aria-label={$t("llm.central.threads.dismiss")} onclick={() => threadsUi.dismissUndo()}>
        <Icon name="x" size={12} />
      </button>
    </div>
  {/key}
{/if}

<style>
  .undo {
    position: absolute; left: 16px; bottom: 16px; z-index: 40;
    display: flex; align-items: center; gap: 10px;
    padding: 8px 8px 8px 14px; border-radius: 12px;
    background: var(--popup-bg, var(--surface)); color: var(--text);
    border: 1px solid var(--separator);
    box-shadow: 0 8px 24px color-mix(in srgb, #000 22%, transparent);
    font-size: 13px; max-width: min(420px, calc(100% - 32px));
    animation: rise 180ms var(--ease-out, ease-out);
  }
  .msg { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .undo-btn { display: inline-flex; align-items: center; gap: 6px; border: 0; background: var(--accent-soft); color: var(--accent-hi); padding: 5px 10px; border-radius: 8px; font-weight: 600; cursor: pointer; font-size: 12px; }
  .close { border: 0; background: transparent; color: var(--text-muted); padding: 4px; border-radius: 6px; cursor: pointer; display: inline-flex; }
  .close:hover { background: var(--fill-2); }
  @keyframes rise { from { transform: translateY(8px); opacity: 0; } to { transform: none; opacity: 1; } }
  @media (prefers-reduced-motion: reduce) { .undo { animation: none; } }
</style>
