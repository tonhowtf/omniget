<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "./TgIcon.svelte";
  import { tg, errText, formatDate, listOf, type AccountProfile, type TelegramSelf } from "./api";

  let { onLoggedOut, onSwitched }: { onLoggedOut: () => void; onSwitched: () => void } = $props();

  let me = $state<TelegramSelf | null>(null);
  let profiles = $state<AccountProfile[]>([]);
  let busy = $state<string | null>(null);
  let label = $state("");
  let renaming = $state<string | null>(null);
  let renameText = $state("");
  let confirmLogout = $state(false);

  let displayName = $derived(me ? [me.first_name, me.last_name].filter(Boolean).join(" ") : "");

  onMount(() => {
    void load();
  });

  async function load() {
    try {
      me = await tg.getSelf();
    } catch {
      me = null;
    }
    try {
      profiles = listOf<AccountProfile>(await tg.accountsList(), "profiles");
    } catch {
      profiles = [];
    }
  }

  async function act(key: string, fn: () => Promise<unknown>, ok?: string) {
    busy = key;
    try {
      await fn();
      if (ok) showToast("success", ok);
      await load();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  function saveCurrent() {
    const name = label.trim() || displayName || me?.username || "Telegram";
    void act("save", () => tg.accountsSaveCurrent(name, me?.phone, me?.user_id), $t("telegram.account.saved") as string).then(() => (label = ""));
  }

  async function restore(p: AccountProfile) {
    busy = p.id;
    try {
      await tg.accountsRestore(p.id);
      showToast("success", $t("telegram.account.switched", { name: p.label }) as string);
      onSwitched();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  async function logout() {
    busy = "logout";
    try {
      await tg.logout();
      onLoggedOut();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
      confirmLogout = false;
    }
  }
</script>

<section class="account">
  <div class="me surface-card">
    <span class="avatar-lg">{(displayName || "?").slice(0, 1).toUpperCase()}</span>
    <div class="me-body">
      <strong>{displayName || $t("telegram.account.unknown")}</strong>
      <span>{[me?.username ? `@${me.username}` : "", me?.phone ? `+${me.phone.replace(/^\+/, "")}` : ""].filter(Boolean).join(" · ")}</span>
    </div>
    {#if confirmLogout}
      <span class="confirm">{$t("telegram.account.logout_confirm")}</span>
      <button class="btn btn-ghost btn-sm" onclick={() => (confirmLogout = false)}>{$t("telegram.common.cancel")}</button>
      <button class="btn btn-destructive btn-sm" onclick={logout} disabled={busy === "logout"}>
        {#if busy === "logout"}<span class="spinner"></span>{/if}{$t("telegram.account.logout")}
      </button>
    {:else}
      <button class="btn btn-secondary btn-sm" onclick={() => (confirmLogout = true)}><TgIcon name="logout" /> {$t("telegram.account.logout")}</button>
    {/if}
  </div>

  <div>
    <span class="group-label">{$t("telegram.account.saved_title")}</span>
    <div class="group">
      {#each profiles as p (p.id)}
        <div class="group-row">
          <TgIcon name="user" size={16} />
          <div class="group-row-content">
            {#if renaming === p.id}
              <form onsubmit={(e) => { e.preventDefault(); const id = p.id; renaming = null; void act(id, () => tg.accountsRename(id, renameText.trim())); }}>
                <!-- svelte-ignore a11y_autofocus -->
                <input class="input" bind:value={renameText} autofocus onblur={() => (renaming = null)} />
              </form>
            {:else}
              <span class="group-row-title">{p.label}</span>
            {/if}
            <span class="group-row-sub">{[p.phone_redacted, formatDate(p.updated_at || p.created_at)].filter(Boolean).join(" · ")}</span>
          </div>
          <div class="group-row-trailing">
            <button class="btn btn-ghost btn-sm" onclick={() => restore(p)} disabled={!!busy}>{$t("telegram.account.switch")}</button>
            <button class="btn btn-ghost btn-sm" onclick={() => { renaming = p.id; renameText = p.label; }}>{$t("telegram.account.rename")}</button>
            <button class="btn btn-ghost btn-sm btn-icon" aria-label={$t("telegram.net.remove")} title={$t("telegram.net.remove")} onclick={() => act(p.id, () => tg.accountsRemove(p.id))} disabled={!!busy}>
              <TgIcon name="trash" />
            </button>
          </div>
        </div>
      {:else}
        <div class="group-row"><span class="group-row-sub">{$t("telegram.account.none_saved")}</span></div>
      {/each}
    </div>
    <p class="group-footer">{$t("telegram.account.saved_hint")}</p>
  </div>

  <form class="save" onsubmit={(e) => { e.preventDefault(); saveCurrent(); }}>
    <input class="input" bind:value={label} placeholder={displayName || $t("telegram.account.label")} aria-label={$t("telegram.account.label")} />
    <button class="btn btn-secondary" type="submit" disabled={busy === "save"}>
      {#if busy === "save"}<span class="spinner"></span>{:else}<TgIcon name="save" />{/if}
      {$t("telegram.account.save_current")}
    </button>
  </form>
</section>

<style>
  .account {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    max-width: 640px;
  }

  .me {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4);
  }

  .avatar-lg {
    width: 44px;
    height: 44px;
    border-radius: var(--radius-full);
    display: grid;
    place-items: center;
    flex-shrink: 0;
    font-weight: 700;
    font-size: var(--text-lg);
    color: #fff;
    background: linear-gradient(160deg, #5aa9ff, #1e6fe8);
  }

  .me-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .me-body strong {
    font-size: var(--text-md);
    color: var(--text);
  }

  .me-body span,
  .confirm {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .save {
    display: flex;
    gap: var(--space-2);
  }

  .save .input {
    flex: 1;
  }
</style>
