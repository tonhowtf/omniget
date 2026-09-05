<script lang="ts">
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    telegramFullChannelInfo,
    telegramSetMute,
    telegramTogglePin,
    telegramSetArchived,
    telegramLeaveChannel,
    telegramDeleteHistory,
    telegramListParticipants,
    telegramSetBlocked,
    telegramReportPeer,
    telegramDeleteChannel,
    type TelegramChatType,
    type TelegramFullChannelInfo,
    type TelegramParticipant,
    type TelegramParticipantFilter,
  } from "$lib/study-telegram-bridge";

  type ChatLite = { id: number; title: string; chat_type: string };

  let {
    open = $bindable(false),
    chat,
    onChatRemoved,
  } = $props<{
    open: boolean;
    chat: ChatLite | null;
    onChatRemoved?: (chatId: number) => void;
  }>();

  let tab = $state<"info" | "members">("info");
  let info = $state<TelegramFullChannelInfo | null>(null);
  let infoLoading = $state(false);
  let infoError = $state("");

  let participants = $state<TelegramParticipant[]>([]);
  let participantsCount = $state(0);
  let participantsLoading = $state(false);
  let participantsError = $state("");
  let participantsFilter = $state<TelegramParticipantFilter>("recent");
  let participantsSearch = $state("");
  let searchDebounce: ReturnType<typeof setTimeout> | null = null;

  let muted = $state(false);
  let pinned = $state(false);
  let archived = $state(false);
  let blocked = $state(false);

  let confirm = $state<{
    kind: "leave" | "delete-channel" | "clear-history" | "report" | null;
    busy: boolean;
  }>({ kind: null, busy: false });

  let clearMode = $state<"clear-me" | "delete-all" | "leave">("clear-me");
  let reportMessage = $state("");

  const chatType = $derived<TelegramChatType>((chat?.chat_type ?? "channel") as TelegramChatType);
  const isChannel = $derived(chat?.chat_type === "channel");
  const isGroup = $derived(chat?.chat_type === "group");
  const isPrivate = $derived(chat?.chat_type === "private");

  $effect(() => {
    if (open && chat) {
      tab = "info";
      loadInfo();
      participants = [];
      participantsCount = 0;
    }
  });

  async function loadInfo() {
    if (!chat) return;
    if (isPrivate) {
      info = {
        chat_id: chat.id,
        title: chat.title,
        about: "",
      };
      return;
    }
    infoLoading = true;
    infoError = "";
    try {
      info = await telegramFullChannelInfo({
        chatId: chat.id,
        chatType,
      });
    } catch (e: any) {
      infoError = typeof e === "string" ? e : (e?.message ?? "Erro ao carregar info");
    } finally {
      infoLoading = false;
    }
  }

  async function loadParticipants() {
    if (!chat || isPrivate) return;
    participantsLoading = true;
    participantsError = "";
    try {
      const page = await telegramListParticipants({
        chatId: chat.id,
        filter: participantsFilter,
        offset: 0,
        limit: 100,
        search: participantsSearch.trim() || undefined,
      });
      participants = page.users;
      participantsCount = page.count;
    } catch (e: any) {
      participantsError = typeof e === "string" ? e : (e?.message ?? "Erro ao carregar membros");
    } finally {
      participantsLoading = false;
    }
  }

  function selectTab(t: "info" | "members") {
    tab = t;
    if (t === "members" && participants.length === 0) {
      loadParticipants();
    }
  }

  function selectFilter(f: TelegramParticipantFilter) {
    participantsFilter = f;
    if (f === "search") return;
    loadParticipants();
  }

  function onSearchInput() {
    if (searchDebounce) clearTimeout(searchDebounce);
    searchDebounce = setTimeout(() => {
      if (participantsSearch.trim()) {
        participantsFilter = "search";
      }
      loadParticipants();
    }, 350);
  }

  async function toggleMute() {
    if (!chat) return;
    const next = !muted;
    try {
      await telegramSetMute({
        chatId: chat.id,
        chatType,
        muteUntil: next ? 0 : -1,
      });
      muted = next;
      showToast("info", next ? "Silenciado" : "Reativado");
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
    }
  }

  async function togglePin() {
    if (!chat) return;
    const next = !pinned;
    try {
      await telegramTogglePin({ chatId: chat.id, chatType, pinned: next });
      pinned = next;
      showToast("info", next ? "Fixado" : "Desfixado");
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
    }
  }

  async function toggleArchive() {
    if (!chat) return;
    const next = !archived;
    try {
      await telegramSetArchived({ chatId: chat.id, chatType, archived: next });
      archived = next;
      showToast("info", next ? "Arquivado" : "Desarquivado");
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
    }
  }

  async function toggleBlock() {
    if (!chat) return;
    const next = !blocked;
    try {
      await telegramSetBlocked({ chatId: chat.id, chatType, blocked: next });
      blocked = next;
      showToast("info", next ? "Bloqueado" : "Desbloqueado");
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
    }
  }

  async function commitConfirm() {
    if (!chat || confirm.busy) return;
    confirm.busy = true;
    const id = chat.id;
    try {
      if (confirm.kind === "leave") {
        await telegramLeaveChannel({ chatId: id, chatType });
        showToast("info", "Você saiu do canal");
        confirm.kind = null;
        open = false;
        onChatRemoved?.(id);
      } else if (confirm.kind === "delete-channel") {
        await telegramDeleteChannel(id);
        showToast("info", "Canal deletado");
        confirm.kind = null;
        open = false;
        onChatRemoved?.(id);
      } else if (confirm.kind === "clear-history") {
        if (clearMode === "leave") {
          await telegramLeaveChannel({ chatId: id, chatType });
          showToast("info", "Você saiu do chat");
          onChatRemoved?.(id);
          open = false;
        } else {
          await telegramDeleteHistory({
            chatId: id,
            chatType,
            justClear: true,
            revoke: clearMode === "delete-all",
          });
          showToast("info", clearMode === "delete-all" ? "Histórico apagado pra todos" : "Histórico limpo");
        }
        confirm.kind = null;
      } else if (confirm.kind === "report") {
        await telegramReportPeer({
          chatId: id,
          chatType,
          messageIds: [],
          option: [0],
          message: reportMessage.trim(),
        });
        showToast("info", "Denúncia enviada");
        reportMessage = "";
        confirm.kind = null;
      }
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
    } finally {
      confirm.busy = false;
    }
  }

  function close() {
    if (confirm.busy) return;
    open = false;
    confirm.kind = null;
  }

  function roleLabel(role: string): string {
    switch (role) {
      case "creator": return "Criador";
      case "admin": return "Admin";
      case "member": return "Membro";
      case "banned": return "Banido";
      case "left": return "Saiu";
      case "self": return "Você";
      default: return role;
    }
  }

  function userDisplay(u: TelegramParticipant): string {
    const name = `${u.first_name} ${u.last_name}`.trim();
    if (name) return name;
    if (u.username) return `@${u.username}`;
    return `Usuário ${u.user_id}`;
  }
</script>

{#if open && chat}
  <div
    class="drawer-overlay"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) close(); }}
    onkeydown={(e) => { if (e.key === "Escape") close(); }}
  >
    <aside class="drawer" role="dialog" aria-modal="true" aria-label="Canal: {chat.title}">
      <header class="drawer-header">
        <div class="header-info">
          <div class="header-avatar">{chat.title.charAt(0).toUpperCase()}</div>
          <div class="header-text">
            <h2>{chat.title}</h2>
            <span class="header-meta">
              {isChannel ? "Canal" : isGroup ? "Grupo" : "Privado"}
              {#if info?.participants_count}
                · {info.participants_count.toLocaleString()} membros
              {/if}
              {#if info?.username}
                · @{info.username}
              {/if}
            </span>
          </div>
        </div>
        <button type="button" class="icon-btn close-btn" onclick={close} aria-label="Fechar">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 6L6 18" />
            <path d="M6 6l12 12" />
          </svg>
        </button>
      </header>

      {#if !isPrivate}
        <nav class="drawer-tabs" role="tablist">
          <button
            type="button"
            class="tab-btn"
            class:active={tab === "info"}
            role="tab"
            aria-selected={tab === "info"}
            onclick={() => selectTab("info")}
          >
            Info
          </button>
          <button
            type="button"
            class="tab-btn"
            class:active={tab === "members"}
            role="tab"
            aria-selected={tab === "members"}
            onclick={() => selectTab("members")}
          >
            Membros
          </button>
        </nav>
      {/if}

      <div class="drawer-body">
        {#if tab === "info" || isPrivate}
          {#if infoLoading}
            <div class="loading-section"><span class="spinner"></span></div>
          {:else if infoError}
            <div class="error-section">
              <p class="error-msg">{infoError}</p>
              <button type="button" class="button" onclick={loadInfo}>Tentar novamente</button>
            </div>
          {:else if info}
            {#if info.about}
              <section class="info-block">
                <span class="info-label">Sobre</span>
                <p class="info-text">{info.about}</p>
              </section>
            {/if}

            <section class="actions-grid">
              <button type="button" class="action-row" onclick={toggleMute}>
                <span class="action-icon">{muted ? "🔔" : "🔕"}</span>
                <span class="action-label">{muted ? "Reativar notificações" : "Silenciar"}</span>
              </button>
              <button type="button" class="action-row" onclick={togglePin}>
                <span class="action-icon">{pinned ? "📌" : "📍"}</span>
                <span class="action-label">{pinned ? "Desfixar" : "Fixar no topo"}</span>
              </button>
              <button type="button" class="action-row" onclick={toggleArchive}>
                <span class="action-icon">{archived ? "📂" : "📁"}</span>
                <span class="action-label">{archived ? "Desarquivar" : "Arquivar"}</span>
              </button>
              {#if isPrivate}
                <button type="button" class="action-row" onclick={toggleBlock}>
                  <span class="action-icon">{blocked ? "✓" : "🚫"}</span>
                  <span class="action-label">{blocked ? "Desbloquear" : "Bloquear"}</span>
                </button>
              {/if}
            </section>

            <section class="danger-zone">
              <span class="info-label">Ações</span>
              <button
                type="button"
                class="action-row danger"
                onclick={() => { clearMode = "clear-me"; confirm.kind = "clear-history"; }}
              >
                <span class="action-icon">🧹</span>
                <span class="action-label">Limpar histórico</span>
              </button>
              <button
                type="button"
                class="action-row danger"
                onclick={() => { confirm.kind = "report"; }}
              >
                <span class="action-icon">⚠️</span>
                <span class="action-label">Denunciar</span>
              </button>
              {#if !isPrivate}
                <button
                  type="button"
                  class="action-row danger"
                  onclick={() => { confirm.kind = "leave"; }}
                >
                  <span class="action-icon">🚪</span>
                  <span class="action-label">Sair {isChannel ? "do canal" : "do grupo"}</span>
                </button>
                <button
                  type="button"
                  class="action-row danger"
                  onclick={() => { confirm.kind = "delete-channel"; }}
                >
                  <span class="action-icon">🗑️</span>
                  <span class="action-label">Deletar {isChannel ? "canal" : "grupo"} (apenas dono)</span>
                </button>
              {/if}
            </section>
          {/if}
        {:else if tab === "members"}
          <section class="members-section">
            <div class="members-filters">
              {#each [
                { key: "recent", label: "Todos" },
                { key: "admins", label: "Admins" },
                { key: "bots", label: "Bots" },
                { key: "banned", label: "Banidos" },
              ] as f}
                <button
                  type="button"
                  class="filter-pill"
                  class:active={participantsFilter === f.key}
                  onclick={() => selectFilter(f.key as TelegramParticipantFilter)}
                >
                  {f.label}
                </button>
              {/each}
            </div>
            <input
              type="text"
              class="input"
              placeholder="Buscar membro..."
              bind:value={participantsSearch}
              oninput={onSearchInput}
            />
            {#if participantsLoading}
              <div class="loading-section"><span class="spinner"></span></div>
            {:else if participantsError}
              <div class="error-section">
                <p class="error-msg">{participantsError}</p>
                <button type="button" class="button" onclick={loadParticipants}>Tentar novamente</button>
              </div>
            {:else if participants.length === 0}
              <p class="empty-text">Nenhum membro encontrado.</p>
            {:else}
              <div class="members-meta">
                {participantsCount.toLocaleString()} no total
              </div>
              <ul class="members-list">
                {#each participants as p (p.user_id)}
                  <li class="member-item">
                    <div class="member-avatar">{p.first_name.charAt(0).toUpperCase() || p.username?.charAt(0).toUpperCase() || "?"}</div>
                    <div class="member-info">
                      <span class="member-name">{userDisplay(p)}</span>
                      {#if p.username}
                        <span class="member-username">@{p.username}</span>
                      {/if}
                    </div>
                    {#if p.is_bot}
                      <span class="role-badge bot">BOT</span>
                    {/if}
                    <span class="role-badge" class:role-creator={p.role === "creator"} class:role-admin={p.role === "admin"} class:role-banned={p.role === "banned"}>
                      {roleLabel(p.role)}
                    </span>
                  </li>
                {/each}
              </ul>
            {/if}
          </section>
        {/if}
      </div>
    </aside>
  </div>
{/if}

{#if confirm.kind === "leave"}
  <div class="dialog-overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget && !confirm.busy) confirm.kind = null; }} onkeydown={() => {}}>
    <div class="dialog" role="dialog" aria-modal="true">
      <h3>Sair {isChannel ? "do canal" : "do grupo"}?</h3>
      <p>Você não receberá mais mensagens de <strong>{chat?.title}</strong>.</p>
      <div class="dialog-actions">
        <button type="button" class="button" onclick={() => (confirm.kind = null)} disabled={confirm.busy}>Cancelar</button>
        <button type="button" class="button danger-btn" onclick={commitConfirm} disabled={confirm.busy}>
          {confirm.busy ? "Saindo..." : "Sair"}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if confirm.kind === "delete-channel"}
  <div class="dialog-overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget && !confirm.busy) confirm.kind = null; }} onkeydown={() => {}}>
    <div class="dialog" role="dialog" aria-modal="true">
      <h3>Deletar {isChannel ? "canal" : "grupo"}?</h3>
      <p class="warn">⚠️ Irreversível. Todos os membros perdem acesso ao conteúdo.</p>
      <p>Confirme deletando <strong>{chat?.title}</strong>.</p>
      <div class="dialog-actions">
        <button type="button" class="button" onclick={() => (confirm.kind = null)} disabled={confirm.busy}>Cancelar</button>
        <button type="button" class="button danger-btn" onclick={commitConfirm} disabled={confirm.busy}>
          {confirm.busy ? "Deletando..." : "Deletar"}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if confirm.kind === "clear-history"}
  <div class="dialog-overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget && !confirm.busy) confirm.kind = null; }} onkeydown={() => {}}>
    <div class="dialog" role="dialog" aria-modal="true">
      <h3>Limpar conversa</h3>
      <p>Como você quer limpar <strong>{chat?.title}</strong>?</p>
      <div class="radio-group">
        <label class="radio-row">
          <input type="radio" bind:group={clearMode} value="clear-me" />
          <div>
            <span class="radio-title">Limpar pra mim</span>
            <span class="radio-desc">Some do seu lado. Outros continuam vendo.</span>
          </div>
        </label>
        {#if isPrivate || isGroup}
          <label class="radio-row">
            <input type="radio" bind:group={clearMode} value="delete-all" />
            <div>
              <span class="radio-title">Apagar pra todos</span>
              <span class="radio-desc">Remove a conversa de todos os participantes.</span>
            </div>
          </label>
        {/if}
        {#if !isPrivate}
          <label class="radio-row">
            <input type="radio" bind:group={clearMode} value="leave" />
            <div>
              <span class="radio-title">Sair {isChannel ? "do canal" : "do grupo"}</span>
              <span class="radio-desc">Deixa o {isChannel ? "canal" : "grupo"} e remove da lista.</span>
            </div>
          </label>
        {/if}
      </div>
      <div class="dialog-actions">
        <button type="button" class="button" onclick={() => (confirm.kind = null)} disabled={confirm.busy}>Cancelar</button>
        <button type="button" class="button danger-btn" onclick={commitConfirm} disabled={confirm.busy}>
          {confirm.busy ? "Aplicando..." : "Confirmar"}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if confirm.kind === "report"}
  <div class="dialog-overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget && !confirm.busy) confirm.kind = null; }} onkeydown={() => {}}>
    <div class="dialog" role="dialog" aria-modal="true">
      <h3>Denunciar {chat?.title}</h3>
      <p>Telegram revisará a denúncia.</p>
      <textarea
        class="input textarea"
        placeholder="Detalhes (opcional)"
        bind:value={reportMessage}
        rows="3"
      ></textarea>
      <div class="dialog-actions">
        <button type="button" class="button" onclick={() => (confirm.kind = null)} disabled={confirm.busy}>Cancelar</button>
        <button type="button" class="button danger-btn" onclick={commitConfirm} disabled={confirm.busy}>
          {confirm.busy ? "Enviando..." : "Enviar denúncia"}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .drawer-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    z-index: 100;
    display: flex;
    justify-content: flex-end;
    animation: fade-in 150ms ease;
  }

  .drawer {
    width: min(420px, 100vw);
    height: 100%;
    background: var(--surface, var(--button));
    display: flex;
    flex-direction: column;
    animation: slide-in 200ms cubic-bezier(0.22, 1, 0.36, 1);
    box-shadow: -2px 0 12px rgba(0, 0, 0, 0.2);
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes slide-in {
    from { transform: translateX(20px); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }

  .drawer-header {
    display: flex;
    align-items: center;
    gap: var(--padding);
    padding: var(--padding);
    border-bottom: none;
  }

  .header-info {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--padding);
    min-width: 0;
  }

  .header-avatar {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background: var(--blue);
    color: #fff;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 18px;
    font-weight: 600;
    flex-shrink: 0;
  }

  .header-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .header-text h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .header-meta {
    font-size: 12px;
    color: var(--gray);
  }

  .icon-btn {
    background: transparent;
    border: none;
    color: var(--gray);
    cursor: pointer;
    padding: 8px;
    border-radius: var(--border-radius);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .icon-btn:hover {
    background: var(--button-elevated);
    color: var(--secondary);
  }

  .drawer-tabs {
    display: flex;
    border-bottom: none;
  }

  .tab-btn {
    flex: 1;
    padding: 12px;
    background: transparent;
    border: none;
    color: var(--gray);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    transition: color 150ms, border-color 150ms;
  }

  .tab-btn.active {
    color: var(--blue);
    border-bottom-color: var(--blue);
  }

  .tab-btn:not(.active):hover {
    color: var(--secondary);
  }

  .drawer-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--padding);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
  }

  .info-block {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .info-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .info-text {
    margin: 0;
    font-size: 13px;
    color: var(--secondary);
    line-height: 1.5;
    white-space: pre-wrap;
  }

  .actions-grid,
  .danger-zone {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .danger-zone {
    margin-top: var(--padding);
    padding-top: var(--padding);
    border-top: none;
  }

  .action-row {
    display: flex;
    align-items: center;
    gap: var(--padding);
    padding: 10px 12px;
    background: transparent;
    border: none;
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    transition: background 150ms;
  }

  .action-row:hover {
    background: var(--button-elevated);
  }

  .action-row.danger {
    color: var(--red);
  }

  .action-row.danger:hover {
    background: color-mix(in oklab, var(--red) 12%, transparent);
  }

  .action-icon {
    width: 24px;
    text-align: center;
    flex-shrink: 0;
  }

  .action-label {
    flex: 1;
  }

  .members-section {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .members-filters {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .filter-pill {
    padding: 4px 10px;
    background: var(--button-elevated);
    border: none;
    border-radius: 100px;
    color: var(--gray);
    font-family: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: background 150ms, color 150ms;
  }

  .filter-pill.active {
    background: var(--blue);
    color: #fff;
  }

  .filter-pill:not(.active):hover {
    color: var(--secondary);
  }

  .members-meta {
    font-size: 11px;
    color: var(--gray);
  }

  .members-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .member-item {
    display: flex;
    align-items: center;
    gap: var(--padding);
    padding: 8px;
    border-radius: var(--border-radius);
  }

  .member-item:hover {
    background: var(--button-elevated);
  }

  .member-avatar {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: var(--blue);
    color: #fff;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    font-weight: 600;
    flex-shrink: 0;
  }

  .member-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .member-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .member-username {
    font-size: 11px;
    color: var(--gray);
  }

  .role-badge {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--button-elevated);
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }

  .role-badge.bot {
    background: var(--blue);
    color: #fff;
  }

  .role-badge.role-creator {
    background: var(--warning);
    color: #fff;
  }

  .role-badge.role-admin {
    background: var(--green);
    color: #fff;
  }

  .role-badge.role-banned {
    background: var(--red);
    color: #fff;
  }

  .loading-section,
  .error-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    padding: calc(var(--padding) * 3);
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .error-msg {
    color: var(--red);
    font-size: 12px;
    margin: 0;
  }

  .empty-text {
    text-align: center;
    color: var(--gray);
    font-size: 13px;
    margin: 0;
    padding: var(--padding);
  }

  .input {
    width: 100%;
    padding: 8px 12px;
    background: var(--button);
    border: none;
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
  }

  .input:focus-visible {
    outline: none;
    border-color: var(--blue);
  }

  .textarea {
    resize: vertical;
    min-height: 60px;
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
    animation: fade-in 150ms ease;
  }

  .dialog {
    background: var(--surface, var(--button));
    border-radius: var(--border-radius);
    padding: calc(var(--padding) * 1.5);
    max-width: 400px;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
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

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }

  .button {
    padding: 8px 16px;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background 150ms;
  }

  .button:hover:not(:disabled) {
    background: color-mix(in oklab, var(--secondary) 8%, var(--button-elevated));
  }

  .button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .danger-btn {
    background: var(--red);
    color: #fff;
  }

  .danger-btn:hover:not(:disabled) {
    background: color-mix(in oklab, var(--red) 88%, #000);
  }

  .radio-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .radio-row {
    display: flex;
    align-items: flex-start;
    gap: var(--padding);
    padding: 10px;
    background: var(--button-elevated);
    border-radius: var(--border-radius);
    cursor: pointer;
  }

  .radio-row input[type="radio"] {
    margin-top: 2px;
  }

  .radio-row > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .radio-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
  }

  .radio-desc {
    font-size: 11px;
    color: var(--gray);
  }

  .close-btn {
    flex-shrink: 0;
  }
</style>
