<script lang="ts">
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";
  import {
    telegramGetSelf,
    telegramAccountsList,
    telegramAccountsSaveCurrent,
    telegramAccountsRestore,
    telegramAccountsRemove,
    telegramAccountsRename,
    telegramAccountsBackupNow,
    telegramAccountsListBackups,
    type TelegramAccountProfile,
    type TelegramSelf,
  } from "$lib/study-telegram-bridge";

  let { open = $bindable(false), sessionPhone } = $props<{ open: boolean; sessionPhone: string }>();

  let loading = $state(false);
  let error = $state("");
  let me = $state<TelegramSelf | null>(null);
  let profiles = $state<TelegramAccountProfile[]>([]);
  let backups = $state<Array<{ name: string; modified_at: number }>>([]);

  let saveOpen = $state(false);
  let saveLabel = $state("");
  let saveBusy = $state(false);

  let renameId = $state<string | null>(null);
  let renameLabel = $state("");
  let renameBusy = $state(false);

  let confirmDeleteId = $state<string | null>(null);
  let confirmRestoreId = $state<string | null>(null);
  let actionBusy = $state(false);

  $effect(() => {
    if (open) load();
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
    } catch (e: any) {
      error = typeof e === "string" ? e : (e?.message ?? "Erro");
    } finally {
      loading = false;
    }
  }

  function close() {
    if (actionBusy || saveBusy || renameBusy) return;
    open = false;
  }

  function openSaveDialog() {
    saveLabel = me?.first_name ? `${me.first_name}${me.last_name ? " " + me.last_name : ""}` : "";
    saveOpen = true;
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
      showToast("info", `Conta '${profile.label}' salva`);
      saveOpen = false;
      saveLabel = "";
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
    } finally {
      saveBusy = false;
    }
  }

  function startRename(p: TelegramAccountProfile) {
    renameId = p.id;
    renameLabel = p.label;
  }

  async function commitRename() {
    if (!renameId || renameBusy) return;
    renameBusy = true;
    try {
      await telegramAccountsRename({ id: renameId, label: renameLabel.trim() });
      renameId = null;
      renameLabel = "";
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
    } finally {
      renameBusy = false;
    }
  }

  async function commitRestore() {
    if (!confirmRestoreId || actionBusy) return;
    actionBusy = true;
    try {
      await telegramAccountsRestore({ id: confirmRestoreId });
      showToast(
        "info",
        "Sessão ativada. Reinicie o app pra entrar nesta conta.",
      );
      confirmRestoreId = null;
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
    } finally {
      actionBusy = false;
    }
  }

  async function commitDelete() {
    if (!confirmDeleteId || actionBusy) return;
    actionBusy = true;
    try {
      await telegramAccountsRemove({ id: confirmDeleteId });
      showToast("info", "Perfil removido");
      confirmDeleteId = null;
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
    } finally {
      actionBusy = false;
    }
  }

  async function backupNow() {
    actionBusy = true;
    try {
      const r = await telegramAccountsBackupNow();
      showToast("info", `Backup criado: ${r.name}`);
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
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
    onkeydown={(e) => { if (e.key === "Escape") close(); }}
  >
    <aside class="panel" role="dialog" aria-modal="true" aria-label={$t("telegram.manage_accounts")}>
      <header class="panel-header">
        <div>
          <h2>Contas Telegram</h2>
          <p class="subtitle">Salve e alterne entre múltiplas sessões.</p>
        </div>
        <button type="button" class="icon-btn" onclick={close} aria-label="Fechar">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 6L6 18" />
            <path d="M6 6l12 12" />
          </svg>
        </button>
      </header>

      <div class="body">
        {#if loading}
          <div class="status"><span class="spinner"></span></div>
        {:else if error}
          <div class="status status-error">{error}</div>
        {:else}
          <section class="active-card">
            <span class="section-label">Conta ativa</span>
            <div class="active-row">
              <div class="active-avatar">
                {me ? initials(`${me.first_name} ${me.last_name ?? ""}`.trim() || sessionPhone) : "?"}
              </div>
              <div class="active-info">
                <span class="active-name">
                  {#if me}
                    {me.first_name}{me.last_name ? " " + me.last_name : ""}
                  {:else}
                    {sessionPhone || "Sessão local"}
                  {/if}
                </span>
                <span class="active-meta">
                  {#if sessionPhone}{sessionPhone}{/if}
                  {#if me?.username}<span class="dot">·</span>@{me.username}{/if}
                </span>
              </div>
              <span class="active-badge">Ativa</span>
            </div>
            <button
              type="button"
              class="primary-btn"
              onclick={openSaveDialog}
              disabled={!sessionPhone && !me}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M19 21H5a2 2 0 01-2-2V5a2 2 0 012-2h11l5 5v11a2 2 0 01-2 2z" />
                <path d="M17 21v-8H7v8M7 3v5h8" />
              </svg>
              Salvar como perfil
            </button>
          </section>

          <section>
            <div class="section-row">
              <span class="section-label">Perfis salvos</span>
              <span class="section-count">{profiles.length}</span>
            </div>
            {#if profiles.length === 0}
              <div class="empty-state">
                <p class="empty-title">Nenhum perfil salvo ainda.</p>
                <p class="empty-desc">
                  Salve sua sessão atual antes de fazer logout — assim você consegue voltar pra ela depois sem refazer login.
                </p>
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
                        <input
                          type="text"
                          class="input"
                          bind:value={renameLabel}
                          disabled={renameBusy}
                          autofocus
                        />
                        <button type="submit" class="ghost-btn" disabled={renameBusy || !renameLabel.trim()}>OK</button>
                        <button type="button" class="ghost-btn" onclick={() => (renameId = null)} disabled={renameBusy}>Cancelar</button>
                      </form>
                    {:else}
                      <div class="profile-row">
                        <div class="profile-avatar">{initials(p.label)}</div>
                        <div class="profile-info">
                          <span class="profile-label">{p.label}</span>
                          <span class="profile-meta">
                            {#if p.phone_redacted}{p.phone_redacted}{:else}—{/if}
                            <span class="dot">·</span>
                            criado {fmtDate(p.created_at)}
                          </span>
                        </div>
                      </div>
                      <div class="profile-actions">
                        <button type="button" class="ghost-btn" onclick={() => startRename(p)}>Renomear</button>
                        <button type="button" class="primary-btn small" onclick={() => (confirmRestoreId = p.id)}>
                          Ativar
                        </button>
                        <button type="button" class="danger-btn small" onclick={() => (confirmDeleteId = p.id)} aria-label="Remover">
                          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
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
              <span class="section-label">Backups</span>
              <button type="button" class="ghost-btn" onclick={backupNow} disabled={actionBusy || (!sessionPhone && !me)}>
                Criar backup agora
              </button>
            </div>
            {#if backups.length === 0}
              <p class="empty-text">Nenhum backup. Faça um antes de mudanças importantes na sessão.</p>
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
    </aside>
  </div>

  {#if saveOpen}
    <div
      class="dialog-overlay"
      role="presentation"
      onclick={(e) => { if (e.target === e.currentTarget && !saveBusy) saveOpen = false; }}
      onkeydown={() => {}}
    >
      <div class="dialog" role="dialog" aria-modal="true">
        <h3>Salvar conta atual como perfil</h3>
        <p>Crie um nome pra reconhecer essa conta depois (ex: "Pessoal", "Trabalho").</p>
        <form onsubmit={(e) => { e.preventDefault(); commitSave(); }}>
          <input
            type="text"
            class="input"
            placeholder="Nome do perfil"
            bind:value={saveLabel}
            disabled={saveBusy}
            autofocus
            required
          />
          <div class="dialog-actions">
            <button type="button" class="ghost-btn" onclick={() => (saveOpen = false)} disabled={saveBusy}>Cancelar</button>
            <button type="submit" class="primary-btn" disabled={saveBusy || !saveLabel.trim()}>
              {saveBusy ? "Salvando..." : "Salvar perfil"}
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
      <div class="dialog" role="dialog" aria-modal="true">
        <h3>Ativar perfil "{p?.label}"?</h3>
        <p>
          Sua sessão atual será preservada como backup automático. O app precisa ser reiniciado para concluir a troca.
        </p>
        <div class="dialog-actions">
          <button type="button" class="ghost-btn" onclick={() => (confirmRestoreId = null)} disabled={actionBusy}>Cancelar</button>
          <button type="button" class="primary-btn" onclick={commitRestore} disabled={actionBusy}>
            {actionBusy ? "Ativando..." : "Ativar e reiniciar"}
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
      <div class="dialog" role="dialog" aria-modal="true">
        <h3>Remover perfil "{p?.label}"?</h3>
        <p class="warn">
          A sessão deste perfil será apagada permanentemente. Você precisará refazer login pra acessar essa conta novamente.
        </p>
        <div class="dialog-actions">
          <button type="button" class="ghost-btn" onclick={() => (confirmDeleteId = null)} disabled={actionBusy}>Cancelar</button>
          <button type="button" class="danger-btn" onclick={commitDelete} disabled={actionBusy}>
            {actionBusy ? "Removendo..." : "Remover perfil"}
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
    background: rgba(0, 0, 0, 0.55);
    z-index: 130;
    display: flex;
    justify-content: flex-end;
    animation: fade-in 150ms ease;
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .panel {
    width: min(480px, 100vw);
    height: 100%;
    background: var(--surface, var(--button));
    display: flex;
    flex-direction: column;
    box-shadow: -2px 0 12px rgba(0, 0, 0, 0.2);
    animation: slide-in 200ms cubic-bezier(0.22, 1, 0.36, 1);
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
  }

  .icon-btn:hover {
    background: var(--button-elevated);
    color: var(--secondary);
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
    border: 1px solid color-mix(in oklab, var(--green, #10b981) 35%, transparent);
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
    background: linear-gradient(135deg, var(--blue), color-mix(in oklab, var(--blue) 60%, var(--green, #10b981)));
    color: #fff;
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
    background: var(--green, #10b981);
    color: #fff;
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
    outline: none;
    border-color: var(--blue);
  }

  .primary-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px 16px;
    background: var(--blue);
    color: #fff;
    border: none;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background 150ms;
  }

  .primary-btn.small {
    padding: 5px 10px;
    font-size: 12px;
  }

  .primary-btn:hover:not(:disabled) {
    background: color-mix(in oklab, var(--blue) 90%, #000);
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
  }

  .ghost-btn:hover:not(:disabled) {
    color: var(--secondary);
    background: var(--button-elevated);
  }

  .ghost-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .danger-btn {
    padding: 5px 8px;
    background: transparent;
    border: 1px solid color-mix(in oklab, var(--red) 30%, transparent);
    color: var(--red);
    border-radius: var(--border-radius);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .danger-btn.small {
    padding: 5px 8px;
  }

  .danger-btn:hover:not(:disabled) {
    background: color-mix(in oklab, var(--red) 12%, transparent);
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
    color: var(--red);
  }

  .spinner {
    width: 22px;
    height: 22px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .dialog-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    z-index: 200;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--padding);
  }

  .dialog {
    width: min(420px, 100%);
    background: var(--surface, var(--button));
    padding: calc(var(--padding) * 1.5);
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.3);
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
    color: var(--red);
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
</style>
