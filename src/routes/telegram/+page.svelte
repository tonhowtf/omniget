<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { onBatchFileStatus, type BatchFileStatusPayload } from "$lib/stores/download-listener";
  import ContextHint from "$components/hints/ContextHint.svelte";
  import TelegramChannelDrawer from "$lib/study-components/TelegramChannelDrawer.svelte";
  import TelegramPerfPanel from "$lib/study-components/TelegramPerfPanel.svelte";
  import TelegramGlobalSearchModal from "$lib/study-components/TelegramGlobalSearchModal.svelte";
  import TelegramTransferPanel from "$lib/study-components/TelegramTransferPanel.svelte";
  import TelegramCloneWizard from "$lib/study-components/TelegramCloneWizard.svelte";
  import TelegramAccountPanel from "$lib/study-components/TelegramAccountPanel.svelte";
  import TelegramSyncIndicator from "$lib/study-components/TelegramSyncIndicator.svelte";
  import { telegramCreateFolder, isOmnigetFolder, type TelegramGlobalSearchHit } from "$lib/study-telegram-bridge";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";

  type PluginStatus =
    | "checking_plugin"
    | "ready"
    | "not-installed"
    | "needs-restart"
    | "incompatible"
    | "load-failed";
  type PluginLoadError = {
    message: string;
    kind: string;
    plugin_abi?: number | null;
    expected_abi?: number | null;
  };
  let pluginStatus = $state<PluginStatus>("checking_plugin");
  let loadError = $state<PluginLoadError | null>(null);

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
  let mediaSearch = $state("");
  let searchDebounce: ReturnType<typeof setTimeout> | null = null;
  let isSearching = $state(false);
  let searchInputRef: HTMLInputElement | null = $state(null);

  let batchStatus: Map<number, { status: FileStatus; percent: number }> = $state(new Map());
  let activeBatchId: number | null = $state(null);
  let batchDone = $state(0);
  let batchTotal = $state(0);

  let isBatchActive = $derived(activeBatchId !== null);
  let batchPercent = $derived(batchTotal > 0 ? (batchDone / batchTotal) * 100 : 0);

  let downloadingIds: Set<number> = $state(new Set());
  let downloadIdToMessageId: Map<number, number> = $state(new Map());
  let downloadProgress: Map<number, number> = $state(new Map());
  let downloadUnlisteners: UnlistenFn[] = [];

  let chatPhotos: Map<number, string> = $state(new Map());
  let thumbnails: Map<number, string> = $state(new Map());

  let drawerOpen = $state(false);
  let drawerChat: TelegramChat | null = $state(null);
  let perfPanelOpen = $state(false);

  type ChatViewMode = "all" | "drive";
  let chatViewMode = $state<ChatViewMode>("all");
  let createFolderOpen = $state(false);
  let createFolderName = $state("");
  let createFolderBusy = $state(false);
  let createFolderError = $state("");
  let globalSearchOpen = $state(false);

  type TransferRecord = {
    id: number;
    fileName: string;
    sizeBytes?: number;
    percent: number;
    status: "downloading" | "done" | "error";
    error?: string;
    completedAt?: number;
    path?: string;
  };
  const TRANSFER_HISTORY_KEY = "tg-transfer-history";
  let transferHistory = $state<TransferRecord[]>(loadHistoryFromStorage());
  let transferPanelOpen = $state(false);

  function loadHistoryFromStorage(): TransferRecord[] {
    if (typeof localStorage === "undefined") return [];
    try {
      const raw = localStorage.getItem(TRANSFER_HISTORY_KEY);
      if (!raw) return [];
      const arr = JSON.parse(raw);
      if (!Array.isArray(arr)) return [];
      return arr.slice(0, 50);
    } catch {
      return [];
    }
  }

  function saveHistoryToStorage(list: TransferRecord[]) {
    if (typeof localStorage === "undefined") return;
    try {
      localStorage.setItem(TRANSFER_HISTORY_KEY, JSON.stringify(list.slice(0, 50)));
    } catch {
      /* noop */
    }
  }

  function pushHistory(rec: TransferRecord) {
    transferHistory = [rec, ...transferHistory].slice(0, 50);
    saveHistoryToStorage(transferHistory);
  }

  function clearTransferHistory() {
    transferHistory = [];
    saveHistoryToStorage(transferHistory);
  }

  let activeTransfers = $derived<TransferRecord[]>(
    [...downloadingIds].map((mid) => {
      const item = mediaItems.find((m) => m.message_id === mid);
      return {
        id: mid,
        fileName: item?.file_name ?? `Mensagem ${mid}`,
        sizeBytes: item?.file_size,
        percent: downloadProgress.get(mid) ?? 0,
        status: "downloading" as const,
      };
    })
  );

  let activeTransferCount = $derived(activeTransfers.length);
  let cloneWizardOpen = $state(false);
  let accountPanelOpen = $state(false);

  function onGlobalSearchPick(hit: TelegramGlobalSearchHit) {
    const existing = chats.find((c) => c.id === hit.chat_id);
    if (existing) {
      selectChat(existing);
    } else {
      selectChat({
        id: hit.chat_id,
        title: hit.chat_title,
        chat_type: hit.chat_type,
      } as TelegramChat);
    }
  }

  function openDrawer(chat: TelegramChat, e: Event) {
    e.stopPropagation();
    drawerChat = chat;
    drawerOpen = true;
  }

  function onChatRemoved(removedId: number) {
    chats = chats.filter((c) => c.id !== removedId);
    if (selectedChat?.id === removedId) {
      backToChats();
    }
  }

  let thumbGeneration = 0;
  let thumbActive = 0;
  const THUMB_MAX_CONCURRENT = 5;
  const thumbQueue: (() => void)[] = [];

  let filteredChats = $derived(
    (() => {
      let base = chats;
      if (chatViewMode === "drive") {
        base = base.filter((c) => isOmnigetFolder(c as any));
      }
      const q = chatSearch.trim().toLowerCase();
      if (q) base = base.filter((c) => c.title.toLowerCase().includes(q));
      return base;
    })()
  );

  async function commitCreateFolder() {
    const name = createFolderName.trim();
    if (!name || createFolderBusy) return;
    createFolderBusy = true;
    createFolderError = "";
    try {
      const folder = await telegramCreateFolder(name);
      chats = [folder as any, ...chats];
      chatViewMode = "drive";
      createFolderOpen = false;
      createFolderName = "";
    } catch (e: any) {
      createFolderError = typeof e === "string" ? e : (e?.message ?? "Erro ao criar pasta");
    } finally {
      createFolderBusy = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === "f" && view === "media" && searchInputRef) {
      e.preventDefault();
      searchInputRef.focus();
    }
    if ((e.ctrlKey || e.metaKey) && e.key === "k") {
      e.preventDefault();
      globalSearchOpen = true;
    }
  }

  async function checkPluginStatus() {
    try {
      const plugins = await invoke<{
        id: string;
        enabled: boolean;
        loaded: boolean;
        load_error?: PluginLoadError | null;
      }[]>("list_plugins");
      const plugin = plugins.find((p) => p.id === "telegram");
      if (!plugin || !plugin.enabled) {
        pluginStatus = "not-installed";
        return;
      }
      if (!plugin.loaded) {
        if (plugin.load_error) {
          loadError = plugin.load_error;
          pluginStatus =
            plugin.load_error.kind === "abi_mismatch" ||
            plugin.load_error.kind === "missing_abi_symbol"
              ? "incompatible"
              : "load-failed";
        } else {
          pluginStatus = "needs-restart";
        }
        return;
      }
      loadError = null;
      pluginStatus = "ready";
    } catch {
      pluginStatus = "ready";
    }
  }

  onMount(() => {
    let pluginsUnlisten: UnlistenFn | null = null;
    let destroyed = false;

    checkPluginStatus();
    listen("plugins-changed", () => {
      checkPluginStatus();
    }).then((unlisten) => {
      if (destroyed) {
        unlisten();
      } else {
        pluginsUnlisten = unlisten;
      }
    });

    return () => {
      destroyed = true;
      pluginsUnlisten?.();
      pluginsUnlisten = null;
    };
  });

  $effect(() => {
    if (pluginStatus !== "ready") return;

    checkSession();
    initDownloadListeners();

    onBatchFileStatus(handleBatchFileStatus);
    document.addEventListener("keydown", handleKeydown);

    return () => {
      stopQrPolling();
      onBatchFileStatus(null);
      resetThumbnails();
      document.removeEventListener("keydown", handleKeydown);
      if (searchDebounce) clearTimeout(searchDebounce);
      pluginInvoke("telegram", "telegram_clear_thumbnail_cache").catch(() => {});
      for (const unlisten of downloadUnlisteners) unlisten();
      downloadUnlisteners = [];
    };
  });

  async function initDownloadListeners() {
    type GenericProgress = { id: number; title: string; platform: string; percent: number };
    type GenericComplete = {
      id: number; title: string; platform: string; success: boolean;
      error: string | null; file_path: string | null;
      file_size_bytes: number | null; file_count: number | null;
    };

    const unlistenProgress = await listen<GenericProgress>("generic-download-progress", (event) => {
      const d = event.payload;
      if (d.platform !== "telegram") return;
      const msgId = downloadIdToMessageId.get(d.id);
      if (msgId === undefined) return;
      downloadProgress.set(msgId, d.percent);
      downloadProgress = new Map(downloadProgress);
    });

    const unlistenComplete = await listen<GenericComplete>("generic-download-complete", (event) => {
      const d = event.payload;
      if (d.platform !== "telegram") return;
      const msgId = downloadIdToMessageId.get(d.id);
      if (msgId === undefined) return;

      downloadingIds = new Set([...downloadingIds].filter((id) => id !== msgId));
      downloadIdToMessageId.delete(d.id);
      downloadIdToMessageId = new Map(downloadIdToMessageId);
      downloadProgress.delete(msgId);
      downloadProgress = new Map(downloadProgress);

      pushHistory({
        id: msgId,
        fileName: d.title,
        sizeBytes: d.file_size_bytes ?? undefined,
        percent: 100,
        status: d.success ? "done" : "error",
        error: d.error ?? undefined,
        completedAt: Date.now(),
        path: d.file_path ?? undefined,
      });

      if (d.success) {
        showToast("success", $t("toast.download_complete", { name: d.title }));
      } else {
        showToast("error", d.error ?? $t("common.error"));
      }
    });

    downloadUnlisteners = [unlistenProgress, unlistenComplete];
  }

  function handleBatchFileStatus(payload: BatchFileStatusPayload) {
    if (payload.batch_id !== activeBatchId) return;

    batchStatus.set(payload.message_id, {
      status: payload.status,
      percent: payload.percent,
    });
    batchStatus = new Map(batchStatus);

    let done = 0;
    for (const [, entry] of batchStatus) {
      if (entry.status === "done" || entry.status === "error" || entry.status === "skipped") {
        done++;
      }
    }
    batchDone = done;

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
      const result = await pluginInvoke<string>("telegram", "telegram_check_session");
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
      const result = await pluginInvoke<QrStartResponse>("telegram", "telegram_qr_start");
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
      const status = await pluginInvoke<string>("telegram", "telegram_qr_poll");
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
      await pluginInvoke("telegram", "telegram_send_code", { phone: phone.trim() });
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
      const result = await pluginInvoke<string>("telegram", "telegram_verify_code", { code: code.trim() });
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
      const result = await pluginInvoke<string>("telegram", "telegram_verify_2fa", { password });
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
      await pluginInvoke("telegram", "telegram_logout");
    } catch {}
    sessionPhone = "";
    chats = [];
    mediaItems = [];
    selectedChat = null;
    chatPhotos = new Map();
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
      chats = await pluginInvoke<TelegramChat[]>("telegram", "telegram_list_chats");
    } catch (e: any) {
      chatsError = typeof e === "string" ? e : e.message ?? $t("telegram.chats_error");
    } finally {
      loadingChats = false;
    }
  }

  async function selectChat(chat: TelegramChat) {
    selectedChat = chat;
    mediaFilter = "all";
    mediaSearch = "";
    view = "media";
    batchStatus = new Map();
    activeBatchId = null;
    batchDone = 0;
    batchTotal = 0;
    downloadingIds = new Set();
    resetThumbnails();
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
    resetThumbnails();
    view = "chats";
  }

  async function loadMedia() {
    if (!selectedChat) return;
    loadingMedia = true;
    mediaError = "";
    try {
      const items: TelegramMediaItem[] = await pluginInvoke<TelegramMediaItem[]>("telegram", "telegram_list_media", {
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
      const offset = mediaItems.length > 0
        ? Math.min(...mediaItems.map((m) => m.message_id))
        : 0;
      const items: TelegramMediaItem[] = await pluginInvoke<TelegramMediaItem[]>("telegram", "telegram_list_media", {
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        mediaType: mediaFilter === "all" ? null : mediaFilter,
        offset,
        limit: 100,
      });
      if (items.length > 0) {
        const existingIds = new Set(mediaItems.map((m) => m.message_id));
        const newItems = items.filter((item) => !existingIds.has(item.message_id));
        mediaItems = [...mediaItems, ...newItems];
      }
      hasMore = items.length >= 100;
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    } finally {
      loadingMore = false;
    }
  }

  async function searchMedia() {
    if (!selectedChat) return;
    const query = mediaSearch.trim();
    if (!query) {
      loadMedia();
      return;
    }
    isSearching = true;
    loadingMedia = true;
    mediaError = "";
    hasMore = false;
    try {
      const items: TelegramMediaItem[] = await pluginInvoke<TelegramMediaItem[]>("telegram", "telegram_search_media", {
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        query,
        mediaType: mediaFilter === "all" ? null : mediaFilter,
        limit: 100,
      });
      mediaItems = items;
    } catch (e: any) {
      mediaError = typeof e === "string" ? e : e.message ?? $t("telegram.media_error");
    } finally {
      loadingMedia = false;
      isSearching = false;
    }
  }

  function handleSearchInput() {
    if (searchDebounce) clearTimeout(searchDebounce);
    searchDebounce = setTimeout(() => {
      if (mediaSearch.trim()) {
        searchMedia();
      } else {
        hasMore = true;
        loadMedia();
      }
    }, 400);
  }

  function changeFilter(filter: string) {
    mediaFilter = filter;
    batchStatus = new Map();
    activeBatchId = null;
    batchDone = 0;
    batchTotal = 0;
    hasMore = true;
    if (mediaSearch.trim()) {
      searchMedia();
    } else {
      loadMedia();
    }
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
      const result = await pluginInvoke<{ id: number; file_name: string }>("telegram", "telegram_download_media", {
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        messageId: item.message_id,
        fileName: item.file_name,
        outputDir,
      });
      downloadIdToMessageId.set(result.id, item.message_id);
      downloadIdToMessageId = new Map(downloadIdToMessageId);
      showToast("info", $t("toast.download_started", { name: item.file_name }));
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
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
      const batchId = await pluginInvoke<number>("telegram", "telegram_download_batch", {
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        chatTitle: selectedChat.title,
        items,
        outputDir,
      });
      activeBatchId = batchId;
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
      batchStatus = new Map();
      batchTotal = 0;
    }
  }

  async function cancelBatch() {
    if (!activeBatchId) return;
    try {
      await pluginInvoke("telegram", "telegram_cancel_batch", { batchId: activeBatchId });
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

  function thumbAcquire(): Promise<void> {
    if (thumbActive < THUMB_MAX_CONCURRENT) {
      thumbActive++;
      return Promise.resolve();
    }
    return new Promise((resolve) => {
      thumbQueue.push(() => {
        thumbActive++;
        resolve();
      });
    });
  }

  function thumbRelease() {
    thumbActive--;
    if (thumbQueue.length > 0) {
      thumbQueue.shift()!();
    }
  }

  let pendingThumbs: Array<[number, string]> = [];
  let pendingThumbFlushScheduled = false;

  function flushPendingThumbs() {
    pendingThumbFlushScheduled = false;
    if (pendingThumbs.length === 0) return;
    const next = new Map(thumbnails);
    for (const [id, t] of pendingThumbs) {
      next.set(id, t);
    }
    pendingThumbs = [];
    thumbnails = next;
  }

  function scheduleThumbFlush() {
    if (pendingThumbFlushScheduled) return;
    pendingThumbFlushScheduled = true;
    requestAnimationFrame(flushPendingThumbs);
  }

  async function getThumbnail(chatId: number, chatType: string, messageId: number): Promise<string | null> {
    if (thumbnails.has(messageId)) return thumbnails.get(messageId)!;

    const gen = thumbGeneration;
    await thumbAcquire();
    try {
      if (gen !== thumbGeneration) return null;
      if (thumbnails.has(messageId)) return thumbnails.get(messageId)!;

      const result = await pluginInvoke<string>("telegram", "telegram_get_thumbnail", { chatId, chatType, messageId });
      if (gen !== thumbGeneration) return null;

      pendingThumbs.push([messageId, result]);
      scheduleThumbFlush();
      return result;
    } catch {
      return null;
    } finally {
      thumbRelease();
    }
  }

  function resetThumbnails() {
    thumbnails = new Map();
    thumbGeneration++;
    thumbQueue.length = 0;
  }

  const CHAT_PHOTO_MAX_CONCURRENT = 4;
  let chatPhotoActive = 0;
  const chatPhotoQueue: (() => void)[] = [];
  let pendingPhotos: Array<[number, string]> = [];
  let pendingPhotoFlushScheduled = false;

  function chatPhotoAcquire(): Promise<void> {
    if (chatPhotoActive < CHAT_PHOTO_MAX_CONCURRENT) {
      chatPhotoActive++;
      return Promise.resolve();
    }
    return new Promise((resolve) => {
      chatPhotoQueue.push(() => {
        chatPhotoActive++;
        resolve();
      });
    });
  }

  function chatPhotoRelease() {
    chatPhotoActive--;
    if (chatPhotoQueue.length > 0) {
      chatPhotoQueue.shift()!();
    }
  }

  function flushPendingPhotos() {
    pendingPhotoFlushScheduled = false;
    if (pendingPhotos.length === 0) return;
    const next = new Map(chatPhotos);
    for (const [id, photo] of pendingPhotos) {
      next.set(id, photo);
    }
    pendingPhotos = [];
    chatPhotos = next;
  }

  function schedulePhotoFlush() {
    if (pendingPhotoFlushScheduled) return;
    pendingPhotoFlushScheduled = true;
    requestAnimationFrame(flushPendingPhotos);
  }

  async function getChatPhoto(chatId: number, chatType: string) {
    if (chatPhotos.has(chatId)) return;
    await chatPhotoAcquire();
    try {
      if (chatPhotos.has(chatId)) return;
      const result = await pluginInvoke<string>("telegram", "telegram_get_chat_photo", { chatId, chatType });
      pendingPhotos.push([chatId, result]);
      schedulePhotoFlush();
    } catch {
    } finally {
      chatPhotoRelease();
    }
  }

  function observeChatPhoto(node: HTMLElement, params: { chatId: number; chatType: string }) {
    if (chatPhotos.has(params.chatId)) return;

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting) {
          observer.disconnect();
          getChatPhoto(params.chatId, params.chatType);
        }
      },
      { rootMargin: "200px" }
    );
    observer.observe(node);

    return {
      destroy() {
        observer.disconnect();
      },
    };
  }

  function observeThumbnail(node: HTMLElement, params: { messageId: number; mediaType: string }) {
    if (!selectedChat) return;
    if (params.mediaType !== "photo" && params.mediaType !== "video") return;
    if (thumbnails.has(params.messageId)) return;

    const chatId = selectedChat.id;
    const chatType = selectedChat.chat_type;

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting) {
          observer.disconnect();
          getThumbnail(chatId, chatType, params.messageId);
        }
      },
      { rootMargin: "200px" }
    );
    observer.observe(node);

    return {
      destroy() {
        observer.disconnect();
      },
    };
  }
</script>

{#if pluginStatus === "checking_plugin"}
  <div class="plugin-guard"><span class="spinner"></span></div>
{:else if pluginStatus === "not-installed"}
  <div class="plugin-guard">
    <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M21 5L2 12.5l7 1M21 5l-5.5 15-4.5-7.5M21 5L9 13.5" />
    </svg>
    <h2>{$t("marketplace.plugin_not_installed")}</h2>
    <p>{$t("marketplace.plugin_install_hint")}</p>
    <a href="/marketplace" class="guard-link">{$t("marketplace.go_to_marketplace")}</a>
  </div>
{:else if pluginStatus === "needs-restart"}
  <div class="plugin-guard">
    <h2>{$t("marketplace.restart_required")}</h2>
    <p>{$t("marketplace.plugin_restart_hint")}</p>
  </div>
{:else if pluginStatus === "incompatible"}
  <div class="plugin-guard">
    <h2>{$t("marketplace.plugin_incompatible_title")}</h2>
    <p>{$t("marketplace.plugin_incompatible_hint")}</p>
    <a href="/marketplace" class="guard-link">{$t("marketplace.go_to_marketplace")}</a>
    {#if loadError}
      <p class="guard-detail"><code>{loadError.message}</code></p>
    {/if}
  </div>
{:else if pluginStatus === "load-failed"}
  <div class="plugin-guard">
    <h2>{$t("marketplace.plugin_load_failed_title")}</h2>
    <p>{$t("marketplace.plugin_load_failed_hint")}</p>
    <a href="/marketplace" class="guard-link">{$t("marketplace.go_to_marketplace")}</a>
    {#if loadError}
      <p class="guard-detail"><code>{loadError.message}</code></p>
    {/if}
  </div>
{:else}
{#if view === "checking"}
  <div class="page-center">
    <span class="spinner"></span>
    <span class="spinner-text">{$t("telegram.checking_session")}</span>
  </div>
{:else if view === "qr"}
  <div class="page-center">
    <div class="login-card">
      <h2>{$t("telegram.title")} <ContextHint text={$t('hints.telegram')} dismissKey="telegram" /></h2>

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

      <div class="or-divider">
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
        <TelegramSyncIndicator />
        <button
          class="button account-btn"
          onclick={() => (accountPanelOpen = true)}
          aria-label="Gerenciar contas"
          title="Gerenciar contas"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4 4v2" />
            <circle cx="12" cy="7" r="4" />
          </svg>
        </button>
        <button
          class="button"
          onclick={() => (cloneWizardOpen = true)}
          aria-label="Clonar canais"
          title="Clonar canais"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="9" width="13" height="13" rx="2" />
            <path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1" />
          </svg>
        </button>
        <button
          class="button transfers-btn"
          onclick={() => (transferPanelOpen = true)}
          aria-label="Transferências"
          title="Transferências"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 5v14M19 12l-7 7-7-7" />
          </svg>
          {#if activeTransferCount > 0}
            <span class="transfers-badge">{activeTransferCount}</span>
          {/if}
        </button>
        <button
          class="button"
          onclick={() => (globalSearchOpen = true)}
          aria-label="Busca global"
          title="Busca global (Ctrl+K)"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
        </button>
        <button
          class="button"
          onclick={() => (perfPanelOpen = true)}
          aria-label="Performance"
          title="Performance de download"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 11-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 11-4 0v-.09a1.65 1.65 0 00-1-1.51 1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 11-2.83-2.83l.06-.06a1.65 1.65 0 00.33-1.82 1.65 1.65 0 00-1.51-1H3a2 2 0 110-4h.09a1.65 1.65 0 001.51-1 1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 112.83-2.83l.06.06a1.65 1.65 0 001.82.33h0a1.65 1.65 0 001-1.51V3a2 2 0 114 0v.09a1.65 1.65 0 001 1.51h0a1.65 1.65 0 001.82-.33l.06-.06a2 2 0 112.83 2.83l-.06.06a1.65 1.65 0 00-.33 1.82v0a1.65 1.65 0 001.51 1H21a2 2 0 110 4h-.09a1.65 1.65 0 00-1.51 1z" />
          </svg>
        </button>
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

      <div class="view-mode-row">
        <div class="view-tabs" role="tablist">
          <button
            type="button"
            class="view-tab"
            class:active={chatViewMode === "all"}
            role="tab"
            aria-selected={chatViewMode === "all"}
            onclick={() => (chatViewMode = "all")}
          >
            Todos
          </button>
          <button
            type="button"
            class="view-tab"
            class:active={chatViewMode === "drive"}
            role="tab"
            aria-selected={chatViewMode === "drive"}
            onclick={() => (chatViewMode = "drive")}
          >
            Drive
            <span class="view-tab-count">{chats.filter((c) => isOmnigetFolder(c as any)).length}</span>
          </button>
        </div>
        {#if chatViewMode === "drive"}
          <button type="button" class="button create-folder-btn" onclick={() => (createFolderOpen = true)}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 5v14M5 12h14" />
            </svg>
            Nova pasta
          </button>
        {/if}
      </div>

      <input
        type="text"
        class="input search-input"
        placeholder={chatViewMode === "drive" ? "Buscar pasta..." : "Search..."}
        bind:value={chatSearch}
      />

      <div class="chats-list">
        {#each filteredChats as chat (chat.id)}
          <div class="chat-row">
            <button class="chat-item button" onclick={() => selectChat(chat)}>
              <div class="chat-avatar" class:has-photo={chatPhotos.get(chat.id)} use:observeChatPhoto={{ chatId: chat.id, chatType: chat.chat_type }}>
                {#if chatPhotos.get(chat.id)}
                  <img
                    src="data:image/jpeg;base64,{chatPhotos.get(chat.id)}"
                    alt=""
                    class="chat-photo-img"
                  />
                {:else}
                  {chat.title.charAt(0).toUpperCase()}
                {/if}
              </div>
              <div class="chat-info">
                <span class="chat-title">{chat.title}</span>
                <span class="chat-type">{chatTypeLabel(chat.chat_type)}</span>
              </div>
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" class="chat-arrow">
                <path d="M9 6l6 6-6 6" />
              </svg>
            </button>
            <button
              type="button"
              class="chat-info-btn"
              onclick={(e) => openDrawer(chat, e)}
              aria-label="Gerenciar {chat.title}"
              title="Gerenciar"
            >
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="5" r="1.5" />
                <circle cx="12" cy="12" r="1.5" />
                <circle cx="12" cy="19" r="1.5" />
              </svg>
            </button>
          </div>
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
      <button
        type="button"
        class="button manage-btn"
        onclick={(e) => openDrawer(selectedChat!, e)}
        aria-label="Gerenciar canal"
      >
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="5" r="1.5" />
          <circle cx="12" cy="12" r="1.5" />
          <circle cx="12" cy="19" r="1.5" />
        </svg>
        Gerenciar
      </button>
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

    <input
      type="text"
      class="input search-input"
      placeholder={$t("telegram.search_files")}
      bind:value={mediaSearch}
      bind:this={searchInputRef}
      oninput={handleSearchInput}
      disabled={isBatchActive}
    />

    {#if loadingMedia}
      <div class="spinner-section">
        <span class="spinner"></span>
        <span class="spinner-text">{isSearching ? $t("telegram.searching") : $t("telegram.loading_media")}</span>
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
              style:width="{batchPercent}%"
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
          <div class="media-item" use:observeThumbnail={{ messageId: item.message_id, mediaType: item.media_type }}>
            <div class="media-icon" class:has-thumb={(item.media_type === "photo" || item.media_type === "video") && thumbnails.get(item.message_id)}>
              {#if (item.media_type === "photo" || item.media_type === "video") && thumbnails.get(item.message_id)}
                <img
                  src="data:image/jpeg;base64,{thumbnails.get(item.message_id)}"
                  alt=""
                  class="thumb-img"
                />
              {:else}
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
              {/if}
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
                  {@const pct = downloadProgress.get(item.message_id) ?? 0}
                  {pct > 0 ? `${Math.round(pct)}%` : $t("telegram.downloading")}
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
{/if}

<TelegramChannelDrawer
  bind:open={drawerOpen}
  chat={drawerChat}
  onChatRemoved={onChatRemoved}
/>

<TelegramPerfPanel bind:open={perfPanelOpen} />

<TelegramGlobalSearchModal
  bind:open={globalSearchOpen}
  onPickResult={onGlobalSearchPick}
/>

<TelegramTransferPanel
  bind:open={transferPanelOpen}
  active={activeTransfers}
  history={transferHistory}
  onClearHistory={clearTransferHistory}
/>

<TelegramCloneWizard bind:open={cloneWizardOpen} chats={chats} />

<TelegramAccountPanel bind:open={accountPanelOpen} sessionPhone={sessionPhone} />

{#if createFolderOpen}
  <div
    class="create-folder-overlay"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget && !createFolderBusy) createFolderOpen = false; }}
    onkeydown={(e) => { if (e.key === "Escape" && !createFolderBusy) createFolderOpen = false; }}
  >
    <div class="create-folder-dialog" role="dialog" aria-modal="true">
      <h3>Nova pasta Drive</h3>
      <p class="dialog-hint">Cria um canal Telegram com sufixo <code>[og]</code> para você usar como pasta privada de mídias.</p>
      <form onsubmit={(e) => { e.preventDefault(); commitCreateFolder(); }}>
        <input
          type="text"
          class="input"
          placeholder="Nome da pasta"
          bind:value={createFolderName}
          disabled={createFolderBusy}
          autofocus
          required
        />
        {#if createFolderError}
          <p class="dialog-error">{createFolderError}</p>
        {/if}
        <div class="dialog-actions">
          <button type="button" class="button" onclick={() => (createFolderOpen = false)} disabled={createFolderBusy}>Cancelar</button>
          <button type="submit" class="button primary" disabled={createFolderBusy || !createFolderName.trim()}>
            {createFolderBusy ? "Criando..." : "Criar"}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .plugin-guard { display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: calc(100vh - var(--padding) * 4); gap: calc(var(--padding) * 1.5); text-align: center; color: var(--gray); }
  .plugin-guard h2 { font-size: 18px; color: var(--secondary); }
  .plugin-guard p { font-size: 14px; max-width: 300px; }
  .guard-link { padding: 10px 24px; font-size: 14px; font-weight: 500; background: var(--cta); color: var(--on-cta); border-radius: var(--border-radius); text-decoration: none; }
  .guard-detail { font-size: 12px; color: var(--tertiary); max-width: 480px; margin-top: calc(var(--padding) * 0.5); }
  .guard-detail code { font-family: var(--font-mono, ui-monospace, monospace); font-size: 11px; word-break: break-word; background: var(--surface); padding: 2px 6px; border-radius: 4px; }

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
    flex-wrap: wrap;
    gap: calc(var(--padding) / 2);
  }

  .session-info {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--gray);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1 1 auto;
    min-width: 0;
  }

  .session-actions {
    display: flex;
    gap: calc(var(--padding) / 2);
    flex-wrap: wrap;
  }

  .session-bar :global(.button) {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--gray);
    line-height: 1.6;
  }

  .or-divider {
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
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
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
    font-size: var(--text-base);
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
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
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
    font-size: var(--text-base);
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

  .view-mode-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
  }

  .view-tabs {
    display: flex;
    gap: 4px;
    background: var(--button-elevated);
    padding: 3px;
    border-radius: var(--border-radius);
  }

  .view-tab {
    padding: 6px 14px;
    background: transparent;
    border: none;
    color: var(--gray);
    font-family: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
    border-radius: calc(var(--border-radius) - 2px);
    display: flex;
    align-items: center;
    gap: 6px;
    transition: background 150ms, color 150ms;
  }

  .view-tab.active {
    background: var(--button);
    color: var(--secondary);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
  }

  .view-tab:not(.active):hover {
    color: var(--secondary);
  }

  .view-tab-count {
    font-size: 10.5px;
    background: var(--input-border);
    color: var(--gray);
    padding: 1px 6px;
    border-radius: 8px;
    font-weight: 600;
  }

  .view-tab.active .view-tab-count {
    background: var(--blue);
    color: #fff;
  }

  .create-folder-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: calc(var(--padding) / 2) calc(var(--padding) * 1.25);
    font-size: var(--text-sm);
  }

  .create-folder-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    z-index: 200;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--padding);
  }

  .create-folder-dialog {
    width: min(420px, 100%);
    background: var(--surface, var(--button));
    padding: calc(var(--padding) * 1.5);
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.3);
  }

  .create-folder-dialog h3 {
    margin: 0;
    font-size: 16px;
    color: var(--secondary);
  }

  .dialog-hint {
    margin: 0;
    font-size: 12px;
    color: var(--gray);
    line-height: 1.5;
  }

  .dialog-hint code {
    font-family: monospace;
    background: var(--button-elevated);
    padding: 1px 4px;
    border-radius: 3px;
    font-size: 11px;
    color: var(--blue);
  }

  .create-folder-dialog form {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .dialog-error {
    margin: 0;
    color: var(--red);
    font-size: 12px;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .button.primary {
    background: var(--blue);
    color: #fff;
  }

  .button.primary:hover:not(:disabled) {
    background: color-mix(in oklab, var(--blue) 90%, #000);
  }

  .subtext {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--gray);
  }

  .chats-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .chat-row {
    display: flex;
    align-items: stretch;
    gap: 4px;
    position: relative;
  }

  .chat-row > .chat-item {
    flex: 1;
    min-width: 0;
  }

  .chat-info-btn {
    background: transparent;
    border: none;
    color: var(--gray);
    cursor: pointer;
    padding: 0 12px;
    border-radius: var(--border-radius);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 150ms, color 150ms;
    flex-shrink: 0;
  }

  .chat-info-btn:hover {
    background: var(--button-elevated);
    color: var(--secondary);
  }

  .manage-btn {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: var(--text-sm);
  }

  .transfers-btn {
    position: relative;
  }

  .transfers-badge {
    position: absolute;
    top: -4px;
    right: -4px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    background: var(--blue);
    color: #fff;
    border-radius: 8px;
    font-size: 10px;
    font-weight: 600;
    display: flex;
    align-items: center;
    justify-content: center;
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
    font-size: var(--text-base);
    font-weight: 500;
  }

  .chat-avatar.has-photo {
    background: none;
    overflow: hidden;
  }

  .chat-photo-img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    pointer-events: none;
  }

  .chat-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .chat-title {
    font-size: var(--text-base);
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
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
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
    width: 48px;
    height: 48px;
    min-width: 48px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--gray);
    overflow: hidden;
    flex-shrink: 0;
  }

  .media-icon.has-thumb {
    background: none;
  }

  .thumb-img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    pointer-events: none;
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
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }

  @media (max-width: 720px) {
    .page-logged {
      padding: var(--padding);
      gap: var(--padding);
    }

    .session-bar :global(.button) {
      padding: 6px 8px;
      font-size: 12px;
    }

    .manage-btn {
      padding: 6px 10px !important;
    }

    .view-mode-row {
      flex-wrap: wrap;
      gap: 8px;
    }

    .view-tabs {
      flex: 1 1 auto;
    }

    .filters {
      gap: 4px;
    }

    .filter-btn {
      padding: 5px 10px;
      font-size: 11.5px;
    }

    .chat-row {
      gap: 0;
    }

    .chat-info-btn {
      padding: 0 8px;
    }

    .chat-item {
      padding: 10px;
      gap: 10px;
    }

    .chat-avatar {
      width: 32px;
      height: 32px;
      min-width: 32px;
      font-size: 13px;
    }

    .media-item {
      padding: 8px;
      gap: 8px;
    }

    .media-icon {
      width: 40px;
      height: 40px;
      min-width: 40px;
    }

    .media-name {
      font-size: var(--text-sm);
    }

    .media-meta {
      font-size: 10.5px;
    }

    .media-download-btn {
      padding: 5px 10px;
      font-size: 11.5px;
    }

    .media-header {
      flex-wrap: wrap;
      gap: 8px;
    }

    .login-card {
      padding: var(--padding);
    }
  }

  @media (max-width: 480px) {
    .session-info {
      font-size: 11px;
    }

    .chats-header {
      flex-wrap: wrap;
      gap: 6px;
    }

    .chats-header h2 {
      font-size: 16px;
    }

    .view-tab {
      padding: 5px 10px;
      font-size: 11.5px;
    }

    .create-folder-btn {
      padding: 5px 10px;
      font-size: 11.5px;
    }

    .chat-info-btn {
      padding: 0 6px;
    }
  }
</style>
