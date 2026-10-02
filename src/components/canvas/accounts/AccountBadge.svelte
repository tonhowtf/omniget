<script lang="ts">
  import { t } from "$lib/i18n";
  import type { AccountBadge, CanvasAccount } from "$lib/canvas/accounts/store.svelte";

  let {
    badge,
    options = [],
    compact = false,
    readonly = false,
    onpick,
  }: {
    badge: AccountBadge;
    options?: CanvasAccount[];
    compact?: boolean;
    readonly?: boolean;
    onpick?: (id: string | null) => void;
  } = $props();

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) }) as string;

  const name = $derived(badge.id ? badge.label : tr("canvas.accounts.environment", "default account"));
  const hint = $derived.by(() => {
    if (badge.kind === "invalid") {
      if (badge.reason === "disabled") return tr("canvas.accounts.invalidDisabled", "This account is disabled; pick another");
      if (badge.reason === "cli") return tr("canvas.accounts.invalidCli", "This account is for another CLI; pick another");
      return tr("canvas.accounts.invalidMissing", "This account no longer exists; pick another");
    }
    if (badge.kind === "restart") return tr("canvas.accounts.restartHint", "The new account applies when this terminal restarts");
    if (badge.kind === "environment") return tr("canvas.accounts.environmentHint", "Runs on the account of the environment the app was started in");
    return tr("canvas.accounts.accountHint", "Runs on the account {{name}}", { name });
  });

  function change(e: Event) {
    const v = (e.currentTarget as HTMLSelectElement).value;
    onpick?.(v === "" ? null : v);
  }
</script>

<span
  class="acct"
  class:invalid={badge.kind === "invalid"}
  class:restart={badge.kind === "restart"}
  class:env={badge.kind === "environment"}
  class:compact
  style:--acct={badge.color}
  title={hint}
>
  <span class="dot" aria-hidden="true">●</span>
  {#if !compact}<span class="name">{name}</span>{/if}
  {#if badge.kind === "restart" && !compact}
    <span class="note">{tr("canvas.accounts.restart", "restart to apply")}</span>
  {/if}
  {#if !readonly}
    <select
      aria-label={tr("canvas.accounts.pick", "Account")}
      value={badge.id ?? ""}
      onchange={change}
      onpointerdown={(e) => e.stopPropagation()}
    >
      <option value="">{tr("canvas.accounts.environmentOption", "Environment account")}</option>
      {#each options as a (a.id)}
        <option value={a.id} disabled={a.disabled}>{a.label}</option>
      {/each}
      {#if badge.id && !options.some((a) => a.id === badge.id)}
        <option value={badge.id} disabled>{badge.label}</option>
      {/if}
    </select>
  {/if}
</span>

<style>
  .acct {
    position: relative;
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 160px;
    height: 18px;
    padding: 0 6px;
    border-radius: 9px;
    font-size: 11px;
    line-height: 1;
    color: var(--text, #f5f5f7);
    background: color-mix(in srgb, var(--acct) 18%, transparent);
    border: 1px solid color-mix(in srgb, var(--acct) 45%, transparent);
  }
  .acct.env {
    background: transparent;
    border-style: dashed;
    color: var(--text-dim, #98989d);
  }
  .acct.restart {
    border-style: dashed;
  }
  .acct.invalid {
    --acct: var(--error, #ff453a);
    color: var(--error, #ff453a);
  }
  .acct.compact {
    padding: 0 4px;
  }
  .dot {
    color: var(--acct);
    font-size: 9px;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .note {
    flex: none;
    color: var(--accent, #ff9f0a);
    white-space: nowrap;
  }
  select {
    position: absolute;
    inset: 0;
    width: 100%;
    opacity: 0;
    cursor: pointer;
    font-size: 12px;
  }
</style>
