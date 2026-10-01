<script lang="ts">
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { bestAvatar } from "$lib/tube/format";
  import { go } from "$lib/tube/ui.svelte";
  import TIcon from "../TIcon.svelte";
  import { session, signIn, signOut } from "./session.svelte";

  let open = $state(false);
  let root = $state<HTMLDivElement>();

  let acc = $derived(session.account);
  let initial = $derived((acc?.name ?? "?").trim().charAt(0).toUpperCase());

  async function login() {
    open = false;
    try {
      await signIn();
    } catch (e) {
      showToast("error", String(e));
    }
  }
  async function logout() {
    open = false;
    try {
      await signOut();
    } catch (e) {
      showToast("error", String(e));
    }
  }
  function outside(e: PointerEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }
</script>

<svelte:window onpointerdown={outside} />

<div class="acct" bind:this={root}>
  {#if acc?.logged_in}
    <button class="av" onclick={() => (open = !open)} aria-haspopup="menu" aria-expanded={open} aria-label={$t("tube.shell.account") as string}>
      {#if acc.avatar}
        <img class="yt-avatar" src={bestAvatar(acc.avatar, 32)} alt="" width="32" height="32" referrerpolicy="no-referrer" />
      {:else}
        <span class="ini">{initial}</span>
      {/if}
    </button>
  {:else}
    <button class="yt-btn yt-btn--outline signin" onclick={login} disabled={session.busy}>
      <span class="ring"><TIcon name="user" size={18} /></span>{$t("tube.shell.sign_in")}
    </button>
  {/if}

  {#if open && acc?.logged_in}
    <div class="yt-menu menu" role="menu" in:fly={{ y: -6, duration: 150, easing: cubicOut }}>
      <div class="head">
        {#if acc.avatar}
          <img class="yt-avatar" src={bestAvatar(acc.avatar, 40)} alt="" width="40" height="40" referrerpolicy="no-referrer" />
        {:else}
          <span class="ini big">{initial}</span>
        {/if}
        <div class="who">
          <strong class="yt-clamp-1">{acc.name ?? ""}</strong>
          {#if acc.handle}<span class="yt-clamp-1">{acc.handle}</span>{/if}
          {#if acc.channel_id}
            <button class="lnk" onclick={() => ((open = false), go({ name: "channel", id: acc.channel_id! }))}>{$t("tube.shell.view_channel")}</button>
          {/if}
        </div>
      </div>
      <div class="yt-menu-sep"></div>
      <button class="yt-menu-item" role="menuitem" onclick={login}><TIcon name="user" size={22} />{$t("tube.shell.switch_account")}</button>
      <button class="yt-menu-item" role="menuitem" onclick={logout}><TIcon name="logout" size={22} />{$t("tube.shell.sign_out")}</button>
      <div class="yt-menu-sep"></div>
      <button class="yt-menu-item" role="menuitem" onclick={() => ((open = false), go({ name: "settings" }))}><TIcon name="settings" size={22} />{$t("tube.shell.settings")}</button>
      <button class="yt-menu-item" role="menuitem" onclick={() => ((open = false), go({ name: "import" }))}><TIcon name="import" size={22} />{$t("tube.shell.import")}</button>
    </div>
  {/if}
</div>

<style>
  .acct {
    position: relative;
    display: flex;
    align-items: center;
  }
  .av {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: transparent;
    cursor: pointer;
  }
  .av:focus-visible {
    outline: 2px solid var(--yt-link);
  }
  .av img {
    width: 32px;
    height: 32px;
  }
  .ini {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: var(--yt-accent);
    color: var(--yt-on-accent);
    font: 500 15px/1 var(--yt-font);
  }
  .ini.big {
    width: 40px;
    height: 40px;
    font-size: 18px;
    flex: none;
  }
  .signin {
    gap: 6px;
    padding: 0 12px 0 8px;
    height: 36px;
  }
  .ring {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1.5px currentColor;
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    width: 300px;
    z-index: 60;
  }
  .head {
    display: flex;
    gap: 16px;
    padding: 8px 16px;
  }
  .head img {
    width: 40px;
    height: 40px;
  }
  .who {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font: var(--yt-type-meta);
  }
  .who strong {
    font: 500 16px/22px var(--yt-font);
  }
  .lnk {
    align-self: flex-start;
    margin-top: 6px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--yt-link);
    font: 400 14px/20px var(--yt-font);
    cursor: pointer;
  }
  .lnk:hover {
    text-decoration: underline;
  }
</style>
