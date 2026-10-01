<script lang="ts">
  import { t } from "$lib/i18n";

  let {
    dirty,
    saving,
    error,
    onSave,
    onDiscard,
  }: {
    dirty: boolean;
    saving: boolean;
    error: string | null;
    onSave: () => void;
    onDiscard: () => void;
  } = $props();

  let visible = $derived(dirty || saving || !!error);
</script>

{#if visible}
  <div class="settings-save-bar" role="group" aria-label={$t("settings.rebrand.form.draft")}>
    <div class="save-bar-text">
      {#if error}
        <p class="save-bar-error" role="alert">{$t("settings.rebrand.state.save_failed")}</p>
      {:else}
        <p class="save-bar-draft" role="status">{saving ? $t("settings.rebrand.state.saving") : $t("settings.rebrand.form.draft")}</p>
      {/if}
    </div>
    <div class="save-bar-actions">
      <button type="button" class="btn btn-secondary btn-sm" onclick={onDiscard} disabled={saving || !dirty}>{$t("settings.rebrand.form.discard")}</button>
      <button type="button" class="btn btn-primary btn-sm" onclick={onSave} disabled={saving || !dirty} aria-busy={saving}>{$t("settings.rebrand.form.save")}</button>
    </div>
  </div>
{/if}

<style>
  .settings-save-bar {
    position: sticky;
    inset-block-end: 0;
    z-index: 1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2) var(--space-3);
    margin-block-start: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--separator);
    border-radius: 10px;
    background: var(--surface);
  }
  .save-bar-text { min-inline-size: 0; flex: 1 1 12rem; }
  .save-bar-text p { margin: 0; font-size: 13px; line-height: 20px; }
  .save-bar-draft { color: var(--text-muted); }
  .save-bar-error { color: var(--danger, var(--error)); }
  .save-bar-actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }
</style>
