<script lang="ts">
  import { tick } from "svelte";
  import { t } from "$lib/i18n";
  import TgIcon from "$lib/telegram/TgIcon.svelte";
  import type { DcMessage } from "./api";
  import { displayName } from "./format";

  const MAX = 2000;

  let {
    placeholder,
    disabled = false,
    replyTo = null,
    editing = null,
    onSubmit,
    onEditLast,
    onCancel,
  }: {
    placeholder: string;
    disabled?: boolean;
    replyTo?: DcMessage | null;
    editing?: DcMessage | null;
    onSubmit: (text: string) => Promise<boolean>;
    onEditLast: () => void;
    onCancel: () => void;
  } = $props();

  let text = $state("");
  let busy = $state(false);
  let box: HTMLTextAreaElement | undefined = $state();
  let count = $derived([...text].length);
  let lastEditing: string | null = null;

  $effect(() => {
    const id = editing?.id ?? null;
    if (id === lastEditing) return;
    lastEditing = id;
    text = editing ? editing.content : "";
    void tick().then(() => {
      box?.focus();
      resize();
    });
  });

  $effect(() => {
    if (box) resize();
  });

  $effect(() => {
    if (replyTo) void tick().then(() => box?.focus());
  });

  export function focus() {
    box?.focus();
  }

  function resize() {
    if (!box) return;
    box.style.height = "auto";
    box.style.height = `${Math.min(box.scrollHeight, 200)}px`;
  }

  async function submit() {
    if (busy || disabled || !text.trim() || count > MAX) return;
    busy = true;
    const ok = await onSubmit(text);
    busy = false;
    if (ok) {
      text = "";
      await tick();
      resize();
      box?.focus();
    }
  }

  function key(e: KeyboardEvent) {
    if (e.isComposing) return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    } else if (e.key === "ArrowUp" && !text && !editing) {
      e.preventDefault();
      onEditLast();
    } else if (e.key === "Escape" && (editing || replyTo)) {
      e.preventDefault();
      onCancel();
    }
  }
</script>

<div class="composer" class:disabled>
  {#if editing || replyTo}
    <div class="ctx">
      <TgIcon name={editing ? "edit" : "forward"} />
      <span class="ctx-text">
        {#if editing}
          {$t("discord.composer.editing")}
        {:else if replyTo}
          {$t("discord.composer.replying", { name: displayName(replyTo.author) })}
        {/if}
      </span>
      <span class="ctx-hint">{$t("discord.composer.esc_hint")}</span>
      <button class="btn btn-ghost btn-sm btn-icon" aria-label={$t("discord.common.cancel")} onclick={onCancel}><TgIcon name="x" /></button>
    </div>
  {/if}
  <div class="field-wrap">
    <textarea
      bind:this={box}
      bind:value={text}
      class="input"
      rows="1"
      {placeholder}
      {disabled}
      oninput={resize}
      onkeydown={key}
      aria-label={placeholder}
    ></textarea>
    {#if count > MAX - 200}<span class="count" class:over={count > MAX}>{MAX - count}</span>{/if}
    <button class="btn btn-primary btn-sm send" disabled={disabled || busy || !text.trim() || count > MAX} onclick={() => void submit()} aria-label={$t(editing ? "discord.composer.save" : "discord.composer.send")}>
      {#if busy}<span class="spinner"></span>{:else}{$t(editing ? "discord.composer.save" : "discord.composer.send")}{/if}
    </button>
  </div>
</div>

<style>
  .composer {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-2) var(--space-3) var(--space-3);
  }

  .ctx {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 2px var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .ctx-text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .ctx-hint {
    font-size: var(--text-xs);
    color: var(--text-faint);
  }

  .field-wrap {
    position: relative;
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
  }

  .field-wrap textarea {
    flex: 1;
    min-height: var(--control-h-lg);
    max-height: 200px;
    padding-block: 8px;
    resize: none;
    line-height: 1.4;
    font: inherit;
    font-size: var(--text-base);
  }

  .count {
    position: absolute;
    right: 84px;
    bottom: 8px;
    font-size: var(--text-xs);
    color: var(--text-faint);
  }

  .count.over {
    color: var(--error);
  }

  .send {
    height: var(--control-h-lg);
    min-width: 72px;
  }
</style>
