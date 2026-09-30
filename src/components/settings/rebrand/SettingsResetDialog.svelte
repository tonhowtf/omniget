<script lang="ts">
  import { t } from "$lib/i18n";
  import DialogContainer from "$components/dialog/DialogContainer.svelte";
  import { resetSettings } from "$lib/stores/settings-store.svelte";
  import type { ResetScope } from "$lib/stores/settings-v3-types";

  // Reset confirmation on the app's shared dialog (focus trap, Escape, focus return).
  // Copy always states the scope and what is kept; the action names the reset.
  let {
    open = $bindable(false),
    scope,
    sectionName = "",
  }: {
    open?: boolean;
    scope: ResetScope;
    sectionName?: string;
  } = $props();

  let busy = $state(false);
  let failed = $state(false);

  $effect(() => {
    if (open) failed = false;
  });

  async function confirm() {
    if (busy) return;
    busy = true;
    failed = false;
    try {
      await resetSettings(scope);
      open = false;
    } catch {
      failed = true;
    } finally {
      busy = false;
    }
  }
</script>

<DialogContainer bind:isOpen={open} titleId="settings-reset-title" onClose={() => { open = false; }}>
  <h2 id="settings-reset-title" class="dialog-title">
    {scope.kind === "app" ? $t("settings.rebrand.reset.app_title") : $t("settings.rebrand.reset.section_title", { name: sectionName })}
  </h2>
  <div class="dialog-body">
    <p>{scope.kind === "app" ? $t("settings.rebrand.reset.app_body") : $t("settings.rebrand.reset.section_body")}</p>
    <p>{$t("settings.rebrand.reset.kept")}</p>
    {#if failed}<p class="dialog-error" role="alert">{$t("settings.save_failed")}</p>{/if}
  </div>
  <div class="dialog-actions">
    <button type="button" class="btn btn-secondary" onclick={() => { open = false; }} disabled={busy}>{$t("settings.rebrand.dialog.cancel")}</button>
    <button type="button" class="btn btn-destructive" onclick={confirm} disabled={busy} aria-busy={busy}>
      {scope.kind === "app" ? $t("settings.rebrand.reset.app_action") : $t("settings.rebrand.reset.section_action")}
    </button>
  </div>
</DialogContainer>

<style>
  .dialog-title { margin: 0; padding: var(--space-5) var(--space-5) var(--space-2); font-size: 15px; line-height: 20px; font-weight: 600; color: var(--text); }
  .dialog-body { padding: 0 var(--space-5) var(--space-4); }
  .dialog-body p { margin: 0 0 var(--space-2); font-size: 13px; line-height: 20px; color: var(--text-muted); }
  .dialog-error { color: var(--danger, var(--error)) !important; }
  .dialog-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--space-2); padding: var(--space-3) var(--space-5) var(--space-5); }
</style>
