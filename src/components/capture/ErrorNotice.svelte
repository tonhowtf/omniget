<script lang="ts">
  import { t } from "$lib/i18n";
  import { errorKey, parseError } from "$lib/capture/record";
  import type { Platform } from "$lib/capture/types";
  import StatusIcon from "#components/converter/StatusIcon.svelte";

  let {
    error,
    platform = "other",
    retryLabel = null,
    onretry,
    ondismiss,
    onopenprivacy,
  }: {
    error: string;
    platform?: Platform;
    retryLabel?: string | null;
    onretry?: () => void;
    ondismiss?: () => void;
    onopenprivacy?: (kind: "screen" | "mic") => void;
  } = $props();

  let parsed = $derived(parseError(error));
  let key = $derived(errorKey(parsed.code));
  let screenPerm = $derived(parsed.code === "CAPTURE_PERMISSION_SCREEN");
  let micPerm = $derived(parsed.code === "CAPTURE_PERMISSION_MIC");
  let mac = $derived(platform === "macos");
</script>

<div class="cap-err" role="alert">
  <StatusIcon name="warning" size={18} />
  <div class="cap-err-copy">
    <h3>{$t(`${key}_title`)}</h3>
    <p>{$t(`${key}_body`)}</p>
    {#if (screenPerm || micPerm) && mac}
      <ol class="cap-err-steps">
        <li>{$t(screenPerm ? "capture.perm_step_screen_1" : "capture.perm_step_mic_1")}</li>
        <li>{$t("capture.perm_step_2")}</li>
        <li>{$t("capture.perm_step_3")}</li>
      </ol>
    {/if}
    {#if parsed.detail && parsed.code !== "CAPTURE_CANCELLED"}
      <details class="cap-err-detail">
        <summary>{$t("capture.err_details")}</summary>
        <pre>{parsed.detail}</pre>
      </details>
    {/if}
    <div class="cap-err-actions">
      {#if (screenPerm || micPerm) && mac && onopenprivacy}
        <button type="button" class="btn btn-primary btn-sm" onclick={() => onopenprivacy?.(micPerm ? "mic" : "screen")}>
          {$t(micPerm ? "capture.open_mic_settings" : "capture.open_screen_settings")}
        </button>
      {/if}
      {#if onretry && retryLabel}
        <button type="button" class="btn btn-secondary btn-sm" onclick={onretry}>{retryLabel}</button>
      {/if}
      {#if ondismiss}
        <button type="button" class="btn btn-ghost btn-sm" onclick={ondismiss}>{$t("capture.dismiss")}</button>
      {/if}
    </div>
  </div>
</div>

<style>
  .cap-err {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--warning) 45%, var(--content-border));
    color: var(--warning);
  }

  .cap-err-copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    color: var(--text);
  }

  .cap-err-copy h3 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
  }

  .cap-err-copy p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .cap-err-steps {
    margin: 0;
    padding-inline-start: var(--space-5);
    font-size: var(--text-sm);
    color: var(--text-muted);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .cap-err-detail summary {
    cursor: pointer;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .cap-err-detail pre {
    margin: var(--space-2) 0 0;
    padding: var(--space-2);
    max-height: 140px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--surface-mut);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-muted);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .cap-err-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
</style>
