<script lang="ts">
  import { t } from "$lib/i18n";
  import { captureCombo, shortcutCaps } from "$lib/dictation/logic";
  import type { Platform } from "$lib/dictation/types";
  import Keycaps from "./Keycaps.svelte";

  let {
    value,
    platform,
    defaultValue,
    disabled = false,
    error = null,
    saving = false,
    onrecordstart,
    onrecordend,
    onpick,
  }: {
    value: string;
    platform: Platform;
    defaultValue: string;
    disabled?: boolean;
    error?: string | null;
    saving?: boolean;
    onrecordstart: () => void;
    onrecordend: () => void;
    onpick: (combo: string) => void;
  } = $props();

  let recording = $state(false);
  let held = $state<string[]>([]);
  let box: HTMLButtonElement | undefined = $state();

  function begin() {
    if (disabled) return;
    recording = true;
    held = [];
    onrecordstart();
    queueMicrotask(() => box?.focus());
  }

  function end() {
    if (!recording) return;
    recording = false;
    held = [];
    onrecordend();
  }

  function onkeydown(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    const c = captureCombo(e);
    if (c.cancel) {
      end();
      return;
    }
    held = c.held;
    if (c.combo) {
      const combo = c.combo;
      end();
      onpick(combo);
    }
  }

  function onkeyup(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    held = captureCombo(e).held;
  }

  let heldCaps = $derived(held.length ? shortcutCaps(held.join("+"), platform).join(" ") : "");
</script>

<div class="dc-rec">
  {#if recording}
    <button
      bind:this={box}
      type="button"
      class="dc-rec-box recording"
      aria-live="polite"
      {onkeydown}
      {onkeyup}
      onblur={end}
      onclick={end}
    >
      {heldCaps ? $t("dictation.rec_holding", { keys: heldCaps }) : $t("dictation.rec_press")}
    </button>
    <span class="dc-rec-hint">{$t("dictation.rec_cancel_hint")}</span>
  {:else}
    <div class="dc-rec-row">
      <span class="dc-rec-box" class:invalid={!!error} aria-label={$t("dictation.shortcut_current") as string}>
        {#if value}
          <Keycaps combo={value} {platform} />
        {:else}
          <span class="dc-rec-none">{$t("dictation.shortcut_none")}</span>
        {/if}
        {#if saving}<span class="spinner dc-rec-spin" aria-hidden="true"></span>{/if}
      </span>
      <button type="button" class="btn btn-secondary btn-sm" {disabled} onclick={begin}>{$t("dictation.shortcut_change")}</button>
      {#if value !== defaultValue}
        <button type="button" class="btn btn-ghost btn-sm" {disabled} onclick={() => onpick(defaultValue)}>{$t("dictation.shortcut_reset")}</button>
      {/if}
    </div>
  {/if}
  {#if error}
    <p class="field-error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .dc-rec {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .dc-rec-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .dc-rec-box {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--control-h-lg);
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-md);
    background: var(--control-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
    font: inherit;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .dc-rec-box.recording {
    justify-content: center;
    width: 100%;
    color: var(--accent-text);
    box-shadow: inset 0 0 0 1.5px var(--accent), 0 0 0 3px var(--accent-soft);
    cursor: pointer;
  }

  .dc-rec-box.recording:focus-visible {
    outline: none;
  }

  .dc-rec-box.invalid {
    box-shadow: inset 0 0 0 var(--hairline) var(--danger);
  }

  .dc-rec-none {
    color: var(--text-dim);
  }

  .dc-rec-hint {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .dc-rec-spin {
    width: 12px;
    height: 12px;
  }
</style>
