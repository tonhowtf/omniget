<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { onBatchFileStatus, type BatchFileStatusPayload } from "$lib/stores/download-listener";
  import { t } from "$lib/i18n";

  type TelegramChat = {
    id: number;
    title: string;
    chat_type: string;
  };

  type TelegramMediaItem = {
    message_id: number;
    file_name: string;
    file_size: number;
    media_type: string;
    date: number;
  };

  type QrStartResponse = {
    svg: string;
    expires: number;
  };

  type FileStatus = "waiting" | "downloading" | "done" | "error" | "skipped";

  type View = "checking" | "qr" | "phone" | "code" | "password" | "chats" | "media";

  let view: View = $state("checking");
  let phone = $state("");
  let code = $state("");
  let password = $state("");
  let passwordHint = $state("");
  let sessionPhone = $state("");
  let loading = $state(false);
  let error = $state("");

  let qrSvg = $state("");
  let qrLoading = $state(false);
  let qrError = $state("");
  let qrPollTimer: ReturnType<typeof setInterval> | null = $state(null);
  let qrRefreshTimer: ReturnType<typeof setTimeout> | null = $state(null);

  let chats: TelegramChat[] = $state([]);
  let loadingChats = $state(false);
  let chatsError = $state("");
  let chatSearch = $state("");

  let selectedChat: TelegramChat | null = $state(null);
  let mediaItems: TelegramMediaItem[] = $state([]);
  let loadingMedia = $state(false);
  let mediaError = $state("");
  let mediaFilter = $state("all");
  let loadingMore = $state(false);
  let hasMore = $state(true);

  // Batch download state
  let batchStatus: Map<number, { status: FileStatus; percent: number }> = $state(new Map());
  let activeBatchId: number | null = $state(null);
  let batchDone = $state(0);
  let batchTotal = $state(0);

  let isBatchActive = $derived(activeBatchId !== null);
  let batchPercent = $derived(batchTotal > 0 ? (batchDone / batchTotal) * 100 : 0);

  // Single-file downloading (kept for individual downloads)
  let downloadingIds: Set<number> = $state(new Set());

  let filteredChats = $derived(
    chatSearch.trim()
      ? chats.filter((c) =>
          c.title.toLowerCase().includes(chatSearch.trim().toLowerCase())
        )
      : chats
  );

  $effect(() => {
    checkSession();

    onBatchFileStatus(handleBatchFileStatus);

    return () => {
      stopQrPolling();
      onBatchFileStatus(null);
    };
  });

  function handleBatchFileStatus(payload: BatchFileStatusPayload) {
    if (payload.batch_id !== activeBatchId) return;

    batchStatus.set(payload.message_id, {
      status: payload.status,
      percent: payload.percent,
    });
    batchStatus = new Map(batchStatus);

    // Count completed items
    let done = 0;
    for (const [, entry] of batchStatus) {
      if (entry.status === "done" || entry.status === "error" || entry.status === "skipped") {
        done++;
      }
    }
    batchDone = done;

    // Batch finished
    if (done >= batchTotal && batchTotal > 0) {
      activeBatchId = null;
    }
  }

  function stopQrPolling() {
    if (qrPollTimer) {
      clearInterval(qrPollTimer);
      qrPollTimer = null;
    }
    if (qrRefreshTimer) {
      clearTimeout(qrRefreshTimer);
      qrRefreshTimer = null;
    }
  }

  async function checkSession() {
    view = "checking";
    try {
      const result = await invoke<string>("telegram_check_session");
      sessionPhone = result;
      view = "chats";
      loadChats();
    } catch {
      view = "qr";
      startQrLogin();
    }
  }

  async function startQrLogin() {
    qrLoading = true;
    qrError = "";
    qrSvg = "";
    stopQrPolling();

    try {
      const result = await invoke<QrStartResponse>("telegram_qr_start");
      qrSvg = result.svg;
      qrLoading = false;

      const now = Math.floor(Date.now() / 1000);
      const expiresIn = Math.max((result.expires - now) * 1000 - 2000, 5000);
      qrRefreshTimer = setTimeout(() => {
        if (view === "qr") startQrLogin();
      }, expiresIn);

      qrPollTimer = setInterval(pollQrLogin, 1500);
    } catch (e: any) {
      qrLoading = false;
      const msg = typeof e === "string" ? e : e.message ?? "";
      if (msg.includes("already_authenticated")) {
        checkSession();
      } else {
        qrError = msg || $t("telegram.qr_error");
      }
    }
  }

  async function pollQrLogin() {
    try {
      const status = await invoke<string>("telegram_qr_poll");
      if (status === "waiting") return;

      stopQrPolling();

      if (status === "password_required" || status.startsWith("password_required:")) {
        passwordHint = status.startsWith("password_required:")
          ? status.slice("password_required:".length)
          : "";
        view = "password";
      } else if (status.startsWith("success:")) {
        sessionPhone = status.slice("success:".length);
        view = "chats";
        loadChats();
      }
    } catch {
      // ignore transient poll errors
    }
  }

  function switchToPhone() {
    stopQrPolling();
    view = "phone";
  }

  function switchToQr() {
    error = "";
    view = "qr";
    startQrLogin();
  }

  async function handleSendCode() {
    error = "";
    loading = true;
    try {
      await invoke("telegram_send_code", { phone: phone.trim() });
      view = "code";
    } catch (e: any) {
      error = typeof e === "string" ? e : e.message ?? $t("telegram.unknown_error");
    } finally {
      loading = false;
    }
  }

  async function handleVerifyCode() {
    error = "";
    loading = true;
    try {
      const result = await invoke<string>("telegram_verify_code", { code: code.trim() });
      sessionPhone = result;
      view = "chats";
      loadChats();
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "";
      if (msg === "invalid_code") {
        error = $t("telegram.invalid_code");
      } else if (msg.startsWith("password_required:")) {
        passwordHint = msg.slice("password_required:".length);
        view = "password";
      } else {
        error = msg || $t("telegram.unknown_error");
      }
    } finally {
      loading = false;
    }
  }

  async function handleVerifyPassword() {
    error = "";
    loading = true;
    try {
      const result = await invoke<string>("telegram_verify_2fa", { password });
      sessionPhone = result;
      view = "chats";
      loadChats();
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "";
      if (msg === "invalid_password") {
        error = $t("telegram.invalid_password");
      } else {
        error = msg || $t("telegram.unknown_error");
      }
    } finally {
      loading = false;
    }
  }

  async function handleLogout() {
    stopQrPolling();
    try {
      await invoke("telegram_logout");
    } catch {}
    sessionPhone = "";
    chats = [];
    mediaItems = [];
    selectedChat = null;
    phone = "";
    code = "";
    password = "";
    error = "";
    view = "qr";
    startQrLogin();
  }

  async function loadChats() {
    loadingChats = true;
    chatsError = "";
    try {
      chats = await invoke("telegram_list_chats");
    } catch (e: any) {
      chatsError = typeof e === "string" ? e : e.message ?? $t("telegram.chats_error");
    } finally {
      loadingChats = false;
    }
  }

  async function selectChat(chat: TelegramChat) {
    selectedChat = chat;
    mediaFilter = "all";
    view = "media";
    batchStatus = new Map();
    activeBatchId = null;
    batchDone = 0;
    batchTotal = 0;
    downloadingIds = new Set();
    hasMore = true;
    loadMedia();
  }

  function backToChats() {
    selectedChat = null;
    mediaItems = [];
    mediaError = "";
    batchStatus = new Map();
    activeBatchId = null;
    batchDone = 0;
    batchTotal = 0;
    downloadingIds = new Set();
    view = "chats";
  }

  async function loadMedia() {
    if (!selectedChat) return;
    loadingMedia = true;
    mediaError = "";
    try {
      const items: TelegramMediaItem[] = await invoke("telegram_list_media", {
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        mediaType: mediaFilter === "all" ? null : mediaFilter,
        offset: 0,
        limit: 100,
      });
      mediaItems = items;
      hasMore = items.length >= 100;
    } catch (e: any) {
      mediaError = typeof e === "string" ? e : e.message ?? $t("telegram.media_error");
    } finally {
      loadingMedia = false;
    }
  }

  async function loadMoreMedia() {
    if (!selectedChat || loadingMore || !hasMore) return;
    loadingMore = true;
    try {
      const lastId = mediaItems.length > 0 ? mediaItems[mediaItems.length - 1].message_id : 0;
      const items: TelegramMediaItem[] = await invoke("telegram_list_media", {
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        mediaType: mediaFilter === "all" ? null : mediaFilter,
        offset: lastId,
        limit: 100,
      });
      if (items.length > 0) {
        mediaItems = [...mediaItems, ...items];
      }
      hasMore = items.length >= 100;
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "Error";
      showToast("error", msg);
    } finally {
      loadingMore = false;
    }
  }

  function changeFilter(filter: string) {
    mediaFilter = filter;
    batchStatus = new Map();
    activeBatchId = null;
    batchDone = 0;
    batchTotal = 0;
    hasMore = true;
    loadMedia();
  }

  function formatSize(bytes: number): string {
    if (bytes === 0) return "\u2014";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function formatDate(ts: number): string {
    return new Date(ts * 1000).toLocaleDateString();
  }

  function chatTypeLabel(type: string): string {
    const key = `telegram.chat_type_${type}` as const;
    return $t(key);
  }

  async function resolveOutputDir(): Promise<string | null> {
    const appSettings = getSettings();
    if (appSettings?.download.always_ask_path) {
      return (await open({ directory: true, title: $t("telegram.choose_folder") })) as string | null;
    }
    const defaultDir = appSettings?.download.default_output_dir ?? null;
    if (defaultDir) return defaultDir;
    return (await open({ directory: true, title: $t("telegram.choose_folder") })) as string | null;
  }

  async function downloadItem(item: TelegramMediaItem) {
    if (!selectedChat) return;
    if (downloadingIds.has(item.message_id)) return;

    const outputDir = await resolveOutputDir();
    if (!outputDir) return;

    downloadingIds = new Set([...downloadingIds, item.message_id]);

    try {
      await invoke("telegram_download_media", {
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        messageId: item.message_id,
        fileName: item.file_name,
        outputDir,
      });
      showToast("info", $t("toast.download_started", { name: item.file_name }));
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "Error";
      showToast("error", msg);
      downloadingIds = new Set([...downloadingIds].filter((id) => id !== item.message_id));
    }
  }

  async function downloadAll() {
    if (!selectedChat || isBatchActive || mediaItems.length === 0) return;

    const outputDir = await resolveOutputDir();
    if (!outputDir) return;

    const items = mediaItems.map((m) => ({
      message_id: m.message_id,
      file_name: m.file_name,
      file_size: m.file_size,
    }));

    batchTotal = items.length;
    batchDone = 0;
    batchStatus = new Map(
      items.map((item) => [item.message_id, { status: "waiting" as FileStatus, percent: 0 }])
    );

    try {
      const batchId = await invoke<number>("telegram_download_batch", {
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        chatTitle: selectedChat.title,
        items,
        outputDir,
      });
      activeBatchId = batchId;
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "Error";
      showToast("error", msg);
      batchStatus = new Map();
      batchTotal = 0;
    }
  }

  async function cancelBatch() {
    if (!activeBatchId) return;
    try {
      await invoke("telegram_cancel_batch", { batchId: activeBatchId });
      showToast("info", $t("telegram.batch_cancelled"));
    } catch {}
    activeBatchId = null;
  }

  function getItemStatus(messageId: number): FileStatus | null {
    return batchStatus.get(messageId)?.status ?? null;
  }

  function getItemPercent(messageId: number): number {
    return batchStatus.get(messageId)?.percent ?? 0;
  }
</script>

{#if view === "checking"}
  <div class="page-center">
    <span class="spinner"></span>
    <span class="spinner-text">{$t("telegram.checking_session")}</span>
  </div>
{:else if view === "qr"}
  <div class="page-center">
    <div class="login-card">
      <h2>{$t("telegram.title")}</h2>

      {#if qrLoading}
        <div class="qr-placeholder">
          <span class="spinner"></span>
          <span class="spinner-text">{$t("telegram.qr_loading")}</span>
        </div>
      {:else if qrError}
        <div class="qr-placeholder">
          <p class="error-msg">{qrError}</p>
          <button class="button" onclick={startQrLogin}>{$t("common.retry")}</button>
        </div>
      {:else if qrSvg}
        <div class="qr-container">
          {@html qrSvg}
        </div>
      {/if}

      <div class="qr-text">
        <h3>{$t("telegram.qr_title")}</h3>
        <p class="qr-instruction">{$t("telegram.qr_instruction")}</p>
      </div>

      <div class="separator">
        <span class="separator-line"></span>
        <span class="separator-text">{$t("telegram.or_separator")}</span>
        <span class="separator-line"></span>
      </div>

      <button class="button use-phone-btn" onclick={switchToPhone}>
        {$t("telegram.use_phone")}
      </button>
    </div>
  </div>
{:else if view === "phone"}
  <div class="page-center">
    <div class="login-card">
      <h2>{$t("telegram.title")}</h2>
      <form class="form" onsubmit={(e) => { e.preventDefault(); handleSendCode(); }}>
        <label class="field">
          <span class="field-label">{$t("telegram.phone_label")}</span>
          <input
            type="tel"
            placeholder={$t("telegram.phone_placeholder")}
            bind:value={phone}
            class="input"
            disabled={loading}
            required
          />
          <span class="field-hint">{$t("telegram.phone_hint")}</span>
        </label>
        {#if error}
          <p class="error-msg">{error}</p>
        {/if}
        <button type="submit" class="button" disabled={loading || !phone.trim()}>
          {loading ? $t("telegram.sending_code") : $t("telegram.send_code")}
        </button>
      </form>
      <button class="button back-to-qr-btn" onclick={switchToQr}>
        {$t("telegram.back_to_qr")}
      </button>
    </div>
  </div>
{:else if view === "code"}
  <div class="page-center">
    <div class="login-card">
      <h2>{$t("telegram.title")}</h2>
      <form class="form" onsubmit={(e) => { e.preventDefault(); handleVerifyCode(); }}>
        <label class="field">
          <span class="field-label">{$t("telegram.code_label")}</span>
          <input
            type="text"
            inputmode="numeric"
            placeholder={$t("telegram.code_placeholder")}
            bind:value={code}
            class="input"
            disabled={loading}
            required
          />
          <span class="field-hint">{$t("telegram.code_hint")}</span>
        </label>
        {#if error}
          <p class="error-msg">{error}</p>
        {/if}
        <button type="submit" class="button" disabled={loading || !code.trim()}>
          {loading ? $t("telegram.verifying") : $t("telegram.verify")}
        </button>
      </form>
    </div>
  </div>
{:else if view === "password"}
  <div class="page-center">
    <div class="login-card">
      <h2>{$t("telegram.title")}</h2>
      <form class="form" onsubmit={(e) => { e.preventDefault(); handleVerifyPassword(); }}>
        <label class="field">
          <span class="field-label">{$t("telegram.password_label")}</span>
          <input
            type="password"
            placeholder={$t("telegram.password_placeholder")}
            bind:value={password}
            class="input"
            disabled={loading}
            required
          />
          {#if passwordHint}
            <span class="field-hint">{$t("telegram.password_hint", { hint: passwordHint })}</span>
          {/if}
        </label>
        {#if error}
          <p class="error-msg">{error}</p>
        {/if}
        <button type="submit" class="button" disabled={loading || !password}>
          {loading ? $t("telegram.password_verifying") : $t("telegram.password_submit")}
        </button>
      </form>
    </div>
  </div>
{:else if view === "chats"}
  <div class="page-logged">
    <div class="session-bar">
      <span class="session-info">
        {$t("telegram.logged_as", { phone: sessionPhone || "\u2014" })}
      </span>
      <div class="session-actions">
        <button
          class="button"
          onclick={loadChats}
          disabled={loadingChats}
          aria-label={$t("hotmart.refresh")}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class:spinning={loadingChats}>
            <path d="M21 2v6h-6" />
            <path d="M3 12a9 9 0 0115-6.7L21 8" />
            <path d="M3 22v-6h6" />
            <path d="M21 12a9 9 0 01-15 6.7L3 16" />
          </svg>
        </button>
        <button class="button" onclick={handleLogout}>{$t("telegram.logout")}</button>
      </div>
    </div>

    {#if loadingChats}
      <div class="spinner-section">
        <span class="spinner"></span>
        <span class="spinner-text">{$t("telegram.loading_chats")}</span>
      </div>
    {:else if chatsError}
      <div class="error-section">
        <p class="error-msg">{chatsError}</p>
        <button class="button" onclick={loadChats}>{$t("common.retry")}</button>
      </div>
    {:else if chats.length === 0}
      <p class="empty-text">{$t("telegram.no_chats")}</p>
    {:else}
      <div class="chats-header">
        <h2>{$t("telegram.chats_title")}</h2>
        <span class="subtext">
          {chats.length === 1
            ? $t("telegram.chat_count_one", { count: chats.length })
            : $t("telegram.chat_count", { count: chats.length })}
        </span>
      </div>

      <input
        type="text"
        class="input search-input"
        placeholder="Search..."
        bind:value={chatSearch}
      />

      <div class="chats-list">
        {#each filteredChats as chat (chat.id)}
          <button class="chat-item button" onclick={() => selectChat(chat)}>
            <div class="chat-avatar">
              {chat.title.charAt(0).toUpperCase()}
            </div>
            <div class="chat-info">
              <span class="chat-title">{chat.title}</span>
              <span class="chat-type">{chatTypeLabel(chat.chat_type)}</span>
            </div>
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" class="chat-arrow">
              <path d="M9 6l6 6-6 6" />
            </svg>
          </button>
        {/each}
      </div>
    {/if}
  </div>
{:else if view === "media" && selectedChat}
  <div class="page-logged">
    <div class="session-bar">
      <button class="button back-btn" onclick={backToChats}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M15 18l-6-6 6-6" />
        </svg>
        {$t("telegram.back_to_chats")}
      </button>
      <span class="session-info">{selectedChat.title}</span>
    </div>

    <div class="filters">
      {#each [
        { key: "all", label: $t("telegram.filter_all") },
        { key: "photo", label: $t("telegram.filter_photo") },
        { key: "video", label: $t("telegram.filter_video") },
        { key: "document", label: $t("telegram.filter_document") },
        { key: "audio", label: $t("telegram.filter_audio") },
      ] as f}
        <button
          class="button filter-btn"
          class:active={mediaFilter === f.key}
          onclick={() => changeFilter(f.key)}
          disabled={isBatchActive}
        >
          {f.label}
        </button>
      {/each}
    </div>

    {#if loadingMedia}
      <div class="spinner-section">
        <span class="spinner"></span>
        <span class="spinner-text">{$t("telegram.loading_media")}</span>
      </div>
    {:else if mediaError}
      <div class="error-section">
        <p class="error-msg">{mediaError}</p>
        <button class="button" onclick={loadMedia}>{$t("common.retry")}</button>
      </div>
    {:else if mediaItems.length === 0}
      <p class="empty-text">{$t("telegram.no_media")}</p>
    {:else}
      <div class="media-header">
        <span class="subtext">
          {$t("telegram.file_count", { count: mediaItems.length })}
        </span>
        <div class="media-header-actions">
          {#if isBatchActive}
            <button class="button batch-cancel-btn" onclick={cancelBatch}>
              {$t("telegram.cancel_batch")}
            </button>
          {:else}
            <button
              class="button batch-download-btn"
              onclick={downloadAll}
              disabled={mediaItems.length === 0}
            >
              {$t("telegram.download_all")}
            </button>
          {/if}
        </div>
      </div>

      {#if batchTotal > 0}
        <div class="batch-progress-section">
          <div class="batch-progress-bar-outer">
            <div
              class="batch-progress-bar-inner"
              style="width: {batchPercent}%"
            ></div>
          </div>
          <span class="subtext">
            {$t("telegram.batch_progress", { done: batchDone, total: batchTotal })}
          </span>
        </div>
      {/if}

      <div class="media-list">
        {#each mediaItems as item (item.message_id)}
          {@const itemStatus = getItemStatus(item.message_id)}
          {@const itemPercent = getItemPercent(item.message_id)}
          <div class="media-item">
            <div class="media-icon">
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                {#if item.media_type === "photo"}
                  <rect x="3" y="3" width="18" height="18" rx="2" />
                  <circle cx="8.5" cy="8.5" r="1.5" />
                  <path d="M21 15l-5-5L5 21" />
                {:else if item.media_type === "video"}
                  <rect x="2" y="5" width="20" height="14" rx="2" />
                  <path d="M10 9l5 3-5 3z" fill="currentColor" stroke="none" />
                {:else if item.media_type === "audio"}
                  <path d="M9 18V5l12-2v13" />
                  <circle cx="6" cy="18" r="3" />
                  <circle cx="18" cy="16" r="3" />
                {:else}
                  <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
                  <path d="M14 2v6h6" />
                {/if}
              </svg>
            </div>
            <div class="media-info">
              <span class="media-name">{item.file_name}</span>
              <span class="media-meta">
                {formatSize(item.file_size)} &middot; {formatDate(item.date)}
                {#if itemStatus === "downloading"}
                  &middot; {Math.round(itemPercent)}%
                {:else if itemStatus === "done"}
                  &middot; {$t("telegram.downloaded")}
                {:else if itemStatus === "skipped"}
                  &middot; {$t("telegram.status_skipped")}
                {:else if itemStatus === "error"}
                  &middot; {$t("telegram.status_error")}
                {:else if itemStatus === "waiting"}
                  &middot; {$t("telegram.status_waiting")}
                {/if}
              </span>
            </div>
            {#if itemStatus === "downloading"}
              <span class="media-status-icon downloading">
                <span class="spinner small"></span>
              </span>
            {:else if itemStatus === "done"}
              <span class="media-status-icon done">
                <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="var(--green)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M20 6L9 17l-5-5" />
                </svg>
              </span>
            {:else if itemStatus === "skipped"}
              <span class="media-status-icon skipped">
                <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="var(--gray)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M13 17l5-5-5-5" />
                  <path d="M6 17l5-5-5-5" />
                </svg>
              </span>
            {:else if itemStatus === "error"}
              <span class="media-status-icon error-icon">
                <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="var(--red)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M18 6L6 18" />
                  <path d="M6 6l12 12" />
                </svg>
              </span>
            {:else if itemStatus === "waiting"}
              <span class="media-status-icon waiting">
                <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="var(--gray)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="10" />
                  <path d="M12 6v6l4 2" />
                </svg>
              </span>
            {:else}
              <button
                class="button media-download-btn"
                disabled={downloadingIds.has(item.message_id) || isBatchActive}
                onclick={() => downloadItem(item)}
              >
                {#if downloadingIds.has(item.message_id)}
                  {$t("telegram.downloading")}
                {:else}
                  {$t("telegram.download_btn")}
                {/if}
              </button>
            {/if}
          </div>
        {/each}
      </div>

      {#if hasMore}
        <button
          class="button load-more-btn"
          onclick={loadMoreMedia}
          disabled={loadingMore || isBatchActive}
        >
          {#if loadingMore}
            <span class="spinner small"></span>
          {:else}
            {$t("telegram.load_more")}
          {/if}
        </button>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .page-center {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4);
    gap: var(--padding);
  }

  .page-logged {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--padding) * 1.5);
    padding: calc(var(--padding) * 1.5);
    width: 100%;
  }

  .page-logged > :global(*) {
    width: 100%;
    max-width: 800px;
  }

  .session-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .session-info {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .session-actions {
    display: flex;
    gap: calc(var(--padding) / 2);
  }

  .session-bar :global(.button) {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
  }

  .spinning {
    animation: spin 0.6s linear infinite;
  }

  .back-btn {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }

  .login-card {
    width: 100%;
    max-width: 400px;
    background: var(--button-elevated);
    border-radius: var(--border-radius);
    padding: calc(var(--padding) * 2);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
  }

  .login-card h2 {
    margin-block: 0;
  }

  .qr-container {
    display: flex;
    justify-content: center;
    align-items: center;
    background: #ffffff;
    border-radius: var(--border-radius);
    padding: var(--padding);
  }

  .qr-container :global(svg) {
    width: 200px;
    height: 200px;
    display: block;
  }

  .qr-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--padding);
    min-height: 200px;
  }

  .qr-text {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
    text-align: center;
  }

  .qr-text h3 {
    margin-block: 0;
  }

  .qr-instruction {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    line-height: 1.6;
  }

  .separator {
    display: flex;
    align-items: center;
    gap: var(--padding);
  }

  .separator-line {
    flex: 1;
    height: 1px;
    background: var(--input-border);
  }

  .separator-text {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .use-phone-btn,
  .back-to-qr-btn {
    width: 100%;
    text-align: center;
    justify-content: center;
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
  }

  .field-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .field-hint {
    font-size: 11px;
    font-weight: 500;
    color: var(--gray);
    opacity: 0.7;
  }

  .input {
    width: 100%;
    padding: var(--padding);
    font-size: 14.5px;
    background: var(--button);
    border-radius: var(--border-radius);
    color: var(--secondary);
    border: 1px solid var(--input-border);
  }

  .input::placeholder {
    color: var(--gray);
  }

  .input:focus-visible {
    border-color: var(--secondary);
    outline: none;
  }

  .input:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .search-input {
    max-width: 800px;
  }

  .error-msg {
    color: var(--red);
    font-size: 12.5px;
    font-weight: 500;
  }

  .spinner-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    padding: calc(var(--padding) * 4) 0;
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  .spinner.small {
    width: 14px;
    height: 14px;
    border-width: 1.5px;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .spinner-text {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .error-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    padding: calc(var(--padding) * 2) 0;
  }

  .empty-text {
    color: var(--gray);
    font-size: 14.5px;
    text-align: center;
    padding: calc(var(--padding) * 4) 0;
  }

  .chats-header {
    display: flex;
    align-items: baseline;
    gap: var(--padding);
  }

  .chats-header h2 {
    margin-block: 0;
  }

  .subtext {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .chats-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .chat-item {
    display: flex;
    align-items: center;
    gap: var(--padding);
    padding: var(--padding);
    text-align: left;
    width: 100%;
  }

  .chat-avatar {
    width: 36px;
    height: 36px;
    min-width: 36px;
    border-radius: 50%;
    background: var(--blue);
    color: #fff;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 14.5px;
    font-weight: 500;
  }

  .chat-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .chat-title {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chat-type {
    font-size: 11px;
    font-weight: 500;
    color: var(--gray);
  }

  .chat-arrow {
    color: var(--gray);
    flex-shrink: 0;
  }

  .filters {
    display: flex;
    gap: calc(var(--padding) / 2);
    flex-wrap: wrap;
  }

  .filter-btn {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
  }

  .media-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .media-header-actions {
    display: flex;
    gap: calc(var(--padding) / 2);
  }

  .batch-download-btn,
  .batch-cancel-btn {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
  }

  .batch-cancel-btn {
    color: var(--red);
  }

  .batch-progress-section {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
  }

  .batch-progress-bar-outer {
    width: 100%;
    height: 6px;
    background: var(--button-elevated);
    border-radius: 3px;
    overflow: hidden;
  }

  .batch-progress-bar-inner {
    height: 100%;
    background: var(--blue);
    border-radius: 3px;
    transition: width 0.1s;
  }

  .media-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .media-item {
    display: flex;
    align-items: center;
    gap: var(--padding);
    padding: var(--padding);
    background: var(--button);
    border-radius: var(--border-radius);
  }

  .media-icon {
    width: 36px;
    height: 36px;
    min-width: 36px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--gray);
  }

  .media-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .media-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .media-meta {
    font-size: 11px;
    font-weight: 500;
    color: var(--gray);
  }

  .media-download-btn {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
    flex-shrink: 0;
  }

  .media-download-btn:disabled {
    opacity: 0.6;
  }

  .media-status-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    flex-shrink: 0;
  }

  .load-more-btn {
    align-self: center;
    padding: calc(var(--padding) / 2) calc(var(--padding) * 2);
    font-size: 12.5px;
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }
</style>
