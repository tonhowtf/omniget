<script lang="ts">
  /** Asks what a blocked issue waits for (the core refuses `blocked` without it). */
  import { t } from "$lib/i18n";

  let {
    identifier,
    onconfirm,
    oncancel,
  }: { identifier: string; onconfirm: (note: string) => void; oncancel: () => void } = $props();

  let note = $state("");
  let box = $state<HTMLTextAreaElement | null>(null);
  $effect(() => {
    box?.focus();
  });

  function submit(e: Event) {
    e.preventDefault();
    if (note.trim()) onconfirm(note.trim());
  }
</script>

<div class="dialog-backdrop" role="dialog" aria-modal="true" aria-labelledby="blocked-title" tabindex="-1" onkeydown={(e) => e.key === "Escape" && oncancel()} onclick={(e) => e.target === e.currentTarget && oncancel()}>
  <form
    class="dialog-card"
    onsubmit={submit}
   
  >
    <h2 class="dialog-title" id="blocked-title">{$t("company.issues.blocked_title", { identifier })}</h2>
    <label class="field">
      <span class="field-label">{$t("company.issues.blocked_reason")}</span>
      <textarea class="input" rows="3" bind:this={box} bind:value={note} placeholder={$t("company.issues.blocked_placeholder")}></textarea>
    </label>
    <div class="dialog-actions">
      <button type="button" class="btn btn-secondary" onclick={oncancel}>{$t("company.common.cancel")}</button>
      <button type="submit" class="btn btn-primary" disabled={!note.trim()}>{$t("company.issues.mark_blocked")}</button>
    </div>
  </form>
</div>

<style>
  textarea.input {
    height: auto;
    resize: vertical;
    font-family: inherit;
  }
</style>
