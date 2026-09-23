<script lang="ts">
  // "Edit from here": rewind the chat to before a message; the prompt goes
  // back to the composer. Files can be restored too (checkpoint of that
  // turn) — safe in a thread worktree, a warning in a shared checkout.
  import { t } from "$lib/i18n";
  import Modal from "./Modal.svelte";

  let {
    open,
    turnCount,
    worktree,
    busy = false,
    error = null,
    onconfirm,
    onclose,
  }: {
    open: boolean;
    turnCount: number;
    worktree: boolean;
    busy?: boolean;
    error?: string | null;
    onconfirm: (restoreFiles: boolean) => void;
    onclose: () => void;
  } = $props();
</script>

<Modal {open} title={$t("llm.central.threads.revert.title")} onclose={() => !busy && onclose()} width={480}>
  <p class="text">{$t("llm.central.threads.revert.body", { n: turnCount })}</p>
  {#if !worktree}<p class="warn">{$t("llm.central.threads.revert.shared_warning")}</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#snippet footer()}
    <button type="button" class="button" disabled={busy} onclick={onclose}>{$t("llm.central.threads.cancel")}</button>
    <button type="button" class="button" disabled={busy} onclick={() => onconfirm(false)}>{$t("llm.central.threads.revert.chat_only")}</button>
    <button type="button" class="button primary danger" disabled={busy} onclick={() => onconfirm(true)}>{$t("llm.central.threads.revert.files_and_chat")}</button>
  {/snippet}
</Modal>

<style>
  .text { margin: 0; font-size: 13.5px; line-height: 1.5; }
  .warn { margin: 0; font-size: 12.5px; color: var(--orange, #b45309); background: color-mix(in srgb, var(--orange, #f59e0b) 10%, transparent); padding: 8px 10px; border-radius: 8px; }
  .error { margin: 0; color: var(--red, #ef4444); font-size: 12.5px; }
</style>
