<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import * as api from "$lib/phonechat/api";
  import type { WaChat } from "$lib/phonechat/api";
  import { attach, detach, refreshStatus, wa } from "$lib/phonechat/session.svelte";
  import LinkPanel from "$lib/phonechat/LinkPanel.svelte";
  import ChatList from "$lib/phonechat/ChatList.svelte";
  import ChatView from "$lib/phonechat/ChatView.svelte";
  import WaIcon from "$lib/phonechat/WaIcon.svelte";
  import RulesPanel from "$lib/phonechat/RulesPanel.svelte";

  let selected = $state<WaChat | null>(null);
  let outputDir = $state("");
  let confirmLogout = $state(false);
  let loggingOut = $state(false);
  let actionError = $state<string | null>(null);
  let autoTried = false;
  let showRules = $state(false);

  let status = $derived(wa.status);
  let linked = $derived(!!status?.linked);
  let online = $derived(status?.state === "connected");
  let accountName = $derived(status?.account?.push_name || (status?.account?.jid ? `+${status.account.jid.split("@")[0]}` : ""));

  function errorLabel(raw: string): string {
    const k = api.knownError(raw);
    return k ? $t(`phonechat.errors.${k}`) : raw;
  }

  async function reconnect() {
    actionError = null;
    try {
      await api.connect();
    } catch (e) {
      actionError = api.errText(e);
    }
  }

  async function doLogout() {
    loggingOut = true;
    actionError = null;
    try {
      await api.logout();
      selected = null;
      confirmLogout = false;
    } catch (e) {
      actionError = api.errText(e);
    } finally {
      loggingOut = false;
    }
  }

  $effect(() => {
    if (!status || autoTried) return;
    if (status.linked && status.state === "idle") {
      autoTried = true;
      void reconnect();
    }
  });

  onMount(() => {
    outputDir = getSettings()?.download?.default_output_dir ?? "";
    void attach().then(() => refreshStatus());
  });

  onDestroy(() => detach());
</script>

<svelte:head><title>{$t("phonechat.title")} · OmniGet</title></svelte:head>

<section class="wa" class:fill={linked}>
  <header class="wa-head">
    <NavIcon icon="phonechat" size={40} />
    <div class="wa-title">
      <h1>{$t("phonechat.title")}</h1>
      <p>{$t("phonechat.subtitle")}</p>
    </div>
    {#if linked}
      <div class="wa-account">
        <span class="conn" class:ok={online} class:warn={status?.state === "offline"}>
          <span class="dot" aria-hidden="true"></span>
          {$t(`phonechat.state.${status?.state ?? "idle"}`)}
        </span>
        {#if accountName}<span class="account-name" title={status?.account?.jid}>{accountName}</span>{/if}
        <button class="btn btn-secondary btn-sm" onclick={() => (showRules = true)}><WaIcon name="bolt" /> {$t("phonechat.rules.open")}</button>
        {#if status?.state === "offline" || status?.state === "idle"}
          <button class="btn btn-ghost btn-sm" onclick={reconnect}><WaIcon name="refresh" /> {$t("phonechat.reconnect")}</button>
        {/if}
        {#if confirmLogout}
          <span class="confirm">{$t("phonechat.logout_confirm")}</span>
          <button class="btn btn-destructive btn-sm" onclick={doLogout} disabled={loggingOut}>
            {#if loggingOut}<span class="spinner"></span>{/if}
            {$t("phonechat.logout")}
          </button>
          <button class="btn btn-ghost btn-sm" onclick={() => (confirmLogout = false)}>{$t("phonechat.cancel")}</button>
        {:else}
          <button class="btn btn-ghost btn-sm" onclick={() => (confirmLogout = true)}><WaIcon name="logout" /> {$t("phonechat.logout")}</button>
        {/if}
      </div>
    {/if}
  </header>

  {#if wa.loading}
    <div class="boot"><span class="spinner"></span><span>{$t("phonechat.boot")}</span></div>
  {:else if !status}
    <div class="notice notice-warning boot-err">
      <div class="notice-body">
        <div class="notice-title">{$t("phonechat.boot_error")}</div>
        <div class="notice-text">{wa.error}</div>
      </div>
      <button class="btn btn-ghost btn-sm" onclick={() => void refreshStatus()}>{$t("phonechat.retry")}</button>
    </div>
  {:else if !linked}
    {#if status.state === "logged_out"}
      <div class="notice notice-info">
        <div class="notice-body">
          <div class="notice-title">{$t("phonechat.logged_out_title")}</div>
          <div class="notice-text">{$t("phonechat.logged_out_hint")}</div>
        </div>
      </div>
    {/if}
    <LinkPanel />
  {:else}
    {#if status.state === "offline"}
      <div class="notice notice-warning strip" role="status">
        <WaIcon name="wifi_off" size={16} />
        <div class="notice-body">
          <div class="notice-text">{status.error ? errorLabel(status.error) : $t("phonechat.offline_hint")}</div>
        </div>
      </div>
    {:else if status.state === "starting"}
      <div class="notice notice-info strip" role="status">
        <span class="spinner"></span>
        <div class="notice-body"><div class="notice-text">{$t("phonechat.connecting_hint")}</div></div>
      </div>
    {:else if status.sync_progress !== null && status.sync_progress < 100}
      <div class="notice notice-info strip" role="status">
        <span class="spinner"></span>
        <div class="notice-body">
          <div class="notice-text">{$t("phonechat.syncing", { progress: status.sync_progress, messages: wa.syncMessages })}</div>
        </div>
      </div>
    {/if}
    {#if actionError}
      <div class="notice notice-warning strip" role="alert">
        <div class="notice-body"><div class="notice-text">{errorLabel(actionError)}</div></div>
      </div>
    {/if}

    <div class="panes card">
      <ChatList bind:selected />
      {#if selected}
        <ChatView chat={selected} {online} bind:outputDir />
      {:else}
        <div class="pick">
          <div class="empty-state">
            <WaIcon name="download" size={32} />
            <div class="empty-state-title">{$t("phonechat.pick_title")}</div>
            <div class="empty-state-hint">{$t("phonechat.pick_hint")}</div>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</section>

{#if showRules}
  <RulesPanel onclose={() => (showRules = false)} />
{/if}

<style>
  .wa {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    height: 100%;
    max-width: 1120px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-6);
    min-height: 0;
    overflow-y: auto;
  }

  .wa.fill {
    overflow: hidden;
  }

  .wa-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .wa-title {
    flex: 1;
    min-width: 200px;
  }

  .wa-title h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .wa-title p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .wa-account {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .conn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    background: var(--text-muted);
  }

  .conn.ok .dot {
    background: var(--success);
  }

  .conn.warn .dot {
    background: var(--warning);
  }

  .account-name {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .confirm {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .boot {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-8) 0;
    color: var(--text-muted);
    font-size: var(--text-base);
  }

  .boot-err {
    max-width: 560px;
    margin: 0 auto;
  }

  .strip {
    align-items: center;
    gap: var(--space-2);
  }

  .panes {
    flex: 1;
    min-height: 420px;
    display: grid;
    grid-template-columns: minmax(240px, 320px) minmax(0, 1fr);
    padding: 0;
    overflow: hidden;
  }

  .pick {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
  }
</style>
