<script lang="ts">
  /**
   * Accounts & quota.
   *
   * Top to bottom: the CLIs found on this machine, the rotation switch, the
   * fallback chain (drag to reorder, one click to promote), the switch
   * timeline, and the usage dashboard read from the CLIs' own JSONL.
   *
   * Three things this page will not do, by contract (`estudos/74` §A.3, plan
   * §9.2): read a credential, refresh a token, or call an undocumented usage
   * endpoint. An account here is a label and an isolated config directory;
   * signing in happens in a terminal the user sees, inside the CLI.
   *
   * Cadence: one read on mount, then nothing until the user clicks. The only
   * subscription is `llm://rerouted`, which the backend pushes and which stops
   * when the tab closes — no timer lives in this route.
   */
  import { onMount, tick } from "svelte";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import ConnectionWizard from "$components/llm/accounts/ConnectionWizard.svelte";
  import AccountList from "$components/llm/accounts/AccountList.svelte";
  import UsageDashboard from "$components/llm/accounts/UsageDashboard.svelte";
  import AcpAgents from "$components/llm/accounts/AcpAgents.svelte";
  import LimitsStrip from "$components/llm/accounts/LimitsStrip.svelte";
  import {
    activateAccount,
    createAccount,
    detectClis,
    getAccounts,
    getAccountsError,
    getAccountsState,
    getDetected,
    getSwitches,
    getUsage,
    getUsageError,
    getUsageState,
    isBusy,
    loadAccounts,
    loadUsage,
    loginAccount,
    removeAccount,
    setChain,
    setDisabled,
    setRotation,
    setSandbox,
    startAccountsDemo,
    startSwitchLog,
    type CliKind,
  } from "$lib/stores/llm-accounts-store.svelte";

  let snapshot = $derived(getAccounts());
  let usage = $derived(getUsage());
  let detected = $derived(getDetected());
  let switches = $derived(getSwitches());
  let busy = $derived(isBusy());

  let newCli = $state<CliKind>("claude");
  let newLabel = $state("");
  let creating = $state(false);
  let usageSection = $state<HTMLElement | null>(null);
  /** Share settings/skills/commands with the new profile, never the identity. */
  let newShare = $state(true);
  /** Clock of the render, not a ticking one: read once per page load. */
  const nowMs = Date.now();

  async function guard(run: () => Promise<void>) {
    try {
      await run();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function create() {
    if (!newLabel.trim()) return;
    await guard(async () => {
      await createAccount(newCli, newLabel.trim(), newShare);
      newLabel = "";
      creating = false;
      await loadUsage();
    });
  }

  async function refresh() {
    await loadAccounts();
    await loadUsage();
  }

  function timeOf(ms: number): string {
    return new Date(ms).toLocaleTimeString();
  }

  onMount(() => {
    if (page.url.searchParams.get("demo") === "1") {
      startAccountsDemo();
      return;
    }
    void refresh();
    void detectClis();
    let disposed = false;
    let stop: (() => void) | null = null;
    startSwitchLog().then(fn => { if (disposed) fn(); else stop = fn; });
    return () => { disposed = true; stop?.(); };
  });
  $effect(() => {
    getAccountsState();
    getUsageState();
    if (page.url.hash !== "#usage" || !usageSection) return;
    const target = usageSection;
    void tick().then(() => {
      target.scrollIntoView({ block: "start" });
      target.focus({ preventScroll: true });
    });
  });
</script>

<svelte:head><title>{$t("llm.accounts.title")}</title></svelte:head>

<div class="page page-wide accounts-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.accounts.title")}</h1>
      <p class="page-lede">{$t("llm.accounts.lede")}</p>
    </div>
    <div class="head-actions">
      <button class="button" type="button" disabled={busy} onclick={refresh}>
        {$t("llm.accounts.refresh")}
      </button>

    </div>
  </header>

  {#if page.url.searchParams.get("returnTo") === "agent"}
    <p class="return-draft"><a href="/llm/roster?create=1">{$t("assist.bots.create.return_to_draft")}</a></p>
  {:else if page.url.searchParams.get("returnTo") === "editor"}
    <p class="return-draft"><a href="/llm/roster?edit=resume">{$t("llm.roster.return_to_edit")}</a></p>
  {/if}
  {#key page.url.searchParams.get("guide")}
    <ConnectionWizard initial={page.url.searchParams.get("guide") ?? ""} returnToAgent={page.url.searchParams.get("returnTo") === "agent"} returnToEditor={page.url.searchParams.get("returnTo") === "editor"} />
  {/key}
  <p class="connection-links"><a href="/llm/models">{$t("llm.accounts.models_limits")}</a></p>

  <details class="advanced-accounts">
    <summary>{$t("llm.accounts.advanced_accounts")}</summary>
    <button class="button" type="button" onclick={() => (creating = !creating)} aria-expanded={creating}>{$t("llm.accounts.new")}</button>

  {#if detected.length > 0}
    <p class="detected">
      {$t("llm.accounts.detected")}
      {#each detected as d (d.cli)}
        <span class="tag">{d.cli}{d.version ? ` ${d.version}` : ""}</span>
      {/each}
    </p>
  {/if}

  {#if creating}
    <section class="surface-card create">
      <label class="field">
        <span class="field-label">{$t("llm.accounts.cli")}</span>
        <select class="input" bind:value={newCli}>
          <option value="claude">claude</option>
          <option value="codex">codex</option>
        </select>
      </label>
      <label class="field grow">
        <span class="field-label">{$t("llm.accounts.label")}</span>
        <input class="input" type="text" bind:value={newLabel} placeholder={$t("llm.accounts.label_hint")} />
      </label>
      <label class="share">
        <input class="checkbox" type="checkbox" bind:checked={newShare} />
        {$t("llm.accounts.share_profile")}
      </label>
      <button class="button active" type="button" disabled={busy || !newLabel.trim()} onclick={create}>
        {$t("llm.accounts.create")}
      </button>
      <p class="field-hint">{$t("llm.accounts.create_hint")}</p>
    </section>
  {/if}

  <LimitsStrip />

  <section class="surface-card rotation">
    <label class="rot-toggle">
      <input
        class="checkbox"
        type="checkbox"
        checked={snapshot.rotation.enabled}
        onchange={(e) =>
          guard(() =>
            setRotation({ ...snapshot.rotation, enabled: (e.currentTarget as HTMLInputElement).checked }),
          )}
      />
      <span>
        <strong>{$t("llm.accounts.rotation")}</strong>
        <span class="rot-sub">
          {$t("llm.accounts.rotation_hint")}
          {Math.round(snapshot.rotation.threshold * 100)}% · {Math.round(snapshot.rotation.cooldown_s / 60)} min
        </span>
      </span>
    </label>
    <p class="rot-warn">{$t("llm.accounts.rotation_warning")}</p>
  </section>

  {#if getAccountsState() === "unavailable" && snapshot.accounts.length === 0}
    <div class="empty-state">
      <div class="empty-state-title">{$t("llm.accounts.unavailable_title")}</div>
      <div class="empty-state-hint"><code>{getAccountsError()}</code></div>
    </div>
  {:else if snapshot.accounts.length === 0}
    <div class="empty-state">
      <div class="empty-state-title">{$t("llm.accounts.empty_title")}</div>
      <div class="empty-state-hint">{$t("llm.accounts.empty_hint")}</div>
    </div>
  {:else}
    <h2 class="section-title">{$t("llm.accounts.chain")}</h2>
    <p class="section-note">{$t("llm.accounts.chain_hint")}</p>
    <AccountList
      accounts={snapshot.accounts}
      chain={snapshot.chain}
      available={snapshot.quota_available}
      {nowMs}
      {busy}
      onreorder={(chain) => guard(() => setChain(chain))}
      onactivate={(id) => guard(() => activateAccount(id))}
      onlogin={(id) => guard(() => loginAccount(id))}
      ontoggle={(id, disabled) => guard(() => setDisabled(id, disabled))}
      onremove={(id, purge) => guard(() => removeAccount(id, purge))}
      onsandbox={(id, sandbox) => guard(() => setSandbox(id, sandbox))}
    />
  {/if}

  {#if switches.length > 0}
    <h2 class="section-title">{$t("llm.accounts.switch_log")}</h2>
    <ol class="timeline">
      {#each switches as s (s.at_ms + s.to)}
        <li>
          <span class="when">{timeOf(s.at_ms)}</span>
          <span class="what">{s.from} → <strong>{s.to}</strong></span>
          {#if s.why}<code class="why">{s.why}</code>{/if}
        </li>
      {/each}
    </ol>
  {/if}

  <AcpAgents />

  </details>

  <section id="usage" bind:this={usageSection} tabindex="-1" aria-labelledby="usage-heading">
  <h2 id="usage-heading" class="section-title">{$t("llm.accounts.usage")}</h2>
  {#if getUsageState() === "unavailable"}
    <div class="empty-state">
      <div class="empty-state-title">{$t("llm.accounts.usage_unavailable")}</div>
      <div class="empty-state-hint"><code>{getUsageError()}</code></div>
      <button class="button" type="button" onclick={() => loadUsage()}>
        {$t("llm.accounts.refresh")}
      </button>
    </div>
  {:else}
    <UsageDashboard report={usage} />
  {/if}
  </section>
</div>

<style>
  .advanced-accounts { margin-block: var(--space-5); }
  .advanced-accounts > summary { cursor: pointer; padding-block: var(--space-4); font-weight: 600; }
  .advanced-accounts > .button { margin-bottom: var(--space-4); }
  .connection-links, .return-draft { font-size: var(--text-sm); }
  #usage { scroll-margin-top: var(--space-4); }
  #usage:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset); }
  .accounts-page {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .accounts-page :global(.page-head) {
    flex-wrap: wrap;
  }
  .head-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .detected {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    margin: 0 0 var(--space-3);
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .create {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
    flex-wrap: wrap;
    padding: var(--space-4);
    margin-bottom: var(--space-3);
  }
  .create .grow {
    flex: 1;
    min-width: 180px;
  }
  .create .field-hint {
    flex-basis: 100%;
    margin: 0;
  }
  .share {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
    padding-bottom: 6px;
  }
  .rotation {
    padding: var(--space-4);
    margin-bottom: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .rot-toggle {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    font-size: var(--text-sm);
    color: var(--text);
  }
  .rot-sub {
    display: block;
    font-size: var(--text-caption);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .rot-warn {
    margin: 0;
    font-size: var(--text-caption);
    color: var(--text-dim);
    max-width: 70ch;
  }
  .section-title {
    margin-top: var(--space-5);
  }
  .section-note {
    margin: calc(-1 * var(--space-2)) 0 var(--space-3);
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .timeline {
    list-style: none;
    margin: 0 0 var(--space-4);
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .timeline li {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .when {
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    font-size: var(--text-caption);
  }
  .why {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
</style>
