<script lang="ts">
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";
  import {
    telegramCloneStart,
    telegramCloneList,
    telegramClonePause,
    telegramCloneCancel,
    telegramCloneDelete,
    type TelegramCloneSession,
    type TelegramCloneProgressEvent,
    type TelegramChatType,
  } from "$lib/study-telegram-bridge";

  type ChatLite = { id: number; title: string; chat_type: string };

  let {
    open = $bindable(false),
    chats,
  } = $props<{
    open: boolean;
    chats: ChatLite[];
  }>();

  let view = $state<"list" | "new">("list");
  let sessions = $state<TelegramCloneSession[]>([]);
  let loading = $state(false);
  let error = $state("");

  let sourceId = $state<number | null>(null);
  let destMode = $state<"auto" | "existing">("auto");
  let destId = $state<number | null>(null);
  let destTitle = $state("");
  let delayMs = $state(2000);
  let batchSize = $state(50);
  let limitEnabled = $state(false);
  let limit = $state(100);
  let dropAuthor = $state(false);
  let dropCaptions = $state(false);
  let starting = $state(false);

  let unlistenProgress: UnlistenFn | null = null;

  $effect(() => {
    if (open) {
      view = sessions.length > 0 ? "list" : "new";
      refreshSessions();
      attachProgressListener();
    } else {
      detachProgressListener();
    }
  });

  async function attachProgressListener() {
    if (unlistenProgress) return;
    unlistenProgress = await listen<TelegramCloneProgressEvent>("telegram:clone:progress", (ev) => {
      const p = ev.payload;
      sessions = sessions.map((s) =>
        s.id === p.session_id
          ? {
              ...s,
              total_collected: p.total,
              cloned_count: p.current,
              failed_count: p.failed,
              last_message_id: p.last_message_id,
              status:
                p.stage === "completed"
                  ? "completed"
                  : p.stage === "paused"
                  ? "paused"
                  : p.stage === "error"
                  ? "error"
                  : s.status,
              error: p.error ?? s.error,
            }
          : s,
      );
    });
  }

  function detachProgressListener() {
    unlistenProgress?.();
    unlistenProgress = null;
  }

  async function refreshSessions() {
    loading = true;
    error = "";
    try {
      sessions = await telegramCloneList();
    } catch (e: any) {
      error = typeof e === "string" ? e : (e?.message ?? $t("common.error"));
    } finally {
      loading = false;
    }
  }

  function close() {
    open = false;
  }

  function selectableChats(): ChatLite[] {
    return (chats as ChatLite[]).filter((c: ChatLite) => c.chat_type !== "private");
  }

  function chatById(id: number | null): ChatLite | null {
    if (id == null) return null;
    return (chats as ChatLite[]).find((c: ChatLite) => c.id === id) ?? null;
  }

  async function startClone() {
    const source = chatById(sourceId);
    if (!source) {
      showToast("error", "Selecione uma origem");
      return;
    }
    starting = true;
    try {
      const result = await telegramCloneStart({
        sourceId: source.id,
        sourceType: source.chat_type as TelegramChatType,
        sourceTitle: source.title,
        destId: destMode === "existing" && destId != null ? destId : undefined,
        destType:
          destMode === "existing" && destId != null
            ? (chatById(destId)?.chat_type as TelegramChatType)
            : undefined,
        destTitle:
          destMode === "auto" && destTitle.trim() ? destTitle.trim() : undefined,
        options: {
          delay_ms: delayMs,
          batch_size: batchSize,
          limit: limitEnabled ? limit : null,
          drop_author: dropAuthor,
          drop_captions: dropCaptions,
        },
      });
      showToast("info", `Clone iniciado: ${result.dest_title}`);
      view = "list";
      await refreshSessions();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? $t("common.error")));
    } finally {
      starting = false;
    }
  }

  async function pauseSession(s: TelegramCloneSession) {
    try {
      await telegramClonePause({ sessionId: s.id });
      await refreshSessions();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? $t("common.error")));
    }
  }

  async function resumeSession(s: TelegramCloneSession) {
    try {
      await telegramCloneStart({
        sourceId: s.source_chat_id,
        sourceType: s.source_chat_type,
        sourceTitle: s.source_title,
        destId: s.dest_chat_id,
        destType: s.dest_chat_type,
        destTitle: s.dest_title,
        resumeId: s.id,
        options: {
          delay_ms: s.options.delay_ms,
          batch_size: s.options.batch_size,
          limit: s.options.limit ?? null,
          drop_author: s.options.drop_author,
          drop_captions: s.options.drop_captions,
        },
      });
      await refreshSessions();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? $t("common.error")));
    }
  }

  async function cancelSession(s: TelegramCloneSession) {
    try {
      await telegramCloneCancel({ sessionId: s.id });
      await refreshSessions();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? $t("common.error")));
    }
  }

  async function deleteSession(s: TelegramCloneSession) {
    try {
      await telegramCloneDelete({ sessionId: s.id });
      await refreshSessions();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? $t("common.error")));
    }
  }

  function pct(s: TelegramCloneSession): number {
    if (s.total_collected === 0) return 0;
    return Math.min(100, (s.cloned_count / s.total_collected) * 100);
  }

  function statusLabel(status: string): string {
    switch (status) {
      case "running": return $t("telegram.toolbox.status_running");
      case "paused": return $t("telegram.toolbox.status_paused");
      case "completed": return $t("telegram.toolbox.status_completed");
      case "error": return $t("common.error");
      case "cancelled": return $t("telegram.toolbox.status_cancelled");
      default: return status;
    }
  }
</script>

{#if open}
  <div
    class="overlay"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) close(); }}
    onkeydown={(e) => { if (e.key === "Escape") close(); }}
  >
    <div class="panel" role="dialog" aria-modal="true" aria-label={$t("telegram.clone_channels")} tabindex="-1">
      <header class="panel-header">
        <div>
          <h2>{$t("telegram.clone_channels")}</h2>
          <p class="subtitle">{$t("telegram.toolbox.clone_subtitle")}</p>
        </div>
        <button type="button" class="icon-btn" onclick={close} aria-label={$t("common.close")}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 6L6 18" />
            <path d="M6 6l12 12" />
          </svg>
        </button>
      </header>

      <div class="tabs" role="tablist">
        <button
          type="button"
          class="tab"
          class:active={view === "list"}
          role="tab"
          aria-selected={view === "list"}
          onclick={() => (view = "list")}
        >
          {$t("telegram.toolbox.sessions")} {sessions.length > 0 ? `(${sessions.length})` : ""}
        </button>
        <button
          type="button"
          class="tab"
          class:active={view === "new"}
          role="tab"
          aria-selected={view === "new"}
          onclick={() => (view = "new")}
        >
          {$t("telegram.toolbox.new_clone")}
        </button>
      </div>

      <div class="body">
        {#if view === "new"}
          <section class="form-section">
            <label class="field">
              <span class="field-label">{$t("telegram.toolbox.source")}</span>
              <select class="input" bind:value={sourceId}>
                <option value={null}>— {$t("telegram.toolbox.select_channel")} —</option>
                {#each selectableChats() as c (c.id)}
                  <option value={c.id}>{c.title}</option>
                {/each}
              </select>
            </label>

            <fieldset class="dest-fieldset">
              <legend class="field-label">{$t("telegram.toolbox.destination")}</legend>
              <label class="radio-row">
                <input type="radio" bind:group={destMode} value="auto" />
                <div>
                  <span class="radio-title">{$t("telegram.toolbox.create_new_channel")}</span>
                  <span class="radio-desc">{$t("telegram.toolbox.create_new_channel_desc")}</span>
                </div>
              </label>
              {#if destMode === "auto"}
                <input
                  type="text"
                  class="input dest-title"
                  placeholder={$t("telegram.toolbox.new_channel_placeholder")}
                  bind:value={destTitle}
                />
              {/if}
              <label class="radio-row">
                <input type="radio" bind:group={destMode} value="existing" />
                <div>
                  <span class="radio-title">{$t("telegram.toolbox.existing_channel")}</span>
                  <span class="radio-desc">{$t("telegram.toolbox.existing_channel_desc")}</span>
                </div>
              </label>
              {#if destMode === "existing"}
                <select class="input dest-title" bind:value={destId}>
                  <option value={null}>— {$t("telegram.toolbox.select")} —</option>
                  {#each selectableChats().filter((c) => c.id !== sourceId) as c (c.id)}
                    <option value={c.id}>{c.title}</option>
                  {/each}
                </select>
              {/if}
            </fieldset>

            <details class="advanced">
              <summary>{$t("telegram.toolbox.advanced_options")}</summary>
              <div class="advanced-grid">
                <label class="field">
                  <span class="field-label">{$t("telegram.toolbox.batch_delay")}</span>
                  <input type="number" class="input" min="0" max="60000" bind:value={delayMs} />
                </label>
                <label class="field">
                  <span class="field-label">{$t("telegram.toolbox.batch_size")}</span>
                  <input type="number" class="input" min="1" max="100" bind:value={batchSize} />
                </label>
                <label class="checkbox-row">
                  <input type="checkbox" bind:checked={limitEnabled} />
                  <span>{$t("telegram.toolbox.limit_messages")}</span>
                </label>
                {#if limitEnabled}
                  <label class="field">
                    <span class="field-label">{$t("telegram.toolbox.max_messages")}</span>
                    <input type="number" class="input" min="1" bind:value={limit} />
                  </label>
                {/if}
                <label class="checkbox-row">
                  <input type="checkbox" bind:checked={dropAuthor} />
                  <span>{$t("telegram.toolbox.drop_author")}</span>
                </label>
                <label class="checkbox-row">
                  <input type="checkbox" bind:checked={dropCaptions} />
                  <span>{$t("telegram.toolbox.drop_captions")}</span>
                </label>
              </div>
            </details>

            <div class="actions">
              <button type="button" class="button" onclick={close} disabled={starting}>{$t("common.cancel")}</button>
              <button
                type="button"
                class="button primary"
                onclick={startClone}
                disabled={starting || sourceId == null}
              >
                {starting ? $t("telegram.toolbox.starting") : $t("telegram.toolbox.start_clone")}
              </button>
            </div>
          </section>
        {:else}
          <section class="sessions-section">
            {#if loading}
              <div class="status"><span class="spinner"></span></div>
            {:else if error}
              <div class="status status-error">{error}</div>
            {:else if sessions.length === 0}
              <div class="status">
                <p>{$t("telegram.toolbox.clone_empty")}</p>
                <button type="button" class="button primary" onclick={() => (view = "new")}>{$t("telegram.toolbox.create_new")}</button>
              </div>
            {:else}
              <ul class="session-list">
                {#each sessions as s (s.id)}
                  <li class="session-item">
                    <div class="session-header">
                      <div class="session-titles">
                        <span class="session-source">{s.source_title}</span>
                        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="session-arrow">
                          <path d="M5 12h14M13 6l6 6-6 6" />
                        </svg>
                        <span class="session-dest">{s.dest_title}</span>
                      </div>
                      <span class="status-badge status-{s.status}">{statusLabel(s.status)}</span>
                    </div>
                    <div class="progress-outer">
                      <div
                        class="progress-inner"
                        class:status-error={s.status === "error"}
                        class:status-completed={s.status === "completed"}
                        style:width="{pct(s)}%"
                      ></div>
                    </div>
                    <div class="session-meta">
                      <span>{$t("telegram.toolbox.message_progress", { cloned: s.cloned_count, total: s.total_collected || "?" })}</span>
                      {#if s.failed_count > 0}<span class="failed">· {$t("telegram.toolbox.failures", { count: s.failed_count })}</span>{/if}
                      <span>· delay {s.options.delay_ms}ms</span>
                    </div>
                    {#if s.error}
                      <p class="session-error">{s.error}</p>
                    {/if}
                    <div class="session-actions">
                      {#if s.status === "running"}
                        <button type="button" class="button" onclick={() => pauseSession(s)}>{$t("telegram.toolbox.pause")}</button>
                      {:else if s.status === "paused" || s.status === "error"}
                        <button type="button" class="button primary" onclick={() => resumeSession(s)}>{$t("telegram.toolbox.resume")}</button>
                      {/if}
                      {#if s.status === "running" || s.status === "paused"}
                        <button type="button" class="button danger" onclick={() => cancelSession(s)}>{$t("common.cancel")}</button>
                      {/if}
                      {#if s.status !== "running"}
                        <button type="button" class="button ghost" onclick={() => deleteSession(s)}>{$t("telegram.accounts_remove")}</button>
                      {/if}
                    </div>
                  </li>
                {/each}
              </ul>
            {/if}
          </section>
        {/if}
      </div>
    </div>
  </div>
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
    width: min(560px, 100vw);
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

  .tabs {
    display: flex;
    border-bottom: 1px solid var(--input-border);
  }

  .tab {
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
  }

  .tab.active {
    color: var(--blue);
    border-bottom-color: var(--blue);
  }

  .tab:not(.active):hover {
    color: var(--secondary);
  }

  .body {
    flex: 1;
    overflow-y: auto;
    padding: var(--padding);
  }

  .form-section {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .field-label {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }

  .input {
    width: 100%;
    padding: 8px 12px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
  }

  .input:focus-visible {
    outline: none;
    border-color: var(--blue);
  }

  .dest-fieldset {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
    padding: var(--padding);
    margin: 0;
  }

  .dest-fieldset legend {
    padding: 0 6px;
  }

  .radio-row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 8px;
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

  .dest-title {
    margin-left: 32px;
  }

  .advanced {
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
    padding: 8px var(--padding);
  }

  .advanced summary {
    cursor: pointer;
    font-size: 13px;
    color: var(--secondary);
    user-select: none;
  }

  .advanced-grid {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: var(--padding);
  }

  .checkbox-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--secondary);
    cursor: pointer;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding-top: var(--padding);
    border-top: 1px solid var(--input-border);
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
  }

  .button:hover:not(:disabled) {
    background: color-mix(in oklab, var(--secondary) 8%, var(--button-elevated));
  }

  .button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .button.primary {
    background: var(--blue);
    color: #fff;
  }

  .button.primary:hover:not(:disabled) {
    background: color-mix(in oklab, var(--blue) 90%, #000);
  }

  .button.danger {
    background: var(--red);
    color: #fff;
  }

  .button.ghost {
    background: transparent;
    color: var(--gray);
    border: 1px solid var(--input-border);
  }

  .sessions-section {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .session-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .session-item {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: var(--padding);
    background: var(--button);
    border-radius: var(--border-radius);
  }

  .session-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }

  .session-titles {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    font-size: 13px;
    color: var(--secondary);
  }

  .session-source,
  .session-dest {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .session-arrow {
    color: var(--gray);
    flex-shrink: 0;
  }

  .status-badge {
    font-size: 10.5px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--button-elevated);
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }

  .status-running {
    background: var(--blue);
    color: #fff;
  }

  .status-completed {
    background: var(--green, #10b981);
    color: #fff;
  }

  .status-error,
  .status-cancelled {
    background: var(--red);
    color: #fff;
  }

  .status-paused {
    background: var(--gold, #f59e0b);
    color: #fff;
  }

  .progress-outer {
    width: 100%;
    height: 6px;
    background: var(--button-elevated);
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-inner {
    height: 100%;
    background: var(--blue);
    border-radius: 3px;
    transition: width 200ms;
  }

  .progress-inner.status-error {
    background: var(--red);
  }

  .progress-inner.status-completed {
    background: var(--green, #10b981);
  }

  .session-meta {
    font-size: 11.5px;
    color: var(--gray);
  }

  .session-meta .failed {
    color: var(--red);
  }

  .session-error {
    margin: 0;
    font-size: 11.5px;
    color: var(--red);
  }

  .session-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .session-actions .button {
    padding: 4px 12px;
    font-size: 12px;
  }

  .status {
    text-align: center;
    padding: calc(var(--padding) * 3);
    color: var(--gray);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
  }

  .status-error {
    color: var(--red);
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
</style>
