<script lang="ts">
  import { t } from "$lib/i18n";
  import UsageWindows from "./UsageWindows.svelte";
  import type { AccountView, SandboxMode } from "$lib/stores/llm-accounts-store.svelte";

  interface Props {
    account: AccountView;
    rank: number;
    available: boolean;
    nowMs?: number;
    busy?: boolean;
    onactivate: (id: string) => void;
    onlogin: (id: string) => void;
    ontoggle: (id: string, disabled: boolean) => void;
    onremove: (id: string, purge: boolean) => void;
    onsandbox: (id: string, sandbox: SandboxMode) => void;
  }

  let {
    account,
    rank,
    available,
    nowMs = Date.now(),
    busy = false,
    onactivate,
    onlogin,
    ontoggle,
    onremove,
    onsandbox,
  }: Props = $props();

  let confirming = $state(false);
  let purge = $state(false);
  let confirmingWrite = $state(false);

  let writes = $derived(account.sandbox === "write" || account.sandbox === "full");
  let full = $derived(account.sandbox === "full");
  let confirmingFull = $state(false);

  function toggleFull() {
    if (full) {
      onsandbox(account.id, "write");
      confirmingFull = false;
      return;
    }
    if (!confirmingFull) {
      confirmingFull = true;
      return;
    }
    onsandbox(account.id, "full");
    confirmingFull = false;
  }

  function removeNow() {
    onremove(account.id, purge);
    confirming = false;
    purge = false;
  }

  function toggleSandbox() {
    if (writes) {
      onsandbox(account.id, "read-only");
      confirmingWrite = false;
      return;
    }
    if (!confirmingWrite) {
      confirmingWrite = true;
      return;
    }
    onsandbox(account.id, "write");
    confirmingWrite = false;
  }
</script>

<article class="account surface-card" class:active={account.active} class:off={account.disabled}>
  <header class="head">
    <span class="rank" aria-hidden="true">{rank}</span>
    <div class="ident">
      <h3 class="name">{account.label}</h3>
      <p class="meta">
        <span class="tag">{account.cli}</span>
        {#if account.active}<span class="tag tag-accent">{$t("llm.accounts.active")}</span>{/if}
        {#if account.disabled}<span class="tag tag-warning">{$t("llm.accounts.disabled")}</span>{/if}
        {#if account.sessions > 0}
          <span class="sessions">{account.sessions} {$t("llm.accounts.sessions")}</span>
        {/if}
      </p>
    </div>
  </header>

  <code class="dir" title={account.config_dir}>{account.config_dir}</code>

  <div class="sandbox" class:writes>
    <div class="sandbox-text">
      <span class="sandbox-label">
        {$t("llm.accounts.sandbox")}
        <strong>{full ? $t("llm.accounts.sandbox_full") : writes ? $t("llm.accounts.sandbox_write") : $t("llm.accounts.sandbox_read_only")}</strong>
      </span>
      {#if account.sandbox_flag}
        <code class="sandbox-flag">{account.sandbox_flag}</code>
      {/if}
      <span class="sandbox-hint">
        {full ? $t("llm.accounts.sandbox_full_hint") : writes ? $t("llm.accounts.sandbox_write_hint") : $t("llm.accounts.sandbox_read_only_hint")}
      </span>
    </div>
    <button
      class="button"
      class:danger={!writes && confirmingWrite}
      type="button"
      disabled={busy}
      onclick={toggleSandbox}
    >
      {#if writes}
        {$t("llm.accounts.sandbox_make_read_only")}
      {:else if confirmingWrite}
        {$t("llm.accounts.sandbox_allow_write_confirm")}
      {:else}
        {$t("llm.accounts.sandbox_allow_write")}
      {/if}
    </button>
    {#if confirmingWrite && !writes}
      <button class="button" type="button" onclick={() => (confirmingWrite = false)}>
        {$t("llm.accounts.cancel")}
      </button>
    {/if}
    {#if writes}
      <button
        class="button"
        class:danger={!full && confirmingFull}
        type="button"
        disabled={busy}
        onclick={toggleFull}
      >
        {#if full}
          {$t("llm.accounts.sandbox_make_write")}
        {:else if confirmingFull}
          {$t("llm.accounts.sandbox_allow_full_confirm")}
        {:else}
          {$t("llm.accounts.sandbox_allow_full")}
        {/if}
      </button>
      {#if confirmingFull && !full}
        <button class="button" type="button" onclick={() => (confirmingFull = false)}>
          {$t("llm.accounts.cancel")}
        </button>
      {/if}
    {/if}
  </div>

  <UsageWindows windows={account.windows} {available} active={account.active} {nowMs} />

  {#if account.last_error}
    <p class="error-line" role="status">
      {$t("llm.accounts.last_error")} <code>{account.last_error}</code>
    </p>
  {/if}

  <footer class="actions">
    <button class="button" type="button" disabled={busy} onclick={() => onlogin(account.id)}>
      {$t("llm.accounts.enter")}
    </button>
    <button
      class="button"
      class:active={account.active}
      type="button"
      disabled={busy || account.active || account.disabled}
      onclick={() => onactivate(account.id)}
    >
      {$t("llm.accounts.use_now")}
    </button>
    <button
      class="button"
      type="button"
      disabled={busy}
      onclick={() => ontoggle(account.id, !account.disabled)}
    >
      {account.disabled ? $t("llm.accounts.enable") : $t("llm.accounts.disable")}
    </button>
    {#if confirming}
      <label class="purge">
        <input class="checkbox" type="checkbox" bind:checked={purge} />
        {$t("llm.accounts.purge_dir")}
      </label>
      <button class="button danger" type="button" disabled={busy} onclick={removeNow}>
        {$t("llm.accounts.remove_confirm")}
      </button>
      <button class="button" type="button" onclick={() => (confirming = false)}>
        {$t("llm.accounts.cancel")}
      </button>
    {:else}
      <button class="button" type="button" disabled={busy} onclick={() => (confirming = true)}>
        {$t("llm.accounts.remove")}
      </button>
    {/if}
  </footer>
</article>

<style>
  .account {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    min-width: 0;
  }
  .account.active {
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .account.off {
    opacity: 0.66;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }
  .rank {
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border-radius: var(--radius-full);
    border: 1px solid var(--border);
    font-size: var(--text-caption);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .ident {
    min-width: 0;
  }
  .name {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 2px 0 0;
    flex-wrap: wrap;
  }
  .sessions {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .dir {
    display: block;
    font-size: var(--text-caption);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .error-line {
    margin: 0;
    font-size: var(--text-caption);
    color: var(--warning);
  }
  .sandbox {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }
  .sandbox.writes {
    border-color: color-mix(in srgb, var(--warning) 50%, var(--border));
    background: color-mix(in srgb, var(--warning) 8%, transparent);
  }
  .sandbox-text {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
  }
  .sandbox-label {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .sandbox-label strong {
    color: var(--text);
  }
  .sandbox-flag {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .sandbox-hint {
    flex-basis: 100%;
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .sandbox .button {
    min-height: 28px;
    font-size: var(--text-sm);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .actions .button {
    min-height: 28px;
    font-size: var(--text-sm);
  }
  .button.danger {
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    color: var(--danger);
  }
  .purge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-caption);
    color: var(--text-muted);
  }
</style>
