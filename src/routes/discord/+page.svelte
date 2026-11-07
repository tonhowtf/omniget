<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import TgIcon from "$lib/telegram/TgIcon.svelte";
  import SignIn from "$lib/discord/SignIn.svelte";
  import TosNotice from "$lib/discord/TosNotice.svelte";
  import AccountMenu from "$lib/discord/AccountMenu.svelte";
  import ServerRail from "$lib/discord/ServerRail.svelte";
  import ChannelList from "$lib/discord/ChannelList.svelte";
  import MessageView from "$lib/discord/MessageView.svelte";
  import DownloadDialog from "$lib/discord/DownloadDialog.svelte";
  import ExportDialog from "$lib/discord/ExportDialog.svelte";
  import { errKey } from "$lib/discord/api";
  import type { DcChannel } from "$lib/discord/api";
  import { attachEvents, boot, connection, dcState, detachEvents, getSnapshot, logoutAccount, reconnect, refreshStatus, resetData, switchTo } from "$lib/discord/client.svelte";

  const LAST_KEY = "discord.last";

  let adding = $state(false);
  let place = $state("dms");
  let lastByPlace = $state<Record<string, string>>(readLast());
  let dialog = $state<"download" | "export" | null>(null);
  let outputDir = $state("");
  let now = $state(Date.now());

  let status = $derived(dcState.status);
  let accounts = $derived(status?.accounts ?? []);
  let conn = $derived(connection());
  let snap = $derived(getSnapshot());
  let guild = $derived(place === "dms" ? null : (snap?.guilds.find((g) => g.id === place) ?? null));
  let channels = $derived<DcChannel[]>(place === "dms" ? (snap?.dms ?? []) : (guild?.channels ?? []));
  let channelId = $derived(lastByPlace[place] ?? channels.find((c) => c.kind !== 4)?.id ?? null);
  let channel = $derived(channels.find((c) => c.id === channelId && c.kind !== 4) ?? null);
  let placeName = $derived(guild?.name ?? $t("discord.rail.dms"));
  let retryIn = $derived(conn.state === "reconnecting" && conn.retry_in_ms ? conn.retry_in_ms : 0);
  let signedOut = $derived(dcState.booted && (!accounts.length || adding));

  function readLast(): Record<string, string> {
    try {
      const v = JSON.parse(localStorage.getItem(LAST_KEY) ?? "{}");
      return v && typeof v === "object" ? v : {};
    } catch {
      return {};
    }
  }

  function saveLast() {
    try {
      localStorage.setItem(LAST_KEY, JSON.stringify({ ...lastByPlace, _place: place }));
    } catch {
    }
  }

  function selectPlace(id: string) {
    place = id;
    saveLast();
  }

  function selectChannel(c: DcChannel) {
    lastByPlace = { ...lastByPlace, [place]: c.id };
    saveLast();
  }

  $effect(() => {
    if (snap && place !== "dms" && !snap.guilds.some((g) => g.id === place)) place = "dms";
  });

  let retryStart = $state(Date.now());
  let retrySecs = $derived(retryIn ? Math.max(0, Math.ceil((retryIn - (now - retryStart)) / 1000)) : 0);

  $effect(() => {
    if (!retryIn) return;
    retryStart = Date.now();
    now = retryStart;
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  async function signedIn() {
    adding = false;
    resetData();
    await refreshStatus();
  }

  async function doSwitch(id: string) {
    try {
      await switchTo(id);
    } catch (e) {
      showToast("error", $t(`discord.errors.${errKey(e)}`));
    }
  }

  async function doLogout(id: string) {
    try {
      await logoutAccount(id);
      showToast("success", $t("discord.account.logged_out"));
    } catch (e) {
      showToast("error", $t(`discord.errors.${errKey(e)}`));
    }
  }

  onMount(() => {
    outputDir = getSettings()?.download?.default_output_dir ?? "";
    const last = lastByPlace._place;
    if (last) place = last;
    void attachEvents().then(() => boot());
  });

  onDestroy(() => detachEvents());
</script>

<svelte:head><title>Discord · OmniGet</title></svelte:head>

<section class="dc" class:full={!!snap && !signedOut}>
  <header class="dc-head">
    <NavIcon icon="discord" size={40} />
    <div class="dc-title">
      <h1>{$t("discord.title")}</h1>
      <p>{$t("discord.subtitle")}</p>
    </div>
    {#if accounts.length && !adding}
      <AccountMenu {accounts} active={status?.active ?? null} onSwitch={(id) => void doSwitch(id)} onAdd={() => (adding = true)} onLogout={(id) => void doLogout(id)} />
    {/if}
  </header>

  {#if !dcState.booted}
    <div class="boot"><span class="spinner"></span><span>{$t("discord.boot")}</span></div>
  {:else if signedOut}
    <SignIn onSignedIn={() => void signedIn()} onCancel={accounts.length ? () => (adding = false) : undefined} />
  {:else}
    {#if conn.state === "failed"}
      <div class="notice notice-warning banner" role="alert">
        <TgIcon name="warn" size={16} />
        <div class="notice-body">
          <div class="notice-title">{$t(conn.reason === "unauthorized" ? "discord.conn.expired_title" : "discord.conn.failed_title")}</div>
          <div class="notice-text">{$t(conn.reason === "unauthorized" ? "discord.conn.expired_hint" : "discord.conn.failed_hint", { code: conn.reason ?? "" })}</div>
        </div>
        {#if conn.reason === "unauthorized"}
          <button class="btn btn-primary btn-sm" onclick={() => (adding = true)}>{$t("discord.signin.sign_in")}</button>
        {:else}
          <button class="btn btn-ghost btn-sm" onclick={() => void reconnect()}>{$t("discord.common.retry")}</button>
        {/if}
      </div>
    {:else if conn.state === "reconnecting"}
      <div class="notice notice-info banner" role="status">
        <span class="spinner"></span>
        <div class="notice-body">
          <div class="notice-text">{retrySecs ? $t("discord.conn.reconnecting_in", { s: retrySecs }) : $t("discord.conn.reconnecting")}</div>
        </div>
      </div>
    {:else if conn.state === "offline" && status?.active}
      <div class="notice notice-info banner" role="status">
        <TgIcon name="plug" size={16} />
        <div class="notice-body"><div class="notice-text">{$t("discord.conn.offline")}</div></div>
        <button class="btn btn-ghost btn-sm" onclick={() => void reconnect()}>{$t("discord.conn.connect")}</button>
      </div>
    {/if}
    {#if dcState.bootError}
      <div class="notice notice-warning banner">
        <div class="notice-body"><div class="notice-text">{$t(`discord.errors.${dcState.bootError}`)}</div></div>
        <button class="btn btn-ghost btn-sm" onclick={() => void boot()}>{$t("discord.common.retry")}</button>
      </div>
    {/if}

    <TosNotice />

    {#if !snap}
      <div class="boot"><span class="spinner"></span><span>{$t("discord.conn.loading")}</span></div>
    {:else}
      <div class="client">
        <ServerRail guilds={snap.guilds} dms={snap.dms} readStates={snap.read_states} selected={place} onSelect={selectPlace} />
        <ChannelList title={placeName} {channels} readStates={snap.read_states} selected={channel?.id ?? null} isDms={place === "dms"} onSelect={selectChannel} />
        {#if channel}
          <MessageView {channel} me={snap.me} place={placeName} {outputDir} onDownload={() => (dialog = "download")} onExport={() => (dialog = "export")} />
        {:else}
          <div class="empty-state pick">
            <TgIcon name="inbox" size={32} />
            <div class="empty-state-title">{$t("discord.view.pick_title")}</div>
            <div class="empty-state-hint">{$t("discord.view.pick_hint")}</div>
          </div>
        {/if}
      </div>
    {/if}
  {/if}
</section>

{#if channel && dialog === "download"}
  <DownloadDialog {channel} place={placeName} bind:outputDir onclose={() => (dialog = null)} />
{:else if channel && dialog === "export"}
  <ExportDialog {channel} place={placeName} bind:outputDir onclose={() => (dialog = null)} />
{/if}

<style>
  .dc {
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

  .dc.full {
    max-width: 1440px;
    gap: var(--space-3);
    padding-bottom: var(--space-4);
    overflow: hidden;
  }

  .dc-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .dc-title {
    flex: 1;
    min-width: 0;
  }

  .dc-title h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .dc-title p {
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

  .banner {
    align-items: center;
  }

  .client {
    flex: 1;
    min-height: 420px;
    display: grid;
    grid-template-columns: 56px 240px minmax(0, 1fr);
    gap: var(--space-3);
  }

  .pick {
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  @media (max-width: 820px) {
    .client {
      grid-template-columns: 56px 200px minmax(0, 1fr);
    }
  }
</style>
