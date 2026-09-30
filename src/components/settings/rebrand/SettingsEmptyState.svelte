<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import SettingsIcon, { type SettingsIconName } from "./SettingsIcon.svelte";

  let {
    icon,
    title,
    hint,
    action,
    error = false,
    onRetry,
  }: {
    icon?: SettingsIconName;
    title: string;
    hint?: string;
    action?: Snippet;
    error?: boolean;
    onRetry?: () => void;
  } = $props();

  let iconName = $derived<SettingsIconName | undefined>(error ? "alert" : icon);
</script>

<div class="settings-empty" class:is-error={error} role={error ? "alert" : undefined}>
  {#if iconName}<span class="empty-icon"><SettingsIcon name={iconName} size={20} /></span>{/if}
  <div class="empty-text">
    <p class="empty-title">{title}</p>
    {#if hint}<p class="empty-hint">{hint}</p>{/if}
  </div>
  {#if action}
    <div class="empty-action">{@render action()}</div>
  {:else if error && onRetry}
    <div class="empty-action"><button type="button" class="btn btn-secondary btn-sm" onclick={onRetry}>{$t("settings.rebrand.state.retry")}</button></div>
  {/if}
</div>

<style>
  .settings-empty { display: flex; align-items: flex-start; gap: var(--space-3); padding: var(--space-4) 0; flex-wrap: wrap; }
  .empty-icon { color: var(--text-muted); padding-block-start: 1px; }
  .is-error .empty-icon, .is-error .empty-title { color: var(--danger, var(--error)); }
  .empty-text { flex: 1 1 14rem; min-inline-size: 0; }
  .empty-title { margin: 0; font-size: 13px; line-height: 20px; color: var(--text); font-weight: 600; }
  .empty-hint { margin: 2px 0 0; font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .empty-action { flex: none; }
</style>
