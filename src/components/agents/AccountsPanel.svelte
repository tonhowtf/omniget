<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SettingGroup from "$components/settings/rebrand/SettingGroup.svelte";
  import AgentLogo from "./AgentLogo.svelte";
  import {
    activateAccount,
    createAccount,
    detectClis,
    getAccounts,
    getAccountsError,
    getAccountsState,
    getDetected,
    isBusy,
    loadAccounts,
    loginAccount,
    removeAccount,
    type AccountView,
    type CliKind,
    type UsageWindow,
  } from "$lib/stores/agent-accounts-store.svelte";

  const CLIS: { cli: CliKind; name: string }[] = [
    { cli: "claude", name: "Claude" },
    { cli: "codex", name: "Codex" },
  ];

  let snapshot = $derived(getAccounts());
  let busy = $derived(isBusy());
  let detected = $derived(getDetected());
  let adding = $state<CliKind | null>(null);
  let newLabel = $state("");
  let confirming = $state<string | null>(null);

  onMount(() => {
    void loadAccounts();
    void detectClis();
  });

  async function guard(run: () => Promise<void>) {
    try {
      await run();
    } catch (error) {
      showToast("error", String(error));
    }
  }

  function startAdding(cli: CliKind, name: string) {
    const count = snapshot.accounts.filter((account) => account.cli === cli).length;
    adding = cli;
    newLabel = count > 0 ? `${name} ${count + 1}` : name;
  }

  async function add() {
    const cli = adding;
    const label = newLabel.trim();
    if (!cli || !label) return;
    await guard(async () => {
      await createAccount(cli, label, true);
      adding = null;
      newLabel = "";
      const created = getAccounts().accounts.find((account) => account.cli === cli && account.label === label);
      if (created) await loginAccount(created.id);
    });
  }

  function installed(cli: CliKind): boolean {
    return detected.length === 0 || detected.some((entry) => entry.cli === cli);
  }

  function percent(window: UsageWindow): number {
    return Math.round(Math.min(1, Math.max(0, window.used ?? 0)) * 100);
  }

  function level(value: number): "ok" | "warn" | "high" {
    return value >= 95 ? "high" : value >= 80 ? "warn" : "ok";
  }

  function resetsIn(window: UsageWindow): string {
    if (!window.resets_at_ms) return "";
    const minutes = Math.max(0, Math.round((window.resets_at_ms - Date.now()) / 60_000));
    const hours = Math.floor(minutes / 60);
    const time = hours >= 24 ? `${Math.floor(hours / 24)} d` : hours > 0 ? `${hours} h` : `${minutes} min`;
    return $t("settings.agents.resets_in", { time }) as string;
  }

  function meters(account: AccountView): UsageWindow[] {
    return account.windows.filter((window) => window.used !== null);
  }
</script>

<SettingGroup id="accounts" icon="person" title={$t("settings.agents.accounts_title") as string} description={$t("settings.ai.accounts_desc") as string}>
  {#if getAccountsState() === "unavailable" && snapshot.accounts.length === 0}
    <p class="note" role="alert"><bdi>{getAccountsError()}</bdi></p>
  {/if}
  <div class="grid" data-setting-key="settings.agents.accounts_title">
    {#each snapshot.accounts as account (account.id)}
      <article class="account" class:active={account.active} class:off={account.disabled}>
        <header>
          <AgentLogo id={account.cli} name={account.label} />
          <div class="ident">
            <h3><bdi>{account.label}</bdi></h3>
            <p>{CLIS.find((entry) => entry.cli === account.cli)?.name ?? account.cli}</p>
          </div>
          {#if account.active}<span class="chip">{$t("settings.agents.in_use")}</span>{/if}
        </header>
        {#if account.last_error}<p class="note error"><bdi>{account.last_error}</bdi></p>{/if}
        {#if meters(account).length > 0}
          <div class="meters">
            {#each meters(account) as window (window.window)}
              {@const used = percent(window)}
              <div class="meter">
                <div class="meter-text">
                  <span>{$t(`agents.accounts.window_${window.window}`)}</span>
                  <span class="meter-value">{used}%</span>
                </div>
                <div class="bar" role="progressbar" aria-valuenow={used} aria-valuemin="0" aria-valuemax="100" aria-label={$t(`agents.accounts.window_${window.window}`) as string}>
                  <span class="fill" data-level={level(used)} style:inline-size={`${used}%`}></span>
                </div>
                {#if resetsIn(window)}<small>{resetsIn(window)}</small>{/if}
              </div>
            {/each}
          </div>
        {/if}
        <footer>
          {#if confirming === account.id}
            <span class="confirm">{$t("settings.agents.remove_confirm")}</span>
            <button type="button" class="btn btn-secondary btn-sm" onclick={() => (confirming = null)}>{$t("settings.agents.keep")}</button>
            <button type="button" class="btn btn-secondary btn-sm danger" disabled={busy} onclick={() => { confirming = null; void guard(() => removeAccount(account.id, false)); }}>{$t("settings.agents.remove")}</button>
          {:else}
            {#if !account.active}
              <button type="button" class="btn btn-primary btn-sm" disabled={busy} onclick={() => guard(() => activateAccount(account.id))}>{$t("settings.agents.use_this")}</button>
            {/if}
            <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => guard(() => loginAccount(account.id))}>{$t("settings.agents.sign_in")}</button>
            <button type="button" class="link" onclick={() => (confirming = account.id)}>{$t("settings.agents.remove")}</button>
          {/if}
        </footer>
      </article>
    {/each}
  </div>

  <h3 class="add-title">{$t("settings.agents.add_title")}</h3>
  <div class="add-row">
    {#each CLIS as entry (entry.cli)}
      <button type="button" class="add" class:selected={adding === entry.cli} aria-expanded={adding === entry.cli} onclick={() => (adding === entry.cli ? (adding = null) : startAdding(entry.cli, entry.name))}>
        <AgentLogo id={entry.cli} name={entry.name} size={22} />
        <span class="add-text">
          <span class="add-name">{entry.name}</span>
          {#if !installed(entry.cli)}<small>{$t("settings.agents.not_installed")}</small>{/if}
        </span>
        <span class="plus" aria-hidden="true">+</span>
      </button>
    {/each}
  </div>
  {#if adding}
    <form class="add-form" onsubmit={(event) => { event.preventDefault(); void add(); }}>
      <label>
        <span>{$t("settings.agents.account_name")}</span>
        <input class="input-text" type="text" bind:value={newLabel} />
      </label>
      <button type="submit" class="btn btn-primary btn-sm" disabled={busy || !newLabel.trim()}>{$t("settings.agents.add_action")}</button>
    </form>
  {/if}
</SettingGroup>

<style>
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 280px), 1fr)); gap: var(--space-2); }
  .account {
    display: flex; flex-direction: column; gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--content-border);
    border-radius: var(--radius-md);
    background: var(--primary);
  }
  .account.active { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
  .account.off { opacity: 0.6; }
  header { display: flex; align-items: center; gap: var(--space-2); }
  .ident { flex: 1; min-inline-size: 0; }
  h3 { margin: 0; font-size: 13px; line-height: 18px; font-weight: 600; color: var(--text); overflow-wrap: anywhere; }
  .ident p { margin: 0; font-size: 12px; line-height: 17px; color: var(--text-muted); }
  .chip { flex: none; padding: 2px var(--space-2); border-radius: var(--radius-full); background: var(--accent); color: var(--on-accent); font-size: 11px; line-height: 16px; font-weight: 600; }
  .meters { display: flex; flex-direction: column; gap: var(--space-2); }
  .meter { display: flex; flex-direction: column; gap: 3px; font-size: 12px; color: var(--text-muted); }
  .meter-text { display: flex; justify-content: space-between; gap: var(--space-2); }
  .meter-value { font-variant-numeric: tabular-nums; color: var(--text); }
  .bar { block-size: 6px; border-radius: var(--radius-full); background: var(--surface-hi-src, var(--surface-src)); overflow: hidden; }
  .fill { display: block; block-size: 100%; border-radius: inherit; background: var(--success); }
  .fill[data-level="warn"] { background: var(--warning); }
  .fill[data-level="high"] { background: var(--error); }
  small { font-size: 11px; line-height: 15px; color: var(--text-muted); }
  footer { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); margin-block-start: auto; }
  .confirm { flex-basis: 100%; font-size: 12px; color: var(--text); }
  .danger { color: var(--error); }
  .link { margin-inline-start: auto; padding: 0; border: 0; background: none; color: var(--text-muted); font: inherit; font-size: 12px; cursor: pointer; }
  .link:hover { color: var(--error); }
  .add-title { margin-block: var(--space-4) var(--space-2); }
  .add-row { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 200px), 1fr)); gap: var(--space-2); }
  .add {
    display: flex; align-items: center; gap: var(--space-2);
    padding: var(--space-2);
    border: 1px dashed var(--input-border);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .add:hover { background: var(--surface-hi); }
  .add.selected { border-style: solid; border-color: var(--accent); }
  .add-text { flex: 1; display: flex; flex-direction: column; min-inline-size: 0; }
  .add-name { font-size: 13px; font-weight: 600; }
  .plus { font-size: 18px; line-height: 1; color: var(--text-muted); }
  .add-form { display: flex; flex-wrap: wrap; align-items: flex-end; gap: var(--space-2); margin-block-start: var(--space-2); }
  .add-form label { flex: 1 1 14rem; display: flex; flex-direction: column; gap: var(--space-1); font-size: 12px; color: var(--text-muted); }
  .note { margin: 0 0 var(--space-2); font-size: 12px; color: var(--text-muted); overflow-wrap: anywhere; }
  .note.error { margin: 0; color: var(--error); }
</style>
