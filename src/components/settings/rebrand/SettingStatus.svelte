<script lang="ts">
  import { t } from "$lib/i18n";
  import { getFieldStatus, retryLocalPresentation, retrySettingsEffect } from "$lib/stores/settings-store.svelte";
  import { registerStatusSurface } from "$lib/stores/settings-status-surfaces";

  type FieldState = "idle" | "saving" | "saved" | "failed" | "pendingApply";

  let {
    path,
    state: stateProp,
    onRetry,
  }: {
    path?: string;
    state?: FieldState;
    onRetry?: () => void;
  } = $props();

  $effect(() => {
    if (!path) return;
    return registerStatusSurface(path);
  });

  let field = $derived(path ? getFieldStatus(path) : undefined);
  let current = $derived<FieldState>(stateProp ?? field?.state ?? "idle");
  let retrying = $state(false);

  let localFailure = $derived(field?.error === "local_apply_failed");
  let restartRequired = $derived(field?.error === "restart_required");
  let canRetryApply = $derived(!restartRequired && (!!onRetry || localFailure || !!field?.effectId));

  async function retryApply() {
    if (onRetry) { onRetry(); return; }
    if (localFailure) { retryLocalPresentation(); return; }
    const effectId = field?.effectId;
    if (!effectId || retrying) return;
    retrying = true;
    try { await retrySettingsEffect(effectId); } catch { }
    finally { retrying = false; }
  }
</script>

<span class="setting-status" role="status" aria-live="polite">
  {#if current === "saving"}
    <span class="status-text">{$t("settings.rebrand.state.saving")}</span>
  {:else if current === "saved"}
    <span class="status-text">{$t("settings.rebrand.state.saved")}</span>
  {:else if current === "pendingApply" && restartRequired}
    <span class="status-text">{$t("settings.rebrand.state.restart_required")}</span>
  {:else if current === "pendingApply"}
    <span class="status-text">{$t("settings.rebrand.state.pending_apply")}</span>
    {#if canRetryApply}
      <button type="button" class="status-action" onclick={retryApply} disabled={retrying}>{$t("settings.rebrand.state.retry_apply")}</button>
    {/if}
  {/if}
</span>
{#if current === "failed"}
  <span class="setting-status is-error" role="alert">
    <span class="status-text">{$t("settings.rebrand.state.save_failed")}</span>
    {#if onRetry}<button type="button" class="status-action" onclick={onRetry}>{$t("settings.rebrand.state.retry")}</button>{/if}
  </span>
{/if}

<style>
  .setting-status { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .setting-status:empty { display: none; }
  .is-error { color: var(--danger, var(--error)); margin-block-start: 2px; }
  .status-action {
    min-block-size: 28px; padding-inline: var(--space-2);
    border: 1px solid var(--separator); border-radius: 6px;
    background: transparent; color: var(--text); font: inherit; font-size: 12px; cursor: pointer;
  }
  .status-action:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .status-action:disabled { opacity: 0.6; cursor: default; }
</style>
