<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import StatusStrip from "$lib/telegram/StatusStrip.svelte";
  import ConnectWizard from "$lib/telegram/ConnectWizard.svelte";
  import ConnectionPanel from "$lib/telegram/ConnectionPanel.svelte";
  import LoginPanel from "$lib/telegram/LoginPanel.svelte";
  import ChatBrowser from "$lib/telegram/ChatBrowser.svelte";
  import LinkDownload from "$lib/telegram/LinkDownload.svelte";
  import ExportPanel from "$lib/telegram/ExportPanel.svelte";
  import AccountPanel from "$lib/telegram/AccountPanel.svelte";
  import WatchRules from "$lib/telegram/WatchRules.svelte";
  import UploadPanel from "$lib/telegram/UploadPanel.svelte";
  import ClonePanel from "$lib/telegram/ClonePanel.svelte";
  import { errText, type TelegramChat } from "$lib/telegram/api";
  import { attachNet, detachNet, netState, scanNet, hasWorkingRoute } from "$lib/telegram/net.svelte";
  import { attachSessionEvents, detachSessionEvents, ensureSession, resetSession } from "$lib/telegram/session.svelte";

  // Screen flow: the first visit walks through the connection wizard, then
  // login; later visits check the saved session while the routes are
  // re-probed in the background.
  type Stage = "boot" | "wizard" | "login" | "ready";
  type Tab = "login" | "chats" | "watch" | "upload" | "links" | "export" | "clone" | "connection" | "account";

  const WIZARD_DONE_KEY = "telegram.wizard_done";

  let enabled = $derived(getSettings()?.telegram?.enabled ?? true);
  let stage = $state<Stage>("boot");
  let tab = $state<Tab>("login");
  let bootError = $state<string | null>(null);
  let outputDir = $state("");
  let uploadChat = $state<TelegramChat | null>(null);
  let cloneSource = $state<TelegramChat | null>(null);

  let tabs = $derived<Tab[]>(stage === "ready" ? ["chats", "watch", "upload", "links", "export", "clone", "connection", "account"] : ["login", "connection"]);

  function wizardDone(): boolean {
    try {
      return localStorage.getItem(WIZARD_DONE_KEY) === "1";
    } catch {
      return false;
    }
  }

  function markWizardDone() {
    try {
      localStorage.setItem(WIZARD_DONE_KEY, "1");
    } catch {
      /* private storage */
    }
  }

  async function checkSession() {
    stage = "boot";
    bootError = null;
    const s = netState.status;
    const scan = s && s.scanned_at ? Promise.resolve(s) : scanNet();
    try {
      // Always the first engine call: it restores the saved session.
      await ensureSession(true);
      stage = "ready";
      tab = "chats";
    } catch (e) {
      const msg = errText(e);
      if (!/not_authenticated/i.test(msg)) {
        await scan;
        if (!hasWorkingRoute(netState.status)) {
          stage = "wizard";
          return;
        }
        bootError = msg;
      }
      stage = "login";
      tab = "login";
    }
  }

  function afterWizard() {
    markWizardDone();
    void checkSession();
  }

  function openConnection() {
    if (stage === "wizard" || stage === "boot") stage = "login";
    tab = "connection";
  }

  function onAuthenticated() {
    stage = "ready";
    tab = "chats";
    bootError = null;
  }

  function onLoggedOut() {
    resetSession();
    stage = "login";
    tab = "login";
  }

  onMount(() => {
    outputDir = getSettings()?.download?.default_output_dir ?? "";
    void attachSessionEvents();
    void attachNet().then(() => {
      if (!enabled) return;
      if (wizardDone()) void checkSession();
      else stage = "wizard";
    });
  });

  onDestroy(() => {
    detachNet();
    detachSessionEvents();
  });
</script>

<svelte:head><title>Telegram · OmniGet</title></svelte:head>

{#if !enabled}
  <section class="tg off">
    <div class="empty-state">
      <NavIcon icon="telegram" size={64} />
      <div class="empty-state-title">{$t("telegram.disabled_title")}</div>
      <div class="empty-state-hint">{$t("superpowers.disabled_hint")}</div>
      <a class="btn btn-secondary" href="/settings?tab=advanced">{$t("superpowers.open_settings")}</a>
    </div>
  </section>
{:else}
  <section class="tg">
    <header class="tg-head">
      <NavIcon icon="telegram" size={40} />
      <div class="tg-title">
        <h1>Telegram</h1>
        <p>{$t("telegram.subtitle")}</p>
      </div>
    </header>

    <StatusStrip onOpenConnection={openConnection} />

    {#if stage === "boot"}
      <div class="boot">
        <span class="spinner"></span>
        <span>{$t("telegram.boot")}</span>
      </div>
    {:else if stage === "wizard"}
      <ConnectWizard onDone={afterWizard} onOpenConnection={openConnection} />
    {:else}
      <div class="tabs" role="tablist" aria-label="Telegram">
        {#each tabs as id (id)}
          <button class="tab" role="tab" aria-selected={tab === id} class:active={tab === id} onclick={() => (tab = id)}>
            {$t(`telegram.tabs.${id}`)}
          </button>
        {/each}
      </div>

      <div class="tab-body" class:fill={tab === "chats"}>
        {#if tab === "connection"}
          <ConnectionPanel />
          {#if stage === "login"}
            <div class="back-login">
              <button class="btn btn-primary" onclick={() => (tab = "login")}>{$t("telegram.login.back_to_login")}</button>
            </div>
          {/if}
        {:else if stage === "login"}
          {#if bootError}
            <div class="notice notice-warning boot-err">
              <div class="notice-body">
                <div class="notice-title">{$t("telegram.boot_error")}</div>
                <div class="notice-text">{bootError}</div>
              </div>
              <button class="btn btn-ghost btn-sm" onclick={() => void checkSession()}>{$t("telegram.common.retry")}</button>
            </div>
          {/if}
          <LoginPanel {onAuthenticated} onTrouble={openConnection} />
        {:else if tab === "chats"}
          <ChatBrowser bind:outputDir onClone={(c) => { cloneSource = c; tab = "clone"; }} />
        {:else if tab === "watch"}
          <WatchRules />
        {:else if tab === "upload"}
          <section class="upload-tab">
            <header>
              <h3>{$t("telegram.upload.title")}</h3>
              <p>{$t("telegram.upload.hint")}</p>
            </header>
            <UploadPanel bind:chat={uploadChat} />
          </section>
        {:else if tab === "clone"}
          <ClonePanel bind:source={cloneSource} />
        {:else if tab === "links"}
          <LinkDownload bind:outputDir />
        {:else if tab === "export"}
          <ExportPanel bind:outputDir />
        {:else if tab === "account"}
          <AccountPanel {onLoggedOut} onSwitched={() => { resetSession(); void checkSession(); }} />
        {/if}
      </div>
    {/if}
  </section>
{/if}

<style>
  .tg {
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

  .tg.off {
    justify-content: center;
  }

  .tg-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .tg-title h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .tg-title p {
    margin: 0;
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

  .tabs {
    display: flex;
    gap: 2px;
    overflow-x: auto;
    scrollbar-width: none;
    border-bottom: var(--hairline) solid var(--separator);
  }

  .tab {
    position: relative;
    flex-shrink: 0;
    white-space: nowrap;
    padding: var(--space-2) var(--space-3);
    border: none;
    background: none;
    font: inherit;
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--text-dim);
    cursor: pointer;
  }

  .tab:hover {
    color: var(--text);
  }

  .tab.active {
    color: var(--text);
  }

  .tab.active::after {
    content: "";
    position: absolute;
    left: var(--space-3);
    right: var(--space-3);
    bottom: -1px;
    height: 2px;
    border-radius: 2px;
    background: var(--accent);
  }

  .tab:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
    border-radius: var(--radius-sm);
  }

  .tab-body {
    min-height: 0;
  }

  .tab-body.fill {
    flex: 1;
    display: flex;
    flex-direction: column;
  }

  .tab-body.fill > :global(*) {
    flex: 1;
    min-height: 420px;
  }

  .boot-err {
    max-width: 560px;
    margin: 0 auto var(--space-4);
  }

  .upload-tab {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 680px;
  }

  .upload-tab h3 {
    margin: 0 0 2px;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .upload-tab p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.5;
  }

  .back-login {
    display: flex;
    justify-content: center;
    margin-top: var(--space-5);
  }
</style>
