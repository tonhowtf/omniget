<script lang="ts">
  import { t } from "$lib/i18n";
  import TgIcon from "$lib/telegram/TgIcon.svelte";
  import DcAvatar from "./DcAvatar.svelte";
  import type { DcAccount } from "./api";
  import { avatarUrl, displayName } from "./format";

  let {
    accounts,
    active,
    onSwitch,
    onAdd,
    onLogout,
  }: {
    accounts: DcAccount[];
    active: string | null;
    onSwitch: (id: string) => void;
    onAdd: () => void;
    onLogout: (id: string) => void;
  } = $props();

  let open = $state(false);
  let confirmOut = $state(false);
  let root: HTMLDivElement | undefined = $state();
  let current = $derived(accounts.find((a) => a.id === active) ?? null);

  $effect(() => {
    if (!open) {
      confirmOut = false;
      return;
    }
    const away = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) open = false;
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") open = false;
    };
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("keydown", esc);
    };
  });

  function pick(fn: () => void) {
    open = false;
    fn();
  }
</script>

<div class="acct" bind:this={root}>
  <button class="btn btn-ghost btn-sm who" aria-haspopup="menu" aria-expanded={open} onclick={() => (open = !open)}>
    {#if current}
      <DcAvatar src={avatarUrl({ ...current, bot: false }, 48)} name={displayName({ ...current, bot: false })} size={20} />
      <span class="nm">{displayName({ ...current, bot: false })}</span>
    {:else}
      <TgIcon name="user" />
    {/if}
    <span class="chev"><TgIcon name="chevron" size={10} /></span>
  </button>
  {#if open}
    <div class="popover menu" role="menu">
      <div class="m-label">{$t("guildchat.account.accounts")}</div>
      {#each accounts as a (a.id)}
        <button class="menu-item" role="menuitemradio" aria-checked={a.id === active} onclick={() => pick(() => a.id !== active && onSwitch(a.id))}>
          <DcAvatar src={avatarUrl({ ...a, bot: false }, 48)} name={displayName({ ...a, bot: false })} size={20} />
          <span class="m-name">{displayName({ ...a, bot: false })}<small>@{a.username}</small></span>
          {#if a.id === active}<TgIcon name="check" />{/if}
        </button>
      {/each}
      <div class="menu-separator"></div>
      <button class="menu-item" role="menuitem" onclick={() => pick(onAdd)}><TgIcon name="plus" /> {$t("guildchat.account.add")}</button>
      {#if active}
        {#if confirmOut}
          <div class="confirm">
            <span>{$t("guildchat.account.logout_confirm")}</span>
            <button class="btn btn-destructive btn-sm" onclick={() => pick(() => active && onLogout(active))}>{$t("guildchat.account.logout")}</button>
          </div>
        {:else}
          <button class="menu-item" role="menuitem" onclick={() => (confirmOut = true)}><TgIcon name="logout" /> {$t("guildchat.account.logout")}</button>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .acct {
    position: relative;
  }

  .who {
    gap: 6px;
    max-width: 220px;
  }

  .nm {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .chev {
    display: inline-flex;
    transform: rotate(90deg);
    color: var(--text-dim);
  }

  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    width: 260px;
  }

  .m-label {
    padding: 4px 10px 2px;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-dim);
  }

  .m-name {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    line-height: 1.25;
    overflow: hidden;
  }

  .m-name small {
    font-size: var(--text-xs);
    opacity: 0.7;
  }

  .confirm {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) 10px;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
</style>
