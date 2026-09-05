<script lang="ts">
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";
  import { relaunch } from "@tauri-apps/plugin-process";
  import {
    telegramGetSelf,
    telegramAccountsList,
    telegramAccountsPrepareAdd,
    telegramAccountsSaveCurrent,
    telegramAccountsRestore,
    telegramAccountsRemove,
    telegramAccountsRename,
    telegramAccountsBackupNow,
    telegramAccountsListBackups,
    type TelegramAccountProfile,
    type TelegramSelf,
  } from "$lib/study-telegram-bridge";

  let {
    open = $bindable(false),
    sessionPhone,
    onAddAccount = () => {},
  } = $props<{ open: boolean; sessionPhone: string; onAddAccount?: () => void }>();

  let loading = $state(false);
  let error = $state("");
  let me = $state<TelegramSelf | null>(null);
  let profiles = $state<TelegramAccountProfile[]>([]);
  let backups = $state<Array<{ name: string; modified_at: number }>>([]);

  let saveOpen = $state(false);
  let saveLabel = $state("");
  let saveBusy = $state(false);
  let addOpen = $state(false);
  let addLabel = $state("");
  let addBusy = $state(false);

  let renameId = $state<string | null>(null);
  let renameLabel = $state("");
  let renameBusy = $state(false);

  let confirmDeleteId = $state<string | null>(null);
  let confirmRestoreId = $state<string | null>(null);
  let actionBusy = $state(false);
  let panel = $state<HTMLElement>();
  let closeButton = $state<HTMLButtonElement>();
  let saveInput = $state<HTMLInputElement>();
  let addInput = $state<HTMLInputElement>();
  let dialogCancelButton = $state<HTMLButtonElement>();
  let previouslyFocused: HTMLElement | null = null;

  $effect(() => {
    if (open) {
      previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      load();
      requestAnimationFrame(() => closeButton?.focus());
    }
  });

  $effect(() => {
    if (confirmRestoreId || confirmDeleteId) {
      requestAnimationFrame(() => dialogCancelButton?.focus());
    }
  });

  async function load() {
    loading = true;
    error = "";
    try {
      const [selfInfo, list, bks] = await Promise.all([
        telegramGetSelf().catch(() => null),
        telegramAccountsList(),
        telegramAccountsListBackups(),
      ]);
      me = selfInfo;
      profiles = list;
      backups = bks;
    } catch (e) {
      error = errorMessage(e);
    } finally {
      loading = false;
    }
  }

  function close() {
    if (actionBusy || saveBusy || renameBusy) return;
    open = false;
    requestAnimationFrame(() => previouslyFocused?.focus());
  }

  function handlePanelKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      if (saveOpen && !saveBusy) saveOpen = false;
      else if (addOpen && !addBusy) addOpen = false;
      else if (confirmRestoreId && !actionBusy) confirmRestoreId = null;
      else if (confirmDeleteId && !actionBusy) confirmDeleteId = null;
      else close();
      return;
    }
    const focusScope = document.querySelector<HTMLElement>(".dialog-overlay .dialog") ?? panel;
    if (event.key !== "Tab" || !focusScope) return;
    const focusable = Array.from(
      focusScope.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])',
      ),
    ).filter((element) => element.offsetParent !== null);
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  function openSaveDialog() {
    saveLabel = me?.first_name ? `${me.first_name}${me.last_name ? " " + me.last_name : ""}` : "";
    saveOpen = true;
    requestAnimationFrame(() => saveInput?.focus());
  }

  function openAddDialog() {
    addLabel = me?.first_name ? `${me.first_name}${me.last_name ? " " + me.last_name : ""}` : "";
    addOpen = true;
    requestAnimationFrame(() => addInput?.focus());
  }

  function errorMessage(e: unknown): string {
    return typeof e === "string" ? e : ((e as { message?: string })?.message ?? $t("common.error"));
  }

  async function commitSave() {
    if (saveBusy) return;
    saveBusy = true;
    try {
      const profile = await telegramAccountsSaveCurrent({
        label: saveLabel.trim(),
        phone: sessionPhone || me?.phone || undefined,
        userId: me?.user_id,
      });
      showToast("info", $t("telegram.accounts_toast_saved", { label: profile.label }));
      saveOpen = false;
      saveLabel = "";
      await load();
    } catch (e) {
      showToast("error", errorMessage(e));
    } finally {
      saveBusy = false;
    }
  }

  async function commitAddAccount() {
    if (addBusy) return;
    addBusy = true;
    try {
      const profile = await telegramAccountsPrepareAdd({
        label: addLabel.trim(),
        phone: sessionPhone || me?.phone || undefined,
        userId: me?.user_id,
      });
      showToast("info", $t("telegram.accounts_toast_saved", { label: profile.label }));
      addOpen = false;
      open = false;
      onAddAccount();
    } catch (e) {
      showToast("error", errorMessage(e));
    } finally {
      addBusy = false;
    }
  }

  function startRename(p: TelegramAccountProfile) {
    renameId = p.id;
    renameLabel = p.label;
    requestAnimationFrame(() => document.getElementById(`rename-account-${p.id}`)?.focus());
  }

  async function commitRename() {
    if (!renameId || renameBusy) return;
    renameBusy = true;
    try {
      await telegramAccountsRename({ id: renameId, label: renameLabel.trim() });
      renameId = null;
      renameLabel = "";
      await load();
    } catch (e) {
      showToast("error", errorMessage(e));
    } finally {
      renameBusy = false;
    }
  }

  async function commitRestore() {
    if (!confirmRestoreId || actionBusy) return;
    actionBusy = true;
    try {
      const result = await telegramAccountsRestore({ id: confirmRestoreId });
      if (result.needs_restart) {
        await relaunch();
        return;
      }
      confirmRestoreId = null;
      await load();
    } catch (e) {
      showToast("error", errorMessage(e));
    } finally {
      actionBusy = false;
    }
  }

  async function commitDelete() {
    if (!confirmDeleteId || actionBusy) return;
    actionBusy = true;
    try {
      await telegramAccountsRemove({ id: confirmDeleteId });
      showToast("info", $t("telegram.accounts_toast_removed"));
      confirmDeleteId = null;
      await load();
    } catch (e) {
      showToast("error", errorMessage(e));
    } finally {
      actionBusy = false;
    }
  }

  async function backupNow() {
    actionBusy = true;
    try {
      const r = await telegramAccountsBackupNow();
      showToast("info", $t("telegram.accounts_toast_backup_created", { name: r.name }));
      await load();
    } catch (e) {
      showToast("error", errorMessage(e));
    } finally {
      actionBusy = false;
    }
  }

  function fmtDate(ts: number): string {
    if (!ts) return "—";
    return new Date(ts * 1000).toLocaleDateString();
  }

  function profileById(id: string | null): TelegramAccountProfile | undefined {
    if (!id) return undefined;
    return profiles.find((p) => p.id === id);
  }

  function initials(label: string): string {
    const parts = label.trim().split(/\s+/).filter(Boolean);
    if (parts.length === 0) return "?";
    if (parts.length === 1) return parts[0].charAt(0).toUpperCase();
    return (parts[0].charAt(0) + parts[parts.length - 1].charAt(0)).toUpperCase();
  }
</script>

{#if open}
  <div
    class="overlay"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) close(); }}
    onkeydown={handlePanelKeydown}
  >
    <div
      class="panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="telegram-accounts-title"
      tabindex="-1"
      bind:this={panel}
    >
      <header class="panel-header">
        <div>
          <h2 id="telegram-accounts-title">{$t("telegram.accounts_title")}</h2>
          <p class="subtitle">{$t("telegram.accounts_subtitle")}</p>
        </div>
        <button type="button" class="icon-btn" onclick={close} aria-label={$t("common.close")} bind:this={closeButton}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
            <path d="M18 6L6 18" />
            <path d="M6 6l12 12" />
          </svg>
        </button>
      </header>

      <div class="body">
        {#if loading}
          <div class="status" role="status"><span class="spinner" aria-hidden="true"></span>{$t("common.loading")}</div>
        {:else if error}
          <div class="status status-error" role="alert">
            <span>{error}</span>
            <button type="button" class="ghost-btn" onclick={load}>{$t("common.retry")}</button>
          </div>
        {:else}
          <section class="active-card">
            <span class="section-label">{$t("telegram.accounts_active_label")}</span>
            <div class="active-row">
              <div class="active-avatar">
                {me ? initials(`${me.first_name} ${me.last_name ?? ""}`.trim() || sessionPhone) : "?"}
              </div>
              <div class="active-info">
                <span class="active-name">
                  {#if me}
                    {me.first_name}{me.last_name ? " " + me.last_name : ""}
                  {:else}
                    {sessionPhone || $t("telegram.accounts_local_session_fallback")}
                  {/if}
                </span>
                <span class="active-meta">
                  {#if sessionPhone}{sessionPhone}{/if}
                  {#if me?.username}<span class="dot">·</span>@{me.username}{/if}
                </span>
              </div>
              <span class="active-badge">{$t("telegram.accounts_active_badge")}</span>
            </div>
            <button
              type="button"
              class="primary-btn"
              onclick={openSaveDialog}
              disabled={!sessionPhone && !me}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
                <path d="M19 21H5a2 2 0 01-2-2V5a2 2 0 012-2h11l5 5v11a2 2 0 01-2 2z" />
                <path d="M17 21v-8H7v8M7 3v5h8" />
              </svg>
              {$t("telegram.accounts_save_as_profile")}
            </button>
            <button
              type="button"
              class="ghost-btn add-account-btn"
              onclick={openAddDialog}
              disabled={!sessionPhone && !me}
            >
              {$t("telegram.accounts_add")}
            </button>
          </section>

          <section>
            <div class="section-row">
              <span class="section-label">{$t("telegram.accounts_saved_profiles")}</span>
              <span class="section-count">{profiles.length}</span>
            </div>
            {#if profiles.length === 0}
              <div class="empty-state">
                <p class="empty-title">{$t("telegram.accounts_empty_title")}</p>
                <p class="empty-desc">{$t("telegram.accounts_empty_desc")}</p>
              </div>
            {:else}
              <ul class="profile-list">
                {#each profiles as p (p.id)}
                  <li class="profile-card">
                    {#if renameId === p.id}
                      <form
                        class="rename-row"
                        onsubmit={(e) => { e.preventDefault(); commitRename(); }}
                      >
                        <label class="sr-only" for={`rename-account-${p.id}`}>{$t("telegram.accounts_profile_name_label")}</label>
                        <input
                          id={`rename-account-${p.id}`}
                          type="text"
                          class="input"
                          bind:value={renameLabel}
                          disabled={renameBusy}
                          required
                          maxlength="64"
                        />
                        <button type="submit" class="ghost-btn" disabled={renameBusy}>{$t("telegram.accounts_rename_save")}</button>
                        <button type="button" class="ghost-btn" onclick={() => (renameId = null)} disabled={renameBusy}>{$t("common.cancel")}</button>
                      </form>
                    {:else}
                      <div class="profile-row">
                        <div class="profile-avatar">{initials(p.label)}</div>
                        <div class="profile-info">
                          <span class="profile-label">{p.label}</span>
                          <span class="profile-meta">
                            {#if p.phone_redacted}{p.phone_redacted}{:else}—{/if}
                            <span class="dot">·</span>
                            {$t("telegram.accounts_created_on", { date: fmtDate(p.created_at) })}
                          </span>
                        </div>
                      </div>
                      <div class="profile-actions">
                        <button type="button" class="ghost-btn" onclick={() => startRename(p)}>{$t("telegram.accounts_rename")}</button>
                        <button type="button" class="primary-btn small" onclick={() => (confirmRestoreId = p.id)}>
                          {$t("telegram.accounts_activate")}
                        </button>
                        <button type="button" class="danger-btn small" onclick={() => (confirmDeleteId = p.id)} aria-label={$t("telegram.accounts_remove")}>
                          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
                            <polyline points="3 6 5 6 21 6" />
                            <path d="M19 6l-2 14a2 2 0 01-2 2H9a2 2 0 01-2-2L5 6" />
                          </svg>
                        </button>
                      </div>
                    {/if}
                  </li>
                {/each}
              </ul>
            {/if}
          </section>

          <section>
            <div class="section-row">
              <span class="section-label">{$t("telegram.accounts_backups_label")}</span>
              <button type="button" class="ghost-btn" onclick={backupNow} disabled={actionBusy || (!sessionPhone && !me)}>
                {$t("telegram.accounts_backup_now")}
              </button>
            </div>
            {#if backups.length === 0}
              <p class="empty-text">{$t("telegram.accounts_no_backups")}</p>
            {:else}
              <ul class="backup-list">
                {#each backups as b (b.name)}
                  <li class="backup-item">
                    <span class="backup-name">{b.name}</span>
                    <span class="backup-date">{fmtDate(b.modified_at)}</span>
                  </li>
                {/each}
              </ul>
            {/if}
          </section>
        {/if}
      </div>
    </div>
  </div>

  {#if saveOpen}
    <div
      class="dialog-overlay"
      role="presentation"
      onclick={(e) => { if (e.target === e.currentTarget && !saveBusy) saveOpen = false; }}
      onkeydown={() => {}}
    >
      <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="save-account-title" aria-describedby="save-account-desc" tabindex="-1">
        <h3 id="save-account-title">{$t("telegram.accounts_save_dialog_title")}</h3>
        <p id="save-account-desc">{$t("telegram.accounts_save_dialog_desc")}</p>
        <form onsubmit={(e) => { e.preventDefault(); commitSave(); }}>
          <label for="save-account-label">{$t("telegram.accounts_profile_name_label")}</label>
          <input
            id="save-account-label"
            type="text"
            class="input"
            placeholder={$t("telegram.accounts_profile_name_example")}
            bind:value={saveLabel}
            bind:this={saveInput}
            disabled={saveBusy}
            required
            maxlength="64"
          />
          <div class="dialog-actions">
            <button type="button" class="ghost-btn" onclick={() => (saveOpen = false)} disabled={saveBusy}>{$t("common.cancel")}</button>
            <button type="submit" class="primary-btn" disabled={saveBusy}>
              {saveBusy ? $t("telegram.accounts_saving") : $t("telegram.accounts_save_profile_btn")}
            </button>
          </div>
        </form>
      </div>
    </div>
  {/if}

  {#if addOpen}
    <div
      class="dialog-overlay"
      role="presentation"
      onclick={(e) => { if (e.target === e.currentTarget && !addBusy) addOpen = false; }}
      onkeydown={() => {}}
    >
      <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="add-account-title" aria-describedby="add-account-desc" tabindex="-1">
        <h3 id="add-account-title">{$t("telegram.accounts_add_dialog_title")}</h3>
        <p id="add-account-desc">{$t("telegram.accounts_add_dialog_desc")}</p>
        <form onsubmit={(e) => { e.preventDefault(); commitAddAccount(); }}>
          <label for="add-account-label">{$t("telegram.accounts_profile_name_label")}</label>
          <input
            id="add-account-label"
            type="text"
            class="input"
            placeholder={$t("telegram.accounts_profile_name_example")}
            bind:value={addLabel}
            bind:this={addInput}
            disabled={addBusy}
            required
            maxlength="64"
          />
          <div class="dialog-actions">
            <button type="button" class="ghost-btn" onclick={() => (addOpen = false)} disabled={addBusy}>{$t("common.cancel")}</button>
            <button type="submit" class="primary-btn" disabled={addBusy}>
              {addBusy ? $t("telegram.accounts_adding") : $t("telegram.accounts_add_btn")}
            </button>
          </div>
        </form>
      </div>
    </div>
  {/if}

  {#if confirmRestoreId}
    {@const p = profileById(confirmRestoreId)}
    <div
      class="dialog-overlay"
      role="presentation"
      onclick={(e) => { if (e.target === e.currentTarget && !actionBusy) confirmRestoreId = null; }}
      onkeydown={() => {}}
    >
      <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="activate-account-title" aria-describedby="activate-account-desc" tabindex="-1">
        <h3 id="activate-account-title">{$t("telegram.accounts_activate_confirm_title", { label: p?.label ?? "" })}</h3>
        <p id="activate-account-desc">{$t("telegram.accounts_activate_confirm_desc")}</p>
        <div class="dialog-actions">
          <button type="button" class="ghost-btn" onclick={() => (confirmRestoreId = null)} disabled={actionBusy} bind:this={dialogCancelButton}>{$t("common.cancel")}</button>
          <button type="button" class="primary-btn" onclick={commitRestore} disabled={actionBusy}>
            {actionBusy ? $t("telegram.accounts_activating") : $t("telegram.accounts_activate_and_restart")}
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if confirmDeleteId}
    {@const p = profileById(confirmDeleteId)}
    <div
      class="dialog-overlay"
      role="presentation"
      onclick={(e) => { if (e.target === e.currentTarget && !actionBusy) confirmDeleteId = null; }}
      onkeydown={() => {}}
    >
      <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="remove-account-title" aria-describedby="remove-account-desc" tabindex="-1">
        <h3 id="remove-account-title">{$t("telegram.accounts_remove_confirm_title", { label: p?.label ?? "" })}</h3>
        <p id="remove-account-desc" class="warn">{$t("telegram.accounts_remove_confirm_desc")}</p>
        <div class="dialog-actions">
          <button type="button" class="ghost-btn" onclick={() => (confirmDeleteId = null)} disabled={actionBusy} bind:this={dialogCancelButton}>{$t("common.cancel")}</button>
          <button type="button" class="danger-btn" onclick={commitDelete} disabled={actionBusy}>
            {actionBusy ? $t("telegram.accounts_removing") : $t("telegram.accounts_remove_profile_btn")}
          </button>
        </div>
      </div>
    </div>
  {/if}
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: var(--dialog-backdrop);
    z-index: 130;
    display: flex;
    justify-content: flex-end;
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .panel {
    width: min(480px, 100vw);
    height: 100%;
    background: var(--popup-bg);
    display: flex;
    flex-direction: column;
    box-shadow: -2px 0 12px color-mix(in oklab, var(--primary) 30%, transparent);
    overscroll-behavior: contain;
  }

  @keyframes slide-in {
    from { transform: translateX(20px); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }

  .panel-header {
    display: flex;
    align-items: flex-start;
    gap: var(--padding);
    padding: var(--padding);
    border-bottom: 1px solid var(--input-border);
  }

  .panel-header > div {
    flex: 1;
  }

  .panel-header h2 {
    margin: 0;
    font-size: 16px;
    color: var(--secondary);
  }

  .subtitle {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--gray);
  }

  .icon-btn {
    background: transparent;
    border: none;
    color: var(--gray);
    cursor: pointer;
    padding: 6px;
    border-radius: var(--border-radius);
    min-width: 40px;
    min-height: 40px;
  }

  @media (hover: hover) {
    .icon-btn:hover {
      background: var(--button-elevated);
      color: var(--secondary);
    }
  }

  .body {
    flex: 1;
    overflow-y: auto;
    padding: var(--padding);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
  }

  .section-label {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }

  .section-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
  }

  .section-count {
    font-size: 11.5px;
    color: var(--gray);
    background: var(--button-elevated);
    padding: 2px 10px;
    border-radius: 100px;
  }

  .active-card {
    background: var(--button);
    border-radius: var(--border-radius);
    padding: var(--padding);
    display: flex;
    flex-direction: column;
    gap: 12px;
    border: 1px solid color-mix(in oklab, var(--success) 35%, transparent);
  }

  .active-row {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .active-avatar {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--accent), color-mix(in oklab, var(--accent) 60%, var(--success)));
    color: var(--on-accent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 16px;
    font-weight: 600;
    flex-shrink: 0;
  }

  .active-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .active-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .active-meta {
    font-size: 11.5px;
    color: var(--gray);
  }

  .active-badge {
    font-size: 10.5px;
    font-weight: 700;
    background: var(--success);
    color: var(--on-success);
    padding: 3px 10px;
    border-radius: 12px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .empty-state {
    text-align: center;
    padding: var(--padding) calc(var(--padding) * 1.5);
    background: var(--button);
    border-radius: var(--border-radius);
    border: 1px dashed var(--input-border);
  }

  .empty-title {
    margin: 0 0 6px 0;
    font-size: 13px;
    color: var(--secondary);
    font-weight: 500;
  }

  .empty-desc {
    margin: 0;
    font-size: 12px;
    color: var(--gray);
    line-height: 1.5;
  }

  .empty-text {
    margin: 0;
    font-size: 12px;
    color: var(--gray);
    text-align: center;
    padding: 12px 0;
  }

  .profile-list,
  .backup-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .profile-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px;
    background: var(--button);
    border-radius: var(--border-radius);
    flex-wrap: wrap;
  }

  .profile-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: 1;
    min-width: 0;
  }

  .profile-avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: var(--button-elevated);
    color: var(--gray);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    font-weight: 600;
    flex-shrink: 0;
  }

  .profile-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .profile-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
  }

  .profile-meta {
    font-size: 11px;
    color: var(--gray);
  }

  .dot {
    margin: 0 4px;
  }

  .profile-actions {
    display: flex;
    gap: 4px;
    align-items: center;
    flex-shrink: 0;
  }

  .rename-row {
    display: flex;
    width: 100%;
    gap: 6px;
    align-items: center;
  }

  .rename-row .input {
    flex: 1;
  }

  .input {
    padding: 7px 10px;
    background: var(--button-elevated);
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
    width: 100%;
  }

  .input:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .primary-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px 16px;
    background: var(--accent);
    color: var(--on-accent);
    border: none;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background 150ms;
    min-height: 40px;
  }

  .primary-btn.small {
    padding: 5px 10px;
    font-size: 12px;
  }

  @media (hover: hover) {
    .primary-btn:hover:not(:disabled) {
      background: var(--button-hover);
    }
  }

  .primary-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .ghost-btn {
    padding: 6px 10px;
    background: transparent;
    border: 1px solid var(--input-border);
    color: var(--gray);
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
    min-height: 40px;
  }

  .active-card > .primary-btn,
  .active-card > .add-account-btn {
    width: 100%;
  }

  @media (hover: hover) {
    .ghost-btn:hover:not(:disabled) {
      color: var(--secondary);
      background: var(--button-elevated);
    }
  }

  .ghost-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .danger-btn {
    padding: 5px 8px;
    background: transparent;
    border: 1px solid color-mix(in oklab, var(--error) 30%, transparent);
    color: var(--error);
    border-radius: var(--border-radius);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 40px;
    min-height: 40px;
  }

  .danger-btn.small {
    padding: 5px 8px;
  }

  @media (hover: hover) {
    .danger-btn:hover:not(:disabled) {
      background: color-mix(in oklab, var(--error) 12%, transparent);
    }
  }

  .danger-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .backup-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    background: var(--button);
    border-radius: var(--border-radius);
    font-size: 12px;
  }

  .backup-name {
    color: var(--secondary);
    font-family: monospace;
  }

  .backup-date {
    color: var(--gray);
  }

  .status {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: calc(var(--padding) * 3);
    gap: 12px;
    color: var(--gray);
  }

  .status-error {
    color: var(--error);
  }

  .spinner {
    width: 22px;
    height: 22px;
    border: 2px solid var(--input-border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .dialog-overlay {
    position: fixed;
    inset: 0;
    background: var(--dialog-backdrop);
    z-index: 200;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--padding);
  }

  .dialog {
    width: min(420px, 100%);
    background: var(--popup-bg);
    padding: calc(var(--padding) * 1.5);
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    box-shadow: 0 12px 40px color-mix(in oklab, var(--primary) 35%, transparent);
  }

  .dialog h3 {
    margin: 0;
    font-size: 16px;
    color: var(--secondary);
  }

  .dialog p {
    margin: 0;
    font-size: 13px;
    color: var(--gray);
    line-height: 1.5;
  }

  .dialog .warn {
    color: var(--error);
    font-weight: 500;
  }

  .dialog form {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }

  .icon-btn:focus-visible,
  .primary-btn:focus-visible,
  .ghost-btn:focus-visible,
  .danger-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  @media (prefers-reduced-motion: no-preference) {
    .overlay {
      animation: fade-in 150ms ease-out;
    }

    .panel {
      animation: slide-in 200ms cubic-bezier(0.2, 0, 0, 1);
    }
  }

  @media (max-width: 535px) {
    .panel {
      padding-bottom: env(safe-area-inset-bottom);
    }

    .input {
      font-size: 16px;
    }

    .profile-actions,
    .rename-row,
    .dialog-actions {
      flex-wrap: wrap;
    }
  }
</style>
