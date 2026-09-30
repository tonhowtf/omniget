<script lang="ts">
  import { t } from "$lib/i18n";
  import TgIcon from "./TgIcon.svelte";
  import { netState, scanNet, isScanning, CLOCK_WARN_SECS } from "./net.svelte";

  let { onOpenConnection }: { onOpenConnection: () => void } = $props();

  let status = $derived(netState.status);
  let scanning = $derived(isScanning());
  let active = $derived(status?.active ?? null);
  let phase = $derived(
    scanning ? "scanning" : status?.phase === "connected" && active ? "connected" : status?.phase === "failed" ? "failed" : status?.phase === "connected" ? "connected" : "idle",
  );
  let clockOff = $derived(status?.clock_offset_secs ?? 0);
</script>

<div class="strip" data-phase={phase} role="status" aria-live="polite">
  <span class="dot" aria-hidden="true"></span>
  <span class="text">
    {#if phase === "scanning"}
      {$t("telegram.status.scanning")}
    {:else if phase === "connected"}
      {#if active}
        {$t("telegram.status.connected", { route: active.route.name })}
        {#if active.health.rtt_ms != null}<span class="ms">{$t("telegram.status.ms", { ms: active.health.rtt_ms })}</span>{/if}
        {#if status?.mode === "fixed"}<span class="tag tag-accent">{$t("telegram.net.mode_fixed")}</span>{/if}
      {:else}
        {$t("telegram.status.connected_plain")}
      {/if}
    {:else if phase === "failed"}
      {$t("telegram.status.failed")}
    {:else}
      {$t("telegram.status.idle")}
    {/if}
  </span>
  <span class="actions">
    <button class="btn btn-ghost btn-sm" onclick={() => void scanNet()} disabled={scanning}>
      {#if scanning}<span class="spinner" aria-hidden="true"></span>{:else}<TgIcon name="refresh" />{/if}
      {$t("telegram.status.scan_again")}
    </button>
    <button class="btn btn-ghost btn-sm" onclick={onOpenConnection}>
      <TgIcon name="plug" />
      {$t("telegram.status.connection")}
    </button>
  </span>
</div>
{#if Math.abs(clockOff) > CLOCK_WARN_SECS}
  <div class="notice notice-warning clock">
    <TgIcon name="clock" size={16} />
    <div class="notice-body">
      <div class="notice-text">{$t("telegram.status.clock_warn", { secs: clockOff > 0 ? `+${clockOff}` : `${clockOff}` })}</div>
    </div>
  </div>
{/if}

<style>
  .strip {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 36px;
    padding: var(--space-1) var(--space-2) var(--space-1) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    background: var(--text-dim);
    flex-shrink: 0;
  }

  [data-phase="connected"] .dot {
    background: var(--success);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--success) 22%, transparent);
  }

  [data-phase="failed"] .dot {
    background: var(--danger);
  }

  [data-phase="scanning"] .dot {
    background: var(--warning);
    animation: pulse 1.1s var(--ease-in-out) infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    [data-phase="scanning"] .dot {
      animation: none;
    }
  }

  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .ms {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .actions {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }

  .clock {
    margin-top: var(--space-2);
  }
</style>
