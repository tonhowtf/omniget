<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import TelegramMediaPlayer from "./TelegramMediaPlayer.svelte";
  import AlbumCarouselModal from "./AlbumCarouselModal.svelte";
  import ResizeBar from "./ResizeBar.svelte";
  import Skeleton from "./Skeleton.svelte";
  import KeymapHint from "./KeymapHint.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import ChatContextMenu from "./ChatContextMenu.svelte";
  import TgIcon from "./TgIcon.svelte";

  type MenuItem = {
    id: string;
    label: string;
    icon?: string;
    danger?: boolean;
    disabled?: boolean;
    onSelect?: () => void;
  };
  type MenuItemOrSep = MenuItem | { separator: true };
  import {
    telegramListChats,
    telegramListChatsPage,
    telegramRestorePeerHashes,
    telegramGetChatPhoto,
    telegramListMedia,
    telegramDiagListMedia,
    telegramSearchMedia,
    telegramGetThumbnail,
    telegramLogout,
    telegramClearThumbnailCache,
    telegramSessionState,
    invalidateChatsCache,
    telegramGetSelf,
    telegramDeleteMessages,
    telegramForwardMessages,
    telegramEditCaption,
    enqueueTelegramUpload,
    studyLibraryTelegramSettingsGet,
    studyLibraryTelegramSettingsSave,
    studyChatsCacheGet,
    studyChatsCacheUpsert,
    studyChatsCacheSetPhoto,
    studyChatsCacheClear,
    enqueueTelegramDownload,
    enqueueTelegramDownloadBatch,
    telegramSetMute,
    telegramTogglePin,
    telegramCreateFolder,
    telegramDeleteChannel,
    telegramRenameChannel,
    isOmnigetFolder,
    studyPlaybackForChat,
    studyBookmarksToggle,
    studyBookmarksList,
    studyTagsAdd,
    studyTagsRemove,
    studyMediaThumbsForChat,
    studyMediaThumbSet,
    avatarColor,
    avatarGradient,
    bookmarkMatchesSmartFolder,
    type TelegramChat,
    type TelegramChatType,
    type TelegramMediaItem,
    type TelegramMediaType,
    type StudyBookmarkRow,
    type SmartFolderId,
    type TelegramSelf,
  } from "$lib/study-telegram-bridge";

  let {
    outputDir = "",
    onDownloadEnqueued = () => {},
    onLogout = () => {},
  } = $props<{
    outputDir?: string;
    onDownloadEnqueued?: () => void;
    onLogout?: () => void;
  }>();

  type ConnectionStatus = "connected" | "checking" | "disconnected";
  let connectionStatus = $state<ConnectionStatus>("connected");
  let connectionPollTimer: ReturnType<typeof setInterval> | null = null;

  let settingsOpen = $state(false);
  let cachePruneBusy = $state(false);
  let logoutBusy = $state(false);

  let allowRemoteDelete = $state(false);
  // L14: settings ampliados
  let autoResumeDownloads = $state(false);
  let thumbCacheLimitMB = $state(500);
  let bulkConfirmThreshold = $state(10);

  async function loadLibrarySettings() {
    try {
      const s = await studyLibraryTelegramSettingsGet();
      allowRemoteDelete = !!s.allow_remote_delete;
      autoResumeDownloads = !!s.auto_resume_downloads;
      if (typeof s.thumb_cache_limit_mb === "number") thumbCacheLimitMB = s.thumb_cache_limit_mb;
      if (typeof s.bulk_confirm_threshold === "number") bulkConfirmThreshold = s.bulk_confirm_threshold;
      const customDir = (s as { default_output_dir?: string | null }).default_output_dir;
      if (typeof customDir === "string" && customDir.length > 0) outputDirOverride = customDir;
    } catch {
      /* keep defaults */
    }
  }

  async function saveSettingPatch(patch: Record<string, unknown>) {
    try {
      await studyLibraryTelegramSettingsSave(patch);
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    }
  }

  async function toggleAllowRemoteDelete() {
    const next = !allowRemoteDelete;
    allowRemoteDelete = next;
    try {
      await studyLibraryTelegramSettingsSave({ allow_remote_delete: next });
    } catch (e) {
      allowRemoteDelete = !next;
      mediaError = e instanceof Error ? e.message : String(e);
    }
  }

  async function toggleAutoResumeDownloads() {
    const next = !autoResumeDownloads;
    autoResumeDownloads = next;
    try {
      await studyLibraryTelegramSettingsSave({ auto_resume_downloads: next });
    } catch (e) {
      autoResumeDownloads = !next;
      mediaError = e instanceof Error ? e.message : String(e);
    }
  }

  let outputDirOverride = $state<string | null>(null);
  const effectiveOutputDir = $derived(outputDirOverride ?? outputDir);

  async function pickOutputDir() {
    try {
      const selected = await openDialog({ directory: true, multiple: false });
      if (typeof selected === "string" && selected.length > 0) {
        outputDirOverride = selected;
        await studyLibraryTelegramSettingsSave({ default_output_dir: selected });
      }
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    }
  }

  async function resetOutputDir() {
    try {
      outputDirOverride = null;
      await studyLibraryTelegramSettingsSave({ default_output_dir: null });
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    }
  }

  let deleteConfirmOpen = $state(false);
  let deleteRevoke = $state(true);
  let forwardModalOpen = $state(false);
  let forwardModalMode = $state<"copy" | "move">("copy");
  let forwardSearch = $state("");
  let forwardBusy = $state(false);
  let uploadBusy = $state(false);
  let keymapOpen = $state(false);
  let dragHover = $state(false);
  let dragUnlisten: (() => void) | null = null;

  type BandwidthSample = { id: number; percent: number; ts: number };
  const bwDownSamples = new Map<number, BandwidthSample>();
  const bwUpSamples = new Map<number, BandwidthSample>();
  let bandwidthDown = $state(0); // bytes/s
  let bandwidthUp = $state(0);
  let bandwidthActive = $state(false);
  let bandwidthTickTimer: ReturnType<typeof setInterval> | null = null;
  let bwDownUnlisten: UnlistenFn | null = null;
  let bwDoneUnlisten: UnlistenFn | null = null;
  let bwUpUnlisten: UnlistenFn | null = null;
  let bwUpDoneUnlisten: UnlistenFn | null = null;

  function recordSample(map: Map<number, BandwidthSample>, id: number, percent: number) {
    map.set(id, { id, percent, ts: Date.now() });
  }

  function computeRateFromSamples(map: Map<number, BandwidthSample>): number {
    // Rough estimate: % delta * implied size. Without knowing the file size,
    // fallback: count active downloads × 256KB/s as a placeholder activity gauge.
    // Better heuristic: integrate over last second.
    const now = Date.now();
    let activeRecent = 0;
    for (const s of map.values()) {
      if (now - s.ts < 2000) activeRecent++;
    }
    return activeRecent * 512 * 1024;
  }

  function updateBandwidthTick() {
    bandwidthDown = computeRateFromSamples(bwDownSamples);
    bandwidthUp = computeRateFromSamples(bwUpSamples);
    bandwidthActive = bandwidthDown > 0 || bandwidthUp > 0;
    const cutoff = Date.now() - 5000;
    for (const [id, s] of bwDownSamples) {
      if (s.ts < cutoff) bwDownSamples.delete(id);
    }
    for (const [id, s] of bwUpSamples) {
      if (s.ts < cutoff) bwUpSamples.delete(id);
    }
  }

  function fmtRate(bytesPerSec: number): string {
    const kb = bytesPerSec / 1024;
    if (kb < 1024) return `${kb.toFixed(0)} kB/s`;
    const mb = kb / 1024;
    return `${mb.toFixed(1)} MB/s`;
  }

  async function setupBandwidthListeners() {
    try {
      bwDownUnlisten = await listen<{ id: number; platform: string; percent: number }>(
        "generic-download-progress",
        (ev) => {
          if (ev.payload.platform !== "telegram") return;
          recordSample(bwDownSamples, ev.payload.id, ev.payload.percent);
        },
      );
      bwDoneUnlisten = await listen<{ id: number; platform: string }>(
        "generic-download-complete",
        (ev) => {
          if (ev.payload.platform !== "telegram") return;
          bwDownSamples.delete(ev.payload.id);
        },
      );
      bwUpUnlisten = await listen<{ id: number; platform: string; percent: number }>(
        "generic-upload-progress",
        (ev) => {
          if (ev.payload.platform !== "telegram") return;
          recordSample(bwUpSamples, ev.payload.id, ev.payload.percent);
        },
      );
      bwUpDoneUnlisten = await listen<{ id: number; platform: string }>(
        "generic-upload-complete",
        (ev) => {
          if (ev.payload.platform !== "telegram") return;
          bwUpSamples.delete(ev.payload.id);
        },
      );
    } catch {
      /* noop */
    }
  }

  let createFolderOpen = $state(false);
  let createFolderName = $state("");
  let createFolderBusy = $state(false);

  let renameOpen = $state(false);
  let renameValue = $state("");
  let renameBusy = $state(false);
  let deleteFolderConfirmOpen = $state(false);
  let deleteFolderBusy = $state(false);

  function openRename() {
    if (!selectedChat) return;
    renameValue = selectedChat.title.replace(/\s*\[og\]\s*$/i, "").trim();
    renameOpen = true;
  }

  async function commitRename() {
    if (!selectedChat) return;
    const base = renameValue.trim();
    if (!base) {
      renameOpen = false;
      return;
    }
    renameBusy = true;
    try {
      const newTitle = `${base} [og]`;
      await telegramRenameChannel({ chatId: selectedChat.id, title: newTitle });
      const updated: TelegramChat = { ...selectedChat, title: newTitle };
      selectedChat = updated;
      chats = chats.map((c) =>
        c.id === updated.id && c.chat_type === updated.chat_type ? updated : c,
      );
      await studyChatsCacheUpsert({
        chats: [updated],
        startingSortOrder: 0,
        replace: false,
      });
      renameOpen = false;
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      renameBusy = false;
    }
  }

  async function commitDeleteFolder() {
    if (!selectedChat) return;
    deleteFolderBusy = true;
    try {
      await telegramDeleteChannel(selectedChat.id);
      const removedId = selectedChat.id;
      const removedType = selectedChat.chat_type;
      chats = chats.filter(
        (c) => !(c.id === removedId && c.chat_type === removedType),
      );
      try {
        await studyChatsCacheClear();
        if (chats.length > 0) {
          await studyChatsCacheUpsert({ chats, startingSortOrder: 0, replace: true });
        }
      } catch {
        /* noop */
      }
      deleteFolderConfirmOpen = false;
      leaveChat();
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      deleteFolderBusy = false;
    }
  }

  async function commitCreateFolder() {
    const name = createFolderName.trim();
    if (!name) return;
    createFolderBusy = true;
    try {
      const newChat = await telegramCreateFolder(name);
      chats = [newChat, ...chats];
      await studyChatsCacheUpsert({
        chats: [newChat],
        startingSortOrder: -1,
        replace: false,
      });
      createFolderOpen = false;
      createFolderName = "";
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      createFolderBusy = false;
    }
  }

  let editingMessageId = $state<number | null>(null);
  let editingValue = $state("");
  let editingBusy = $state(false);

  function startEditTitle(item: TelegramMediaItem, ev: MouseEvent) {
    ev.stopPropagation();
    if (selectedIds.size > 0) return;
    editingMessageId = item.message_id;
    editingValue = item.file_name;
  }

  function cancelEditTitle() {
    editingMessageId = null;
    editingValue = "";
  }

  async function commitEditTitle(item: TelegramMediaItem) {
    if (!selectedChat || editingBusy) return;
    const newValue = editingValue.trim();
    if (!newValue || newValue === item.file_name) {
      cancelEditTitle();
      return;
    }
    editingBusy = true;
    try {
      await telegramEditCaption({
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        messageId: item.message_id,
        caption: newValue,
      });
      mediaItems = mediaItems.map((x) =>
        x.message_id === item.message_id
          ? { ...x, file_name: newValue }
          : x,
      );
      cancelEditTitle();
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      editingBusy = false;
    }
  }

  async function uploadPaths(paths: string[]) {
    if (!selectedChat || paths.length === 0) return;
    uploadBusy = true;
    try {
      for (const filePath of paths) {
        await enqueueTelegramUpload({
          chat: selectedChat,
          filePath,
          fileSize: 0,
        });
      }
      onDownloadEnqueued();
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      uploadBusy = false;
    }
  }

  async function pickAndUpload() {
    if (!selectedChat || uploadBusy) return;
    uploadBusy = true;
    try {
      const picked = await openDialog({ multiple: true });
      if (!picked) return;
      const paths = Array.isArray(picked) ? picked : [picked];
      for (const p of paths) {
        const filePath = typeof p === "string" ? p : (p as { path: string }).path;
        await enqueueTelegramUpload({
          chat: selectedChat,
          filePath,
          fileSize: 0,
        });
      }
      onDownloadEnqueued();
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      uploadBusy = false;
    }
  }

  let displayedConnectionStatus = $state<ConnectionStatus>("connected");
  let checkingTimer: ReturnType<typeof setTimeout> | null = null;
  let stuckCheckingTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    if (checkingTimer) {
      clearTimeout(checkingTimer);
      checkingTimer = null;
    }
    if (stuckCheckingTimer) {
      clearTimeout(stuckCheckingTimer);
      stuckCheckingTimer = null;
    }
    if (connectionStatus === "checking") {
      checkingTimer = setTimeout(() => {
        displayedConnectionStatus = "checking";
      }, 1000);
      stuckCheckingTimer = setTimeout(() => {
        displayedConnectionStatus = "disconnected";
      }, 4000);
    } else {
      displayedConnectionStatus = connectionStatus;
    }
  });

  async function pollConnection() {
    try {
      const state = await telegramSessionState();
      connectionStatus = state.kind === "authenticated" ? "connected" : "disconnected";
    } catch {
      connectionStatus = "checking";
    }
  }

  function statusLabel(): string {
    if (displayedConnectionStatus === "connected")
      return $t("study.library.telegram.status_connected");
    if (displayedConnectionStatus === "checking")
      return $t("study.library.telegram.status_checking");
    return $t("study.library.telegram.status_disconnected");
  }

  async function pruneCache() {
    cachePruneBusy = true;
    try {
      await telegramClearThumbnailCache();
      chatPhotos = new Map();
      mediaThumbs = new Map();
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      cachePruneBusy = false;
    }
  }

  async function doLogout() {
    logoutBusy = true;
    try {
      await telegramLogout();
      invalidateChatsCache();
      try {
        await studyChatsCacheClear();
      } catch {
        /* best-effort */
      }
      chatPhotos = new Map();
      mediaThumbs = new Map();
      bookmarks = [];
      bookmarkKeys = new Set();
      chats = [];
      selectedChat = null;
      showFavorites = false;
      activeSmartFolder = null;
      settingsOpen = false;
      onLogout();
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      logoutBusy = false;
    }
  }

  type FilterChip = "all" | TelegramMediaType;

  const PAGE_SIZE = 50;
  const MEDIA_TYPES: TelegramMediaType[] = [
    "photo",
    "video",
    "document",
    "audio",
    "gif",
    "round_video",
    "round_voice",
    "photo_video",
    "chat_photos",
    "webpage",
  ];

  type TgIconName =
    | "image"
    | "video"
    | "file"
    | "music"
    | "globe"
    | "gif"
    | "round-video"
    | "voice"
    | "photo-video"
    | "chat-photo";

  const MEDIA_TYPE_ICONS: Record<TelegramMediaType, TgIconName> = {
    photo: "image",
    video: "video",
    document: "file",
    audio: "music",
    webpage: "globe",
    gif: "gif",
    round_video: "round-video",
    round_voice: "voice",
    photo_video: "photo-video",
    chat_photos: "chat-photo",
  };

  let chats = $state<TelegramChat[]>([]);
  let chatsLoading = $state(false);
  let chatsError = $state("");

  // F0.8: surface FLOOD_WAIT to user as friendly countdown pill
  let floodWaitSecs = $state(0);
  let floodWaitTimer: ReturnType<typeof setInterval> | null = null;

  function maybeSurfaceFloodWait(msg: string): boolean {
    const m = /FLOOD[_ ]WAIT[_ ](\d+)|esperar (\d+)s|wait (\d+)s/i.exec(msg);
    if (!m) return false;
    const secs = Number(m[1] ?? m[2] ?? m[3]);
    if (!Number.isFinite(secs) || secs <= 0) return false;
    floodWaitSecs = secs;
    if (floodWaitTimer) clearInterval(floodWaitTimer);
    floodWaitTimer = setInterval(() => {
      floodWaitSecs = Math.max(0, floodWaitSecs - 1);
      if (floodWaitSecs === 0 && floodWaitTimer) {
        clearInterval(floodWaitTimer);
        floodWaitTimer = null;
      }
    }, 1000);
    return true;
  }
  let chatPhotos = $state<Map<string, string>>(new Map());
  let selfInfo = $state<TelegramSelf | null>(null);

  let selectedChat = $state<TelegramChat | null>(null);

  let mediaItems = $state<TelegramMediaItem[]>([]);
  let mediaLoading = $state(false);
  let mediaError = $state("");
  let mediaFilter = $state<FilterChip>("all");
  let mediaThumbs = $state<Map<number, string>>(new Map());
  let mediaHasMore = $state(false);
  let albumModal = $state<{ chatId: number; chatType: string; messageId: number } | null>(null);

  let chatSearch = $state("");
  let mediaSearch = $state("");
  let mediaSearchServer = $state(false);

  let showFavorites = $state(false);
  let activeSmartFolder = $state<SmartFolderId | null>(null);
  let bookmarks = $state<StudyBookmarkRow[]>([]);
  let bookmarkKeys = $state<Set<string>>(new Set());

  const SMART_FOLDERS: SmartFolderId[] = [
    "pdfs",
    "videos",
    "audios",
    "images",
    "untagged",
    "yt-courses",
  ];

  function bookmarkKey(chatId: number, messageId: number) {
    return `${chatId}:${messageId}`;
  }

  function smartFolderCount(id: SmartFolderId): number {
    return bookmarks.filter((b) => bookmarkMatchesSmartFolder(b, id)).length;
  }

  function smartFolderLabel(id: SmartFolderId): string {
    return $t(`study.library.telegram.smart_${id}`);
  }

  function smartFolderIcon(id: SmartFolderId): string {
    if (id === "pdfs") return "📄";
    if (id === "videos") return "🎬";
    if (id === "audios") return "🎵";
    if (id === "images") return "🖼";
    if (id === "yt-courses") return "▶";
    return "🏷";
  }

  function smartFolderIconName(
    id: SmartFolderId,
  ): "file" | "video" | "audio" | "image" | "tag" | "play" {
    if (id === "pdfs") return "file";
    if (id === "videos") return "video";
    if (id === "audios") return "audio";
    if (id === "images") return "image";
    if (id === "yt-courses") return "play";
    return "tag";
  }

  const filteredBookmarks = $derived.by(() => {
    const sf = activeSmartFolder;
    if (!sf) return bookmarks;
    return bookmarks.filter((b) => bookmarkMatchesSmartFolder(b, sf));
  });

  function openSmartFolder(id: SmartFolderId) {
    activeSmartFolder = id;
    showFavorites = false;
    selectedChat = null;
    void loadBookmarks();
  }

  function closeSmartFolder() {
    activeSmartFolder = null;
  }

  function isSavedMessages(c: TelegramChat): boolean {
    return (
      !!selfInfo &&
      c.chat_type === "private" &&
      c.id === selfInfo.user_id
    );
  }

  const savedMessagesChat = $derived(
    chats.find((c) => isSavedMessages(c)) ?? null,
  );

  type ChatFilter = "all" | "unread" | "private" | "group" | "channel" | "folder";
  let chatFilter = $state<ChatFilter>("all");

  function chatMatchesFilter(c: TelegramChat, f: ChatFilter): boolean {
    switch (f) {
      case "all":
        return true;
      case "unread":
        return (c.unread_count ?? 0) > 0;
      case "private":
        return c.chat_type === "private";
      case "group":
        return c.chat_type === "group";
      case "channel":
        return c.chat_type === "channel" && !isOmnigetFolder(c);
      case "folder":
        return isOmnigetFolder(c);
    }
  }

  // F0.4: single-pass tally instead of 6× chats.filter()
  const filterCounts = $derived.by(() => {
    const counts = { all: 0, unread: 0, private: 0, group: 0, channel: 0, folder: 0 };
    for (const c of chats) {
      if (isSavedMessages(c)) continue;
      counts.all += 1;
      if ((c.unread_count ?? 0) > 0) counts.unread += 1;
      if (c.chat_type === "private") {
        counts.private += 1;
      } else if (c.chat_type === "group") {
        counts.group += 1;
      } else if (c.chat_type === "channel") {
        if (isOmnigetFolder(c)) counts.folder += 1;
        else counts.channel += 1;
      }
    }
    return counts;
  });

  const visibleChats = $derived.by(() => {
    const term = chatSearch.trim().toLowerCase();
    let base = chats
      .filter((c) => !isSavedMessages(c))
      .filter((c) => chatMatchesFilter(c, chatFilter));
    if (term) {
      base = base.filter((c) => c.title.toLowerCase().includes(term));
    }
    return base.slice().sort((a, b) => {
      const ap = a.is_pinned ? 1 : 0;
      const bp = b.is_pinned ? 1 : 0;
      if (ap !== bp) return bp - ap;
      const ad = a.last_message_date ?? 0;
      const bd = b.last_message_date ?? 0;
      return bd - ad;
    });
  });

  const firstUnpinnedKey = $derived.by(() => {
    const idx = visibleChats.findIndex((c) => !c.is_pinned);
    return idx > 0 ? chatKey(visibleChats[idx]) : null;
  });

  const visibleMediaItems = $derived.by(() => {
    if (mediaSearchServer) return mediaItems;
    const term = mediaSearch.trim().toLowerCase();
    if (!term) return mediaItems;
    return mediaItems.filter(
      (it) =>
        it.file_name.toLowerCase().includes(term) ||
        (it.caption?.toLowerCase().includes(term) ?? false),
    );
  });

  type MediaGroup = { key: string; label: string; items: TelegramMediaItem[] };

  const dateBucketFmt = new Intl.DateTimeFormat(undefined, {
    month: "long",
    year: "numeric",
  });

  function dateBucket(secs: number): { key: string; label: string } {
    if (!secs) return { key: "unknown", label: "—" };
    const d = new Date(secs * 1000);
    const key = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
    const label = dateBucketFmt.format(d).replace(/^\w/, (c) => c.toUpperCase());
    return { key, label };
  }

  const groupedMediaItems = $derived.by<MediaGroup[]>(() => {
    const out: MediaGroup[] = [];
    let current: MediaGroup | null = null;
    for (const it of visibleMediaItems) {
      const b = dateBucket(it.date);
      if (!current || current.key !== b.key) {
        current = { key: b.key, label: b.label, items: [] };
        out.push(current);
      }
      current.items.push(it);
    }
    return out;
  });

  function chatKey(c: TelegramChat) {
    return `${c.chat_type}:${c.id}`;
  }

  function chatTypeLabel(type: TelegramChatType) {
    if (type === "private") return $t("study.library.telegram.chat_type_private");
    if (type === "group") return $t("study.library.telegram.chat_type_group");
    return $t("study.library.telegram.chat_type_channel");
  }

  function chatInitial(title: string) {
    const trimmed = title.trim();
    if (!trimmed) return "?";
    const codePoints = Array.from(trimmed);
    return codePoints[0]!.toUpperCase();
  }

  const dlgTimeFmt = new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
  const dlgWeekdayFmt = new Intl.DateTimeFormat(undefined, { weekday: "short" });
  const dlgShortDateFmt = new Intl.DateTimeFormat(undefined, {
    day: "2-digit",
    month: "2-digit",
  });
  const dlgYearFmt = new Intl.DateTimeFormat(undefined, {
    day: "2-digit",
    month: "2-digit",
    year: "2-digit",
  });

  function dialogTimestamp(secs?: number): string {
    if (!secs) return "";
    const d = new Date(secs * 1000);
    const now = new Date();
    const sameDay =
      d.getDate() === now.getDate() &&
      d.getMonth() === now.getMonth() &&
      d.getFullYear() === now.getFullYear();
    if (sameDay) return dlgTimeFmt.format(d);
    const diffMs = now.getTime() - d.getTime();
    const sevenDaysMs = 7 * 24 * 60 * 60 * 1000;
    if (diffMs < sevenDaysMs && diffMs > 0) return dlgWeekdayFmt.format(d);
    if (d.getFullYear() === now.getFullYear()) return dlgShortDateFmt.format(d);
    return dlgYearFmt.format(d);
  }

  function unreadBadgeText(n: number): string {
    if (n <= 0) return "";
    if (n > 999) return "999+";
    return String(n);
  }

  const INITIAL_PHOTO_FETCH = 30;
  const PAGE_LIMIT = 40;

  let chatsRefreshing = $state(false);
  let chatsLoadingMore = $state(false);

  type CtxState = { x: number; y: number; items: MenuItemOrSep[] } | null;
  let chatCtxMenu = $state<CtxState>(null);

  function openChatContext(ev: MouseEvent, c: TelegramChat) {
    ev.preventDefault();
    ev.stopPropagation();
    const items: MenuItemOrSep[] = [
      {
        id: "open",
        label: $t("study.library.telegram.ctx_open"),
        icon: "→",
        onSelect: () => selectChat(c),
      },
      { separator: true },
      {
        id: "mark-read",
        label: $t("study.library.telegram.ctx_mark_read"),
        icon: "✓",
        disabled: !(c.unread_count ?? 0),
        onSelect: () => {
          chats = chats.map((x) =>
            x.id === c.id && x.chat_type === c.chat_type
              ? { ...x, unread_count: 0 }
              : x,
          );
        },
      },
      {
        id: "pin",
        label: c.is_pinned
          ? $t("study.library.telegram.ctx_unpin")
          : $t("study.library.telegram.ctx_pin"),
        icon: "📌",
        onSelect: () => {
          const next = !c.is_pinned;
          chats = chats.map((x) =>
            x.id === c.id && x.chat_type === c.chat_type
              ? { ...x, is_pinned: next }
              : x,
          );
          void telegramTogglePin({
            chatId: c.id,
            chatType: c.chat_type as "private" | "group" | "channel",
            pinned: next,
          }).catch((err) => {
            chats = chats.map((x) =>
              x.id === c.id && x.chat_type === c.chat_type
                ? { ...x, is_pinned: !next }
                : x,
            );
            console.error("[tg-pin] failed", err);
          });
        },
      },
      {
        id: "mute",
        label: c.is_muted
          ? $t("study.library.telegram.ctx_unmute")
          : $t("study.library.telegram.ctx_mute"),
        icon: "🔇",
        onSelect: () => {
          const next = !c.is_muted;
          chats = chats.map((x) =>
            x.id === c.id && x.chat_type === c.chat_type
              ? { ...x, is_muted: next }
              : x,
          );
          void telegramSetMute({
            chatId: c.id,
            chatType: c.chat_type as "private" | "group" | "channel",
            muteUntil: next ? 2147483647 : 0,
          }).catch((err) => {
            chats = chats.map((x) =>
              x.id === c.id && x.chat_type === c.chat_type
                ? { ...x, is_muted: !next }
                : x,
            );
            console.error("[tg-mute] failed", err);
          });
        },
      },
    ];
    if (isOmnigetFolder(c)) {
      items.push({ separator: true });
      items.push({
        id: "rename",
        label: $t("study.library.telegram.folder_rename"),
        icon: "✎",
        onSelect: () => {
          selectedChat = c;
          openRename();
        },
      });
      items.push({
        id: "delete",
        label: $t("study.library.telegram.folder_delete"),
        icon: "🗑",
        danger: true,
        onSelect: () => {
          selectedChat = c;
          deleteFolderConfirmOpen = true;
        },
      });
    }
    chatCtxMenu = { x: ev.clientX, y: ev.clientY, items };
  }

  function closeChatContext() {
    chatCtxMenu = null;
  }
  let chatsHasMore = $state(false);
  let nextOffset: import("$lib/study-telegram-bridge").ChatPageOffset | null = null;

  async function loadFromCache(): Promise<boolean> {
    try {
      const res = await studyChatsCacheGet();
      if (res.chats.length === 0) return false;
      const items: TelegramChat[] = res.chats.map((c) => ({
        id: c.id,
        title: c.title,
        chat_type: c.chat_type,
        peer_hash: c.peer_hash,
        last_message: c.last_message ?? undefined,
        last_message_date: c.last_message_date ?? undefined,
        unread_count: c.unread_count,
        is_muted: c.is_muted,
        is_pinned: c.is_pinned,
        is_verified: c.is_verified,
      }));
      chats = items;
      const photos = new Map(chatPhotos);
      for (const c of res.chats) {
        if (c.photo_b64) {
          photos.set(`${c.chat_type}:${c.id}`, c.photo_b64);
        }
      }
      chatPhotos = photos;
      // Restore peer_hashes into the plugin's session state so playback / list_media work
      const hashes: [number, number][] = res.chats
        .filter((c) => c.peer_hash && c.peer_hash !== 0)
        .map((c) => [c.id, c.peer_hash]);
      if (hashes.length > 0) {
        try {
          await telegramRestorePeerHashes(hashes);
        } catch {
          /* noop */
        }
      }
      return true;
    } catch {
      return false;
    }
  }

  let silentDiscoveryDone = false;

  /**
   * Silent background discovery: persists subsequent pages' chats + peer_hashes
   * to the cache WITHOUT updating reactive `chats` state (no UI re-renders).
   * Goal: enrich cache so the next cold start has all peer_hashes ready.
   */
  async function silentDiscoverChats(): Promise<void> {
    if (silentDiscoveryDone || !nextOffset) return;
    silentDiscoveryDone = true;

    await new Promise((r) => setTimeout(r, 1500));

    let cursor: import("$lib/study-telegram-bridge").ChatPageOffset | null = nextOffset;
    const maxPages = 10; // 40 * 10 = up to 400 extra chats indexed
    const allHashes: [number, number][] = [];
    let pageBaseSort = chats.length + 1;

    for (let i = 0; i < maxPages && cursor; i++) {
      try {
        const page = await telegramListChatsPage({
          offsetDate: cursor.date,
          offsetId: cursor.id,
          offsetPeer: {
            peer_id: cursor.peer_id,
            peer_type: cursor.peer_type,
            peer_hash: cursor.peer_hash,
          },
          limit: PAGE_LIMIT,
        });
        if (page.chats.length === 0) break;
        // Persist quietly — does NOT touch reactive `chats` state.
        try {
          await studyChatsCacheUpsert({
            chats: page.chats,
            startingSortOrder: pageBaseSort,
            replace: false,
          });
        } catch {
          /* best-effort */
        }
        pageBaseSort += page.chats.length;
        for (const c of page.chats) {
          if (c.peer_hash && c.peer_hash !== 0) {
            allHashes.push([c.id, c.peer_hash]);
          }
        }
        cursor = page.next_offset;
        if (!cursor) break;
        await new Promise((r) => setTimeout(r, 800));
      } catch {
        break;
      }
    }

    if (allHashes.length > 0) {
      try {
        await telegramRestorePeerHashes(allHashes);
      } catch {
        /* noop */
      }
    }
  }

  async function refreshFromNetwork(force = false): Promise<void> {
    const t0 = performance.now();
    console.time("[F0] refreshFromNetwork");
    chatsError = "";
    chatsRefreshing = true;
    try {
      const page = await telegramListChatsPage({ limit: PAGE_LIMIT });
      console.log(`[F0] refreshFromNetwork: first page=${page.chats.length}`);
      // Don't wipe pages user already loaded via "show more".
      // Only replace if user is still on first page.
      if (chats.length <= page.chats.length || force) {
        chats = page.chats;
        nextOffset = page.next_offset;
        chatsHasMore = !!page.next_offset;
      } else {
        // Merge fresh metadata into existing rows by id, keep extra pages intact.
        const fresh = new Map(page.chats.map((c) => [chatKey(c), c]));
        chats = chats.map((c) => fresh.get(chatKey(c)) ?? c);
      }
      // F0.3: fire-and-forget cache upsert
      studyChatsCacheUpsert({
        chats: page.chats,
        startingSortOrder: 0,
        replace: true,
      }).catch(() => {
        /* best-effort */
      });
      page.chats.slice(0, INITIAL_PHOTO_FETCH).forEach((c) => void fetchChatPhoto(c));
      // Silent discovery enriches cache + peer_hashes for next cold start.
      // Does NOT update `chats` state, so UI stays responsive.
      void silentDiscoverChats();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (!maybeSurfaceFloodWait(msg)) {
        chatsError = msg;
      }
    } finally {
      chatsRefreshing = false;
      console.timeEnd("[F0] refreshFromNetwork");
      console.log(`[F0] refreshFromNetwork total=${(performance.now() - t0).toFixed(0)}ms`);
    }
  }

  async function loadChats(force = false) {
    chatsLoading = true;
    try {
      const hadCache = !force && (await loadFromCache());
      if (!hadCache) {
        await refreshFromNetwork(force);
      } else {
        chatsLoading = false;
        void refreshFromNetwork(false);
        return;
      }
    } finally {
      chatsLoading = false;
    }
  }

  async function loadMoreChats() {
    if (!nextOffset || chatsLoadingMore) return;
    const t0 = performance.now();
    console.time("[F0] loadMoreChats");
    console.time("[F0] loadMoreChats.ipc");
    chatsLoadingMore = true;
    try {
      const page = await telegramListChatsPage({
        offsetDate: nextOffset.date,
        offsetId: nextOffset.id,
        offsetPeer: {
          peer_id: nextOffset.peer_id,
          peer_type: nextOffset.peer_type,
          peer_hash: nextOffset.peer_hash,
        },
        limit: PAGE_LIMIT,
      });
      console.timeEnd("[F0] loadMoreChats.ipc");
      console.log(`[F0] loadMoreChats: page=${page.chats.length} chats total=${chats.length}`);

      console.time("[F0] loadMoreChats.reactive-append");
      const seen = new Set(chats.map((c) => chatKey(c)));
      const fresh = page.chats.filter((c) => !seen.has(chatKey(c)));
      chats = [...chats, ...fresh];
      nextOffset = page.next_offset;
      chatsHasMore = !!page.next_offset;
      console.timeEnd("[F0] loadMoreChats.reactive-append");

      // F0.3: fire-and-forget cache upsert; we don't need its completion to render.
      studyChatsCacheUpsert({
        chats: page.chats,
        startingSortOrder: chats.length - page.chats.length,
        replace: false,
      }).catch(() => {
        /* best-effort */
      });
      // Photo fetches now go through pLimit(4) — see fetchChatPhoto.
      page.chats.slice(0, INITIAL_PHOTO_FETCH).forEach((c) => void fetchChatPhoto(c));
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (!maybeSurfaceFloodWait(msg)) chatsError = msg;
    } finally {
      chatsLoadingMore = false;
      console.timeEnd("[F0] loadMoreChats");
      console.log(`[F0] loadMoreChats total=${(performance.now() - t0).toFixed(0)}ms`);
    }
  }

  function ensureChatPhoto(c: TelegramChat) {
    if (chatPhotos.has(chatKey(c))) return;
    void fetchChatPhoto(c);
  }

  function lazyPhoto(node: HTMLElement, chat: TelegramChat) {
    ensureChatPhoto(chat);
    if (typeof IntersectionObserver === "undefined") return {};
    let current = chat;
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            ensureChatPhoto(current);
            observer.disconnect();
            break;
          }
        }
      },
      { rootMargin: "400px" },
    );
    observer.observe(node);
    return {
      update(next: TelegramChat) {
        current = next;
        ensureChatPhoto(next);
      },
      destroy() {
        observer.disconnect();
      },
    };
  }

  function reloadChats() {
    invalidateChatsCache();
    nextOffset = null;
    chatsHasMore = false;
    void refreshFromNetwork(true);
  }

  async function loadSelf() {
    try {
      selfInfo = await telegramGetSelf();
    } catch {
      selfInfo = null;
    }
  }

  async function loadBookmarks() {
    try {
      const res = await studyBookmarksList();
      bookmarks = res.bookmarks;
      bookmarkKeys = new Set(
        bookmarks.map((b) => bookmarkKey(b.chat_id, b.message_id)),
      );
    } catch {
      bookmarks = [];
      bookmarkKeys = new Set();
    }
  }

  async function toggleBookmark(item: TelegramMediaItem) {
    if (!selectedChat) return;
    try {
      const res = await studyBookmarksToggle({
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        messageId: item.message_id,
        title: item.file_name,
        fileSize: item.file_size,
        mimeType: item.media_type,
        thumbB64: mediaThumbs.get(item.message_id) ?? undefined,
      });
      const next = new Set(bookmarkKeys);
      const key = bookmarkKey(selectedChat.id, item.message_id);
      if (res.bookmarked) {
        next.add(key);
      } else {
        next.delete(key);
        bookmarks = bookmarks.filter(
          (b) =>
            !(b.chat_id === selectedChat!.id && b.message_id === item.message_id),
        );
      }
      bookmarkKeys = next;
      if (res.bookmarked) {
        await loadBookmarks();
      }
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    }
  }

  async function removeBookmarkRow(b: StudyBookmarkRow) {
    try {
      await studyBookmarksToggle({
        chatId: b.chat_id,
        chatType: b.chat_type,
        messageId: b.message_id,
        title: b.title,
      });
      const next = new Set(bookmarkKeys);
      next.delete(bookmarkKey(b.chat_id, b.message_id));
      bookmarkKeys = next;
      bookmarks = bookmarks.filter((x) => x.id !== b.id);
    } catch {
      /* noop */
    }
  }

  let tagInputFor = $state<number | null>(null);
  let tagInputValue = $state("");
  let tagBusy = $state(false);

  function startAddTag(bookmarkId: number) {
    tagInputFor = bookmarkId;
    tagInputValue = "";
  }

  function cancelAddTag() {
    tagInputFor = null;
    tagInputValue = "";
  }

  async function commitAddTag(b: StudyBookmarkRow) {
    const raw = tagInputValue.trim();
    if (!raw) {
      cancelAddTag();
      return;
    }
    tagBusy = true;
    try {
      const res = await studyTagsAdd({
        chatId: b.chat_id,
        messageId: b.message_id,
        tag: raw,
      });
      bookmarks = bookmarks.map((x) =>
        x.id === b.id && !x.tags.includes(res.tag)
          ? { ...x, tags: [...x.tags, res.tag].sort() }
          : x,
      );
      tagInputFor = null;
      tagInputValue = "";
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      tagBusy = false;
    }
  }

  async function removeTag(b: StudyBookmarkRow, tag: string) {
    try {
      await studyTagsRemove({
        chatId: b.chat_id,
        messageId: b.message_id,
        tag,
      });
      bookmarks = bookmarks.map((x) =>
        x.id === b.id ? { ...x, tags: x.tags.filter((t) => t !== tag) } : x,
      );
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    }
  }

  function openFavorites() {
    showFavorites = true;
    selectedChat = null;
    activeSmartFolder = null;
    void loadBookmarks();
  }

  function closeFavorites() {
    showFavorites = false;
  }

  // F0.1: simple semaphore limiting concurrent IPC photo fetches.
  // Prevents flooding plugin-telegram with 30+ parallel MTProto calls
  // which trigger FLOOD_WAIT and cascading retries.
  type LimitFn = (fn: () => Promise<unknown>) => Promise<unknown>;
  function makeLimit(max: number): LimitFn {
    let active = 0;
    const queue: (() => void)[] = [];
    function next() {
      if (active >= max || queue.length === 0) return;
      active++;
      const run = queue.shift()!;
      run();
    }
    return function (fn) {
      return new Promise((resolve, reject) => {
        const run = async () => {
          try {
            resolve(await fn());
          } catch (e) {
            reject(e);
          } finally {
            active--;
            next();
          }
        };
        queue.push(run);
        next();
      });
    };
  }
  const photoFetchLimit = makeLimit(4);

  // F0.2: raf-batched updates so 30 photos arriving across ~1s yield ~1
  // reactive update per frame instead of 30 sequential Map reassignments.
  const pendingPhotoUpdates = new Map<string, string>();
  let photoFlushScheduled = false;
  function flushPhotoUpdates() {
    photoFlushScheduled = false;
    if (pendingPhotoUpdates.size === 0) return;
    const next = new Map(chatPhotos);
    for (const [k, v] of pendingPhotoUpdates) next.set(k, v);
    pendingPhotoUpdates.clear();
    chatPhotos = next;
  }
  function schedulePhotoFlush() {
    if (photoFlushScheduled) return;
    photoFlushScheduled = true;
    requestAnimationFrame(flushPhotoUpdates);
  }

  async function fetchChatPhoto(chat: TelegramChat) {
    const key = chatKey(chat);
    if (chatPhotos.has(key) || pendingPhotoUpdates.has(key)) return;
    await photoFetchLimit(async () => {
      if (chatPhotos.has(key) || pendingPhotoUpdates.has(key)) return;
      try {
        const b64 = await telegramGetChatPhoto({
          chatId: chat.id,
          chatType: chat.chat_type,
        });
        if (b64) {
          pendingPhotoUpdates.set(key, b64);
          schedulePhotoFlush();
          studyChatsCacheSetPhoto({ chatId: chat.id, photoB64: b64 }).catch(() => {
            /* cache write best-effort */
          });
        }
      } catch {
        /* sem foto disponível */
      }
    });
  }

  let lastVisitedChatKey = $state<string | null>(null);

  function defaultMediaFilterFor(chat: TelegramChat): FilterChip {
    if (isSavedMessages(chat)) return "webpage";
    return "all";
  }

  async function selectChat(chat: TelegramChat) {
    console.log(
      `[TG] selectChat: type=${chat.chat_type} id=${chat.id} title="${chat.title}" hash=${chat.peer_hash ?? "?"}`,
    );
    selectedChat = chat;
    lastVisitedChatKey = chatKey(chat);
    showFavorites = false;
    mediaFilter = defaultMediaFilterFor(chat);
    mediaItems = [];
    mediaThumbs = new Map();
    mediaHasMore = false;
    mediaSearch = "";
    mediaSearchServer = false;
    playbackByMessage = new Map();
    clearSelection();
    void preloadCachedThumbs(chat.id);
    await loadMedia(true);
    if (mediaItems.length === 0 && !mediaError && mediaFilter !== "all") {
      console.warn(
        `[TG] selectChat: 0 items com filter=${mediaFilter}, tentando filter=all`,
      );
      mediaFilter = "all";
      await loadMedia(true);
    }
    if (mediaItems.length === 0 && !mediaError) {
      console.warn(
        `[TG] selectChat: chat="${chat.title}" tipo=${chat.chat_type} retornou 0 medias mesmo com filter=all — rodando diag`,
      );
      try {
        const diag = await telegramDiagListMedia({
          chatId: chat.id,
          chatType: chat.chat_type,
        });
        console.warn("[TG] diag_list_media:", diag);
        if (diag.get_history_error) {
          mediaError = $t("telegram.toolbox.media_access_denied", { message: diag.get_history_error });
        } else if (diag.search_errors.length > 0) {
          mediaError = $t("telegram.toolbox.media_filter_errors", { message: diag.search_errors.join("; ") });
        } else if (diag.get_history_count === 0 && diag.search_photo_count === 0 && diag.search_video_count === 0 && diag.search_document_count === 0) {
          mediaError = $t("telegram.toolbox.media_unavailable");
        } else if (diag.get_history_with_media === 0) {
          mediaError = $t("telegram.toolbox.media_not_detected", { count: diag.get_history_count });
        }
      } catch (e) {
        console.warn("[TG] diag_list_media failed:", e);
      }
    }
    void loadPlaybackForChat(chat.id);
  }

  async function preloadCachedThumbs(chatId: number) {
    try {
      const res = await studyMediaThumbsForChat({ chatId });
      const next = new Map(mediaThumbs);
      for (const [k, v] of Object.entries(res.thumbs)) {
        const id = Number(k);
        if (!next.has(id)) next.set(id, v);
      }
      mediaThumbs = next;
    } catch {
      /* cache miss is fine */
    }
  }

  function leaveChat() {
    selectedChat = null;
    mediaItems = [];
    mediaThumbs = new Map();
    mediaError = "";
    mediaSearch = "";
    mediaSearchServer = false;
    clearSelection();
  }

  async function searchOnServer() {
    if (!selectedChat) return;
    const q = mediaSearch.trim();
    if (q.length < 2) return;
    mediaLoading = true;
    mediaError = "";
    try {
      const args: Parameters<typeof telegramSearchMedia>[0] = {
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        query: q,
        limit: PAGE_SIZE,
      };
      if (mediaFilter !== "all") args.mediaType = mediaFilter;
      const items = await telegramSearchMedia(args);
      mediaItems = items;
      mediaHasMore = false;
      mediaSearchServer = true;
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      mediaLoading = false;
    }
  }

  function clearSearchServer() {
    mediaSearchServer = false;
    mediaSearch = "";
    void loadMedia(true);
  }

  async function loadMedia(reset: boolean) {
    if (!selectedChat) return;
    mediaLoading = true;
    mediaError = "";
    const t0 = performance.now();
    console.log(`[TG] loadMedia reset=${reset} offset=${reset ? 0 : mediaItems.length} filter=${mediaFilter}`);
    try {
      const args: Parameters<typeof telegramListMedia>[0] = {
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        offset: reset ? 0 : mediaItems.length,
        limit: PAGE_SIZE,
      };
      if (mediaFilter !== "all") args.mediaType = mediaFilter;
      const fetchPromise = telegramListMedia(args);
      const timeoutMs = 25_000;
      const timeoutPromise = new Promise<TelegramMediaItem[]>((_, reject) =>
        setTimeout(
          () => reject(new Error(`telegram_list_media timeout (${timeoutMs}ms) — provavelmente FLOOD_WAIT, tente novamente em alguns segundos`)),
          timeoutMs,
        ),
      );
      const items = await Promise.race([fetchPromise, timeoutPromise]);
      console.log(`[TG] loadMedia ok in ${(performance.now() - t0).toFixed(0)}ms, returned ${items.length} items`);
      mediaItems = reset ? items : [...mediaItems, ...items];
      mediaHasMore = items.length >= PAGE_SIZE;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.warn(`[TG] loadMedia failed in ${(performance.now() - t0).toFixed(0)}ms:`, msg);
      mediaError = msg;
    } finally {
      mediaLoading = false;
    }
  }

  // F0.1 + F0.2: throttle + raf-batch for media thumbs
  const thumbFetchLimit = makeLimit(4);
  const pendingThumbUpdates = new Map<number, string>();
  let thumbFlushScheduled = false;
  function flushThumbUpdates() {
    thumbFlushScheduled = false;
    if (pendingThumbUpdates.size === 0) return;
    const next = new Map(mediaThumbs);
    for (const [k, v] of pendingThumbUpdates) next.set(k, v);
    pendingThumbUpdates.clear();
    mediaThumbs = next;
  }
  function scheduleThumbFlush() {
    if (thumbFlushScheduled) return;
    thumbFlushScheduled = true;
    requestAnimationFrame(flushThumbUpdates);
  }

  async function fetchThumb(item: TelegramMediaItem) {
    if (mediaThumbs.has(item.message_id) || pendingThumbUpdates.has(item.message_id)) return;
    if (!selectedChat) return;
    if (!isThumbnailable(item)) return;
    const chatId = selectedChat.id;
    const chatType = selectedChat.chat_type;
    await thumbFetchLimit(async () => {
      if (mediaThumbs.has(item.message_id) || pendingThumbUpdates.has(item.message_id)) return;
      try {
        const b64 = await telegramGetThumbnail({
          chatId,
          chatType,
          messageId: item.message_id,
        });
        if (b64) {
          pendingThumbUpdates.set(item.message_id, b64);
          scheduleThumbFlush();
          studyMediaThumbSet({
            chatId,
            messageId: item.message_id,
            thumbB64: b64,
          }).catch(() => {
            /* cache write best-effort */
          });
        }
      } catch {
        /* fallback pra ícone genérico */
      }
    });
  }

  function ripplePointer(node: HTMLElement) {
    function handle(ev: PointerEvent) {
      const r = node.getBoundingClientRect();
      const x = ((ev.clientX - r.left) / r.width) * 100;
      const y = ((ev.clientY - r.top) / r.height) * 100;
      node.style.setProperty("--ripple-x", `${x}%`);
      node.style.setProperty("--ripple-y", `${y}%`);
    }
    node.addEventListener("pointerdown", handle);
    return {
      destroy() {
        node.removeEventListener("pointerdown", handle);
      },
    };
  }

  function lazyMediaThumb(node: HTMLElement, item: TelegramMediaItem) {
    void fetchThumb(item);
    if (typeof IntersectionObserver === "undefined") return {};
    let current = item;
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            void fetchThumb(current);
            observer.disconnect();
            break;
          }
        }
      },
      { rootMargin: "400px" },
    );
    observer.observe(node);
    return {
      update(next: TelegramMediaItem) {
        current = next;
        void fetchThumb(next);
      },
      destroy() {
        observer.disconnect();
      },
    };
  }

  function isThumbnailable(item: TelegramMediaItem) {
    const t = item.media_type.toLowerCase();
    return t === "photo" || t === "video" || t === "document" || t === "round_video" || t === "gif";
  }

  type CardVariant = "video" | "audio" | "image" | "voice" | "round" | "gif" | "webpage" | "generic";

  function cardVariantForMedia(item: TelegramMediaItem): CardVariant {
    const t = item.media_type.toLowerCase();
    if (t === "voice" || t === "round_voice") return "voice";
    if (t === "round_video") return "round";
    if (t === "gif") return "gif";
    if (t === "video") return "video";
    if (t === "audio") return "audio";
    if (t === "photo") return "image";
    if (t === "webpage") return "webpage";
    return "generic";
  }

  function voiceWaveformBars(messageId: number): number[] {
    const heights: number[] = [];
    let seed = messageId;
    for (let i = 0; i < 16; i++) {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      heights.push(20 + (seed % 80));
    }
    return heights;
  }

  function approxDuration(bytes: number): string {
    if (!bytes) return "";
    const seconds = Math.max(1, Math.round(bytes / 16000));
    if (seconds < 60) return `${seconds}s`;
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  let hoveringGifId = $state<number | null>(null);
  const reducedMotion = $derived(typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches);

  function formatBytes(n: number) {
    if (!n) return "";
    const units = ["B", "KB", "MB", "GB", "TB"];
    let v = n;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function formatDate(secs: number) {
    if (!secs) return "";
    return new Date(secs * 1000).toLocaleDateString();
  }

  function setFilter(f: FilterChip) {
    if (f === mediaFilter) return;
    mediaFilter = f;
    clearSelection();
    void loadMedia(true);
  }

  function mediaTypeIcon(type: string): string {
    const lc = type.toLowerCase();
    if (lc === "photo") return "🖼";
    if (lc === "video") return "🎬";
    if (lc === "audio") return "🎵";
    if (lc === "webpage") return "🌐";
    return "📄";
  }

  let pendingDownloads = $state<Set<number>>(new Set());
  let selectedIds = $state<Set<number>>(new Set());
  let lastSelectedIdx = $state<number | null>(null);
  let bulkBusy = $state(false);

  let playerOpen = $state(false);
  let playerIdx = $state(0);
  let playerOriginRect = $state<DOMRect | undefined>(undefined);

  type PlaybackInfo = {
    position: number;
    duration: number;
    finished: boolean;
  };
  let playbackByMessage = $state<Map<number, PlaybackInfo>>(new Map());

  async function loadPlaybackForChat(chatId: number) {
    try {
      const res = await studyPlaybackForChat(chatId);
      const m = new Map<number, PlaybackInfo>();
      for (const r of res.items) {
        m.set(r.message_id, {
          position: r.position_seconds,
          duration: r.duration_seconds,
          finished: r.finished,
        });
      }
      playbackByMessage = m;
    } catch {
      playbackByMessage = new Map();
    }
  }

  function fmtTime(seconds: number): string {
    if (!seconds || !Number.isFinite(seconds)) return "";
    const s = Math.floor(seconds % 60);
    const m = Math.floor((seconds / 60) % 60);
    const h = Math.floor(seconds / 3600);
    if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
    return `${m}:${String(s).padStart(2, "0")}`;
  }

  function openPlayer(idx: number, originEl?: HTMLElement) {
    if (!selectedChat) return;
    playerIdx = idx;
    playerOriginRect = originEl ? originEl.getBoundingClientRect() : undefined;
    playerOpen = true;
  }

  function closePlayer() {
    playerOpen = false;
    if (selectedChat) {
      void loadPlaybackForChat(selectedChat.id);
    }
  }

  function longPressSelect(node: HTMLElement, item: TelegramMediaItem) {
    let timer: ReturnType<typeof setTimeout> | null = null;
    let triggered = false;
    let startX = 0;
    let startY = 0;

    function onDown(ev: PointerEvent) {
      if (ev.button !== 0) return;
      startX = ev.clientX;
      startY = ev.clientY;
      triggered = false;
      timer = setTimeout(() => {
        triggered = true;
        if (!selectedIds.has(item.message_id)) {
          const next = new Set(selectedIds);
          next.add(item.message_id);
          selectedIds = next;
          if (navigator.vibrate) navigator.vibrate(15);
        }
      }, 500);
    }
    function clear() {
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
    }
    function onMove(ev: PointerEvent) {
      if (!timer) return;
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) > 8) clear();
    }
    function onClickCapture(ev: MouseEvent) {
      if (triggered) {
        ev.stopPropagation();
        ev.preventDefault();
        triggered = false;
      }
    }
    node.addEventListener("pointerdown", onDown);
    node.addEventListener("pointermove", onMove);
    node.addEventListener("pointerup", clear);
    node.addEventListener("pointercancel", clear);
    node.addEventListener("pointerleave", clear);
    node.addEventListener("click", onClickCapture, true);
    return {
      destroy() {
        node.removeEventListener("pointerdown", onDown);
        node.removeEventListener("pointermove", onMove);
        node.removeEventListener("pointerup", clear);
        node.removeEventListener("pointercancel", clear);
        node.removeEventListener("pointerleave", clear);
        node.removeEventListener("click", onClickCapture, true);
        clear();
      },
    };
  }

  function clearSelection() {
    selectedIds = new Set();
    lastSelectedIdx = null;
  }

  function toggleSelect(item: TelegramMediaItem, idx: number, ev: MouseEvent) {
    ev.preventDefault();
    if (ev.shiftKey && lastSelectedIdx !== null) {
      const a = Math.min(lastSelectedIdx, idx);
      const b = Math.max(lastSelectedIdx, idx);
      const next = new Set(selectedIds);
      for (let i = a; i <= b; i++) {
        const it = visibleMediaItems[i];
        if (it) next.add(it.message_id);
      }
      selectedIds = next;
      return;
    }
    if (ev.ctrlKey || ev.metaKey || selectedIds.size > 0) {
      const next = new Set(selectedIds);
      if (next.has(item.message_id)) next.delete(item.message_id);
      else next.add(item.message_id);
      selectedIds = next;
      lastSelectedIdx = idx;
      return;
    }
    const cardEl = (ev.currentTarget as HTMLElement | null) ?? undefined;
    const thumbEl = cardEl?.querySelector(".thumb") as HTMLElement | null;
    openPlayer(idx, thumbEl ?? cardEl ?? undefined);
  }

  function selectAllVisible() {
    const next = new Set(visibleMediaItems.map((x) => x.message_id));
    selectedIds = next;
  }

  async function bulkDownload() {
    if (!selectedChat || selectedIds.size === 0) return;
    if (!effectiveOutputDir) {
      mediaError = $t("study.library.telegram.downloads.no_output_dir");
      return;
    }
    bulkBusy = true;
    try {
      const items = visibleMediaItems
        .filter((x) => selectedIds.has(x.message_id))
        .map((x) => ({
          message_id: x.message_id,
          file_name: x.file_name,
          file_size: x.file_size,
        }));
      await enqueueTelegramDownloadBatch({
        chat: selectedChat,
        items,
        outputDir: effectiveOutputDir,
      });
      onDownloadEnqueued();
      clearSelection();
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      bulkBusy = false;
    }
  }

  // L11: infer tags from filename extension and media_type.
  function inferTagsFromItem(item: TelegramMediaItem): string[] {
    const tags = new Set<string>();
    const mt = item.media_type.toLowerCase();
    if (mt === "photo") tags.add("photo");
    else if (mt === "video") tags.add("video");
    else if (mt === "audio") tags.add("audio");
    else if (mt === "webpage") tags.add("link");

    const ext = item.file_name.toLowerCase().split(".").pop() ?? "";
    const extMap: Record<string, string> = {
      pdf: "pdf",
      epub: "ebook",
      mobi: "ebook",
      azw3: "ebook",
      mp4: "video",
      mkv: "video",
      mov: "video",
      webm: "video",
      avi: "video",
      mp3: "audio",
      m4a: "audio",
      flac: "audio",
      ogg: "audio",
      wav: "audio",
      jpg: "photo",
      jpeg: "photo",
      png: "photo",
      gif: "photo",
      webp: "photo",
      zip: "archive",
      rar: "archive",
      "7z": "archive",
      doc: "document",
      docx: "document",
      xls: "document",
      xlsx: "document",
      ppt: "document",
      pptx: "document",
      txt: "document",
    };
    const inferred = extMap[ext];
    if (inferred) tags.add(inferred);
    return Array.from(tags);
  }

  async function bulkBookmark() {
    if (!selectedChat || selectedIds.size === 0) return;
    bulkBusy = true;
    try {
      const items = visibleMediaItems.filter((x) =>
        selectedIds.has(x.message_id),
      );
      const newlyBookmarked: TelegramMediaItem[] = [];
      for (const it of items) {
        const key = bookmarkKey(selectedChat.id, it.message_id);
        if (bookmarkKeys.has(key)) continue;
        await studyBookmarksToggle({
          chatId: selectedChat.id,
          chatType: selectedChat.chat_type,
          messageId: it.message_id,
          title: it.file_name,
          fileSize: it.file_size,
          mimeType: it.media_type,
          thumbB64: mediaThumbs.get(it.message_id) ?? undefined,
        });
        newlyBookmarked.push(it);
      }
      // L11: auto-tag in background — não bloqueia o flow
      for (const it of newlyBookmarked) {
        const tags = inferTagsFromItem(it);
        for (const tag of tags) {
          studyTagsAdd({
            chatId: selectedChat.id,
            messageId: it.message_id,
            tag,
          }).catch(() => {
            /* tag write best-effort */
          });
        }
      }
      await loadBookmarks();
      clearSelection();
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      bulkBusy = false;
    }
  }

  async function bulkDelete() {
    if (!selectedChat || selectedIds.size === 0) return;
    bulkBusy = true;
    try {
      const ids = Array.from(selectedIds);
      await telegramDeleteMessages({
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        messageIds: ids,
        revoke: deleteRevoke,
      });
      const set = new Set(ids);
      mediaItems = mediaItems.filter((x) => !set.has(x.message_id));
      const newThumbs = new Map(mediaThumbs);
      ids.forEach((id) => newThumbs.delete(id));
      mediaThumbs = newThumbs;
      const removedKeys = new Set(
        ids.map((id) => bookmarkKey(selectedChat!.id, id)),
      );
      bookmarkKeys = new Set(
        Array.from(bookmarkKeys).filter((k) => !removedKeys.has(k)),
      );
      bookmarks = bookmarks.filter(
        (b) =>
          !(
            b.chat_id === selectedChat!.id &&
            ids.includes(b.message_id)
          ),
      );
      clearSelection();
      deleteConfirmOpen = false;
      const count = ids.length;
      if (deleteRevoke) {
        showToast(
          "info",
          $t("study.library.telegram.delete_done_revoke", { n: count }),
          5000,
        );
      } else {
        showToast(
          "info",
          $t("study.library.telegram.delete_done_local", { n: count }),
          5000,
        );
      }
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      bulkBusy = false;
    }
  }

  function openForwardModal(mode: "copy" | "move") {
    if (!selectedChat || selectedIds.size === 0) return;
    forwardModalMode = mode;
    forwardSearch = "";
    forwardModalOpen = true;
  }

  async function commitForward(targetChat: TelegramChat) {
    if (!selectedChat || selectedIds.size === 0) return;
    if (forwardModalMode === "move" && !allowRemoteDelete) {
      mediaError = $t("study.library.telegram.move_needs_delete_perm");
      return;
    }
    forwardBusy = true;
    try {
      const ids = Array.from(selectedIds);
      await telegramForwardMessages({
        fromChatId: selectedChat.id,
        fromChatType: selectedChat.chat_type,
        toChatId: targetChat.id,
        toChatType: targetChat.chat_type,
        messageIds: ids,
        dropAuthor: false,
        dropCaptions: false,
      });
      if (forwardModalMode === "move") {
        await telegramDeleteMessages({
          chatId: selectedChat.id,
          chatType: selectedChat.chat_type,
          messageIds: ids,
          revoke: true,
        });
        const set = new Set(ids);
        mediaItems = mediaItems.filter((x) => !set.has(x.message_id));
      }
      forwardModalOpen = false;
      clearSelection();
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
    } finally {
      forwardBusy = false;
    }
  }

  const forwardCandidates = $derived.by(() => {
    const term = forwardSearch.trim().toLowerCase();
    const list = chats.filter(
      (c) => !selectedChat || c.id !== selectedChat.id || c.chat_type !== selectedChat.chat_type,
    );
    if (!term) return list;
    return list.filter((c) => c.title.toLowerCase().includes(term));
  });

  function onKeyDown(ev: KeyboardEvent) {
    const target = ev.target as HTMLElement | null;
    const tag = target?.tagName;
    const inEditableField =
      tag === "INPUT" || tag === "TEXTAREA" || target?.isContentEditable;

    // L7: Ctrl/Cmd+F focuses the local search input even from outside fields.
    // If already in an INPUT, let browser default Find behavior take over.
    if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "f" && !inEditableField) {
      const sel = selectedChat
        ? '.tg-browser input.search-input[placeholder*="mídia" i], .tg-browser input.search-input[placeholder*="media" i]'
        : '.tg-browser input.search-input[placeholder*="chat" i]';
      const input = document.querySelector(sel) as HTMLInputElement | null
        ?? document.querySelector(".tg-browser input.search-input") as HTMLInputElement | null;
      if (input) {
        ev.preventDefault();
        input.focus();
        input.select();
        return;
      }
    }

    if (inEditableField) {
      // Allow Escape from search input to clear + blur
      if (ev.key === "Escape" && tag === "INPUT" && (target as HTMLInputElement).type === "search") {
        (target as HTMLInputElement).value = "";
        (target as HTMLInputElement).dispatchEvent(new Event("input", { bubbles: true }));
        (target as HTMLInputElement).blur();
        ev.preventDefault();
      }
      return;
    }
    if (ev.key === "?" || (ev.shiftKey && ev.key === "/")) {
      keymapOpen = !keymapOpen;
      ev.preventDefault();
      return;
    }
    if (ev.key === "Escape" && keymapOpen) {
      keymapOpen = false;
      ev.preventDefault();
      return;
    }
    if (!selectedChat) return;
    if (ev.key === "Escape" && selectedIds.size > 0) {
      clearSelection();
      ev.preventDefault();
    } else if (
      (ev.ctrlKey || ev.metaKey) &&
      ev.key.toLowerCase() === "a"
    ) {
      selectAllVisible();
      ev.preventDefault();
    }
  }

  $effect(() => {
    if (typeof window === "undefined") return;
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  async function downloadItem(item: TelegramMediaItem) {
    if (!selectedChat) return;
    if (!effectiveOutputDir) {
      mediaError = $t("study.library.telegram.downloads.no_output_dir");
      return;
    }
    if (pendingDownloads.has(item.message_id)) return;
    const next = new Set(pendingDownloads);
    next.add(item.message_id);
    pendingDownloads = next;
    try {
      await enqueueTelegramDownload({
        chatId: selectedChat.id,
        chatType: selectedChat.chat_type,
        messageId: item.message_id,
        fileName: item.file_name,
        fileSize: item.file_size,
        outputDir: effectiveOutputDir,
      });
      onDownloadEnqueued();
    } catch (e) {
      mediaError = e instanceof Error ? e.message : String(e);
      const back = new Set(pendingDownloads);
      back.delete(item.message_id);
      pendingDownloads = back;
    }
  }

  async function setupDragDrop() {
    try {
      const webview = getCurrentWebview();
      const unlisten = await webview.onDragDropEvent(async (event) => {
        if (!selectedChat) {
          dragHover = false;
          return;
        }
        const p = event.payload;
        if (p.type === "enter" || p.type === "over") {
          dragHover = true;
        } else if (p.type === "leave") {
          dragHover = false;
        } else if (p.type === "drop") {
          dragHover = false;
          const paths = (p as unknown as { paths?: string[] }).paths ?? [];
          if (paths.length > 0) await uploadPaths(paths);
        }
      });
      dragUnlisten = unlisten;
    } catch {
      /* drag-drop not available in this context */
    }
  }

  onMount(() => {
    void loadChats();
    void loadBookmarks();
    void loadSelf();
    void loadLibrarySettings();
    void pollConnection();
    void setupDragDrop();
    void setupBandwidthListeners();
    connectionPollTimer = setInterval(pollConnection, 30_000);
    bandwidthTickTimer = setInterval(updateBandwidthTick, 1000);
  });

  onDestroy(() => {
    if (dragUnlisten) {
      try { dragUnlisten(); } catch { /* noop */ }
    }
    if (connectionPollTimer) clearInterval(connectionPollTimer);
    if (bandwidthTickTimer) clearInterval(bandwidthTickTimer);
    bwDownUnlisten?.();
    bwDoneUnlisten?.();
    bwUpUnlisten?.();
    bwUpDoneUnlisten?.();
  });
</script>

{#if showFavorites || activeSmartFolder}
  <div class="tg-browser">
    <nav class="crumbs" aria-label="breadcrumb">
      <button
        type="button"
        class="crumb"
        onclick={() => {
          showFavorites = false;
          activeSmartFolder = null;
        }}
      >
        {$t("study.library.telegram.crumb_root")}
      </button>
      <span class="crumb-sep" aria-hidden="true">›</span>
      <span class="crumb-current">
        {#if activeSmartFolder}
          {smartFolderIcon(activeSmartFolder)} {smartFolderLabel(activeSmartFolder)}
        {:else}
          ★ {$t("study.library.telegram.favorites")}
        {/if}
      </span>
    </nav>

    {#if filteredBookmarks.length === 0}
      <p class="muted">
        {#if activeSmartFolder}
          {$t("study.library.telegram.smart_empty")}
        {:else}
          {$t("study.library.telegram.favorites_empty")}
        {/if}
      </p>
    {:else}
      <div class="grid">
        {#each filteredBookmarks as b (b.id)}
          <article class="card media-card" data-media={b.mime_type ?? "document"}>
            <div class="thumb">
              {#if b.thumb_b64}
                <img
                  src={`data:image/jpeg;base64,${b.thumb_b64}`}
                  alt={b.title}
                />
              {:else}
                <span class="thumb-icon" aria-hidden="true">
                  {mediaTypeIcon(b.mime_type ?? "document")}
                </span>
              {/if}
            </div>
            <div class="meta">
              <span class="title" title={b.title}>{b.title}</span>
              <span class="sub muted small">
                {#if b.file_size}<span>{formatBytes(b.file_size)}</span>{/if}
              </span>
              <div class="tag-chips">
                {#each b.tags as tag (tag)}
                  <button
                    type="button"
                    class="tag-chip"
                    onclick={() => removeTag(b, tag)}
                    title={$t("study.library.telegram.tag_remove")}
                  >
                    {tag} <span aria-hidden="true">×</span>
                  </button>
                {/each}
                {#if tagInputFor === b.id}
                  <input
                    type="text"
                    class="tag-input"
                    bind:value={tagInputValue}
                    placeholder={$t("study.library.telegram.tag_placeholder")}
                    disabled={tagBusy}
                    onkeydown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        void commitAddTag(b);
                      } else if (e.key === "Escape") {
                        cancelAddTag();
                      }
                    }}
                    onblur={() => commitAddTag(b)}
                  />
                {:else}
                  <button
                    type="button"
                    class="tag-chip add"
                    onclick={() => startAddTag(b.id)}
                    title={$t("study.library.telegram.tag_add")}
                  >
                    + {$t("study.library.telegram.tag_add_short")}
                  </button>
                {/if}
              </div>
              <div class="actions">
                <button
                  type="button"
                  class="action star active"
                  onclick={() => removeBookmarkRow(b)}
                  title={$t("study.library.telegram.unbookmark")}
                  aria-label={$t("study.library.telegram.unbookmark")}
                >
                  ★
                </button>
              </div>
            </div>
          </article>
        {/each}
      </div>
    {/if}
  </div>
{:else if !selectedChat}
  <div class="tg-browser">
    <header class="tg-head">
      <div class="head-row">
        <h2>{$t("study.library.telegram.chats_title")}</h2>
        <div class="head-actions">
          <span
            class="status-pill"
            data-status={displayedConnectionStatus}
            title={statusLabel()}
            aria-label={statusLabel()}
          >
            {#if displayedConnectionStatus === "checking"}
              <svg class="status-spinner" viewBox="0 0 16 16" aria-hidden="true">
                <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-dasharray="9 28" />
              </svg>
            {:else}
              <span class="status-dot" aria-hidden="true"></span>
            {/if}
            <span class="status-text">{statusLabel()}</span>
          </span>
          {#if bandwidthActive}
            <span class="bandwidth-pill" aria-live="polite">
              {#if bandwidthDown > 0}<span class="bw-segment">↓ {fmtRate(bandwidthDown)}</span>{/if}
              {#if bandwidthUp > 0}<span class="bw-segment">↑ {fmtRate(bandwidthUp)}</span>{/if}
            </span>
          {/if}
          {#if floodWaitSecs > 0}
            <span class="flood-wait-pill" aria-live="polite" title={$t("study.library.telegram.flood_wait_hint")}>
              ⏳ {floodWaitSecs}s
            </span>
          {/if}
          <button
            type="button"
            class="icon-btn primary-icon-btn"
            onclick={() => { createFolderName = ""; createFolderOpen = true; }}
            aria-label={$t("study.library.telegram.create_folder")}
            title={$t("study.library.telegram.create_folder")}
          >
            ＋
          </button>
          <button
            type="button"
            class="icon-btn"
            onclick={reloadChats}
            disabled={chatsLoading}
            aria-label={$t("study.common.refresh")}
            title={$t("study.common.refresh")}
          >
            ↻
          </button>
          <button
            type="button"
            class="icon-btn"
            onclick={() => (settingsOpen = !settingsOpen)}
            aria-label={$t("study.library.telegram.settings_title")}
          >
            ⚙
          </button>
        </div>
      </div>
      <p class="muted small">{$t("study.library.telegram.chats_subtitle")}</p>
    </header>

    {#if settingsOpen}
      <section class="settings-panel" aria-label="settings">
        <h3>{$t("study.library.telegram.settings_title")}</h3>
        <dl class="settings-dl">
          <dt>{$t("study.library.telegram.settings_output_dir")}</dt>
          <dd class="mono small">{effectiveOutputDir || "—"}</dd>
          <dd class="output-dir-actions">
            <button type="button" class="ghost-btn small" onclick={pickOutputDir}>
              {$t("study.library.telegram.output_dir_pick")}
            </button>
            {#if outputDirOverride}
              <button type="button" class="ghost-btn small" onclick={resetOutputDir}>
                {$t("study.library.telegram.output_dir_reset")}
              </button>
            {/if}
          </dd>
        </dl>
        <label class="settings-toggle">
          <input
            type="checkbox"
            checked={allowRemoteDelete}
            onchange={toggleAllowRemoteDelete}
          />
          <span>{$t("study.library.telegram.allow_remote_delete")}</span>
          <span class="muted small">
            {$t("study.library.telegram.allow_remote_delete_hint")}
          </span>
        </label>
        <label class="settings-toggle">
          <input
            type="checkbox"
            checked={autoResumeDownloads}
            onchange={toggleAutoResumeDownloads}
          />
          <span>{$t("study.library.telegram.auto_resume_downloads")}</span>
          <span class="muted small">
            {$t("study.library.telegram.auto_resume_downloads_hint")}
          </span>
        </label>
        <div class="settings-num-row">
          <label>
            <span>{$t("study.library.telegram.thumb_cache_limit")}</span>
            <input
              type="number"
              min="50"
              max="5000"
              step="50"
              value={thumbCacheLimitMB}
              onchange={(e) => {
                const v = parseInt((e.target as HTMLInputElement).value);
                if (Number.isFinite(v) && v >= 50 && v <= 5000) {
                  thumbCacheLimitMB = v;
                  void saveSettingPatch({ thumb_cache_limit_mb: v });
                }
              }}
            />
            <span class="muted small">MB</span>
          </label>
          <label>
            <span>{$t("study.library.telegram.bulk_confirm_threshold")}</span>
            <input
              type="number"
              min="1"
              max="500"
              step="1"
              value={bulkConfirmThreshold}
              onchange={(e) => {
                const v = parseInt((e.target as HTMLInputElement).value);
                if (Number.isFinite(v) && v >= 1 && v <= 500) {
                  bulkConfirmThreshold = v;
                  void saveSettingPatch({ bulk_confirm_threshold: v });
                }
              }}
            />
            <span class="muted small">{$t("study.library.telegram.bulk_confirm_threshold_unit")}</span>
          </label>
        </div>
        <div class="settings-actions">
          <button
            type="button"
            class="ghost-btn"
            onclick={pruneCache}
            disabled={cachePruneBusy}
          >
            {cachePruneBusy
              ? $t("study.common.loading")
              : $t("study.library.telegram.settings_prune_cache")}
          </button>
          <button
            type="button"
            class="ghost-btn danger"
            onclick={doLogout}
            disabled={logoutBusy}
          >
            {logoutBusy
              ? $t("study.common.loading")
              : $t("study.library.telegram.settings_logout")}
          </button>
          <button
            type="button"
            class="ghost-btn"
            onclick={() => (settingsOpen = false)}
          >
            {$t("study.common.close")}
          </button>
        </div>
      </section>
    {/if}

    {#if chats.length > 0}
      <div class="chat-tabs" role="tablist" aria-label={$t("study.library.telegram.tabs_label")}>
        {#each [
          { id: "all", label: $t("study.library.telegram.tab_all"), count: filterCounts.all },
          { id: "unread", label: $t("study.library.telegram.tab_unread"), count: filterCounts.unread },
          { id: "private", label: $t("study.library.telegram.tab_private"), count: filterCounts.private },
          { id: "group", label: $t("study.library.telegram.tab_group"), count: filterCounts.group },
          { id: "channel", label: $t("study.library.telegram.tab_channel"), count: filterCounts.channel },
          { id: "folder", label: $t("study.library.telegram.tab_folder"), count: filterCounts.folder },
        ] as tab (tab.id)}
          {#if tab.count > 0 || tab.id === "all"}
            <button
              type="button"
              role="tab"
              class="chat-tab"
              class:active={chatFilter === tab.id}
              aria-selected={chatFilter === tab.id}
              onclick={() => (chatFilter = tab.id as ChatFilter)}
            >
              <span class="chat-tab-label">{tab.label}</span>
              {#if tab.count > 0}
                <span class="chat-tab-count">{tab.count}</span>
              {/if}
            </button>
          {/if}
        {/each}
      </div>
      <div class="filter-row">
        <input
          type="search"
          class="search-input"
          placeholder={$t("study.library.telegram.search_chats")}
          bind:value={chatSearch}
        />
      </div>
    {/if}

    {#if chatsLoading && chats.length === 0}
      <div class="grid">
        {#each Array(6) as _, i (i)}
          <div class="card chat-card skeleton" aria-hidden="true">
            <div class="avatar"><Skeleton block height={48} /></div>
            <div class="meta">
              <Skeleton lines={[70, 40]} height={10} gap={6} />
            </div>
          </div>
        {/each}
      </div>
    {:else if chatsError}
      <p class="error small">{chatsError}</p>
      <button type="button" class="ghost-btn" onclick={reloadChats}>
        {$t("study.common.retry")}
      </button>
    {:else if chats.length === 0}
      <div class="empty-state">
        <svg class="empty-illustration" viewBox="0 0 200 160" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
          <ellipse cx="100" cy="135" rx="70" ry="8" fill="currentColor" opacity="0.06"/>
          <path d="M40 60 Q40 50 50 50 L80 50 L92 38 L150 38 Q160 38 160 48 L160 110 Q160 120 150 120 L50 120 Q40 120 40 110 Z" fill="currentColor" opacity="0.12"/>
          <path d="M40 60 Q40 50 50 50 L80 50 L92 38 L150 38 Q160 38 160 48 L160 110 Q160 120 150 120 L50 120 Q40 120 40 110 Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round" opacity="0.5"/>
          <circle cx="100" cy="80" r="22" fill="currentColor" opacity="0.18"/>
          <path d="M100 70 v20 M90 80 h20" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"/>
        </svg>
        <h3>{$t("study.library.telegram.chats_empty")}</h3>
        <p class="muted small">{$t("study.library.telegram.chats_empty_hint")}</p>
        <button
          type="button"
          class="bar-btn primary"
          onclick={() => { createFolderName = ""; createFolderOpen = true; }}
        >
          ＋ {$t("study.library.telegram.create_folder")}
        </button>
      </div>
    {:else}
      <div class="grid">
        {#if savedMessagesChat}
          <button
            type="button"
            class="card chat-card saved-card ripple"
            onclick={() => selectChat(savedMessagesChat)}
            use:ripplePointer
          >
            <div class="avatar saved">
              <TgIcon name="saved" size={22} />
            </div>
            <div class="meta">
              <span class="title">{$t("study.library.telegram.saved_messages")}</span>
              <span class="kind muted small">{$t("study.library.telegram.saved_messages_hint")}</span>
            </div>
          </button>
        {/if}

        <button
          type="button"
          class="card chat-card favorites-card ripple"
          onclick={openFavorites}
          use:ripplePointer
        >
          <div class="avatar fav">
            <TgIcon name="star" size={22} />
          </div>
          <div class="meta">
            <span class="title">{$t("study.library.telegram.favorites")}</span>
            <span class="kind muted small">
              {bookmarks.length} {$t("study.library.telegram.favorites_count_suffix")}
            </span>
          </div>
        </button>

        {#each SMART_FOLDERS as sf (sf)}
          {@const count = smartFolderCount(sf)}
          {#if count > 0}
            <button
              type="button"
              class="card chat-card smart-card ripple"
              onclick={() => openSmartFolder(sf)}
              use:ripplePointer
            >
              <div class="avatar smart">
                <TgIcon name={smartFolderIconName(sf)} size={22} />
              </div>
              <div class="meta">
                <span class="title">{smartFolderLabel(sf)}</span>
                <span class="kind muted small">
                  {count} {$t("study.library.telegram.favorites_count_suffix")}
                </span>
              </div>
            </button>
          {/if}
        {/each}

        {#each (visibleChats.length > 200 ? visibleChats.slice(0, 200) : visibleChats) as c (chatKey(c))}
          {@const grad = avatarGradient(c.id)}
          {@const ts = dialogTimestamp(c.last_message_date)}
          {@const ub = unreadBadgeText(c.unread_count ?? 0)}
          {#if firstUnpinnedKey === chatKey(c)}
            <div class="pin-separator" aria-hidden="true"></div>
          {/if}
          <button
            type="button"
            class="card chat-card ripple"
            class:muted-chat={c.is_muted}
            class:pinned-chat={c.is_pinned}
            class:last-visited={lastVisitedChatKey === chatKey(c)}
            onclick={() => selectChat(c)}
            oncontextmenu={(e) => openChatContext(e, c)}
            use:lazyPhoto={c}
            use:ripplePointer
          >
            <div
              class="avatar"
              class:has-photo={chatPhotos.has(chatKey(c))}
              style:--avatar-bg-1={grad[0]}
              style:--avatar-bg-2={grad[1]}
            >
              {#if chatPhotos.get(chatKey(c))}
                <img src={`data:image/jpeg;base64,${chatPhotos.get(chatKey(c))}`} alt={c.title} />
              {:else}
                <span class="initial" aria-hidden="true">{chatInitial(c.title)}</span>
              {/if}
              {#if c.is_online && c.chat_type === "private"}
                <span class="online-dot" aria-label={$t("study.library.telegram.online")}></span>
              {/if}
            </div>
            <div class="row-meta">
              <div class="row-top">
                <span class="row-name">
                  {c.title}
                  {#if c.is_verified}
                    <span class="verified-mark" aria-label={$t("study.library.telegram.verified")} title={$t("study.library.telegram.verified")}>✓</span>
                  {/if}
                </span>
                {#if ts}
                  <span class="row-date" class:row-date-active={ub}>{ts}</span>
                {/if}
              </div>
              <div class="row-bot">
                <span class="row-preview">
                  {c.last_message ?? chatTypeLabel(c.chat_type)}
                </span>
                <span class="row-badges">
                  {#if c.is_muted}
                    <span class="row-mute" aria-label={$t("study.library.telegram.muted")} title={$t("study.library.telegram.muted")}>🔇</span>
                  {/if}
                  {#if c.is_pinned && !ub}
                    <span class="row-pin" aria-label={$t("study.library.telegram.pinned")} title={$t("study.library.telegram.pinned")}>📌</span>
                  {/if}
                  {#if ub}
                    <span class="unread-badge" class:muted={c.is_muted}>{ub}</span>
                  {/if}
                </span>
              </div>
            </div>
          </button>
        {/each}
        {#if visibleChats.length > 200}
          <p class="muted small list-trim-hint">
            {$t("study.library.telegram.list_trimmed_hint", { shown: 200, total: visibleChats.length })}
          </p>
        {/if}
      </div>

      {#if visibleChats.length === 0 && chatSearch.trim()}
        <p class="muted">{$t("study.library.telegram.search_chats_empty")}</p>
      {/if}

      {#if chatsHasMore && !chatSearch.trim()}
        <div class="load-more-row">
          <button
            type="button"
            class="ghost-btn"
            disabled={chatsLoadingMore}
            onclick={loadMoreChats}
          >
            {chatsLoadingMore
              ? $t("study.common.loading")
              : $t("study.library.telegram.load_more_chats")}
          </button>
        </div>
      {/if}

      {#if chatsRefreshing && !chatsLoading}
        <p class="muted small refresh-hint">
          {$t("study.library.telegram.refreshing_in_background")}
        </p>
      {/if}
    {/if}
  </div>
{:else}
  <div class="tg-2col-shell">
    <aside class="tg-mini-list" aria-label="chat list quick switch">
      <div class="tg-mini-list-header">
        <button
          type="button"
          class="ghost-btn small"
          onclick={leaveChat}
          title={$t("study.library.telegram.crumb_root")}
        >
          ← {$t("study.library.telegram.crumb_root")}
        </button>
      </div>
      <div class="tg-mini-list-scroll">
        {#each visibleChats.slice(0, 80) as c (chatKey(c))}
          {@const grad = avatarGradient(c.id)}
          {@const ub = unreadBadgeText(c.unread_count ?? 0)}
          <button
            type="button"
            class="tg-mini-row"
            class:active={selectedChat && c.id === selectedChat.id && c.chat_type === selectedChat.chat_type}
            onclick={() => selectChat(c)}
            title={c.title}
          >
            <div
              class="avatar mini"
              class:has-photo={chatPhotos.has(chatKey(c))}
              style:--avatar-bg-1={grad[0]}
              style:--avatar-bg-2={grad[1]}
            >
              {#if chatPhotos.get(chatKey(c))}
                <img src={`data:image/jpeg;base64,${chatPhotos.get(chatKey(c))}`} alt={c.title} />
              {:else}
                <span class="initial" aria-hidden="true">{chatInitial(c.title)}</span>
              {/if}
              {#if c.is_online && c.chat_type === "private"}
                <span class="online-dot online-dot-mini" aria-hidden="true"></span>
              {/if}
            </div>
            <span class="tg-mini-name">{c.title}</span>
            {#if ub}
              <span class="unread-badge mini" class:muted={c.is_muted}>{ub}</span>
            {/if}
          </button>
        {/each}
      </div>
    </aside>
    <ResizeBar storageKey="study.library.tg.miniListWidth" min={240} max={400} defaultWidth={280} cssVar="--mini-list-width" />
    <div
      class="tg-browser tg-pane-content"
      in:fly={{ x: 24, duration: 200, easing: cubicOut, opacity: 0 }}
      out:fly={{ x: 24, duration: 160, easing: cubicOut, opacity: 0 }}
    >
    <nav class="crumbs" aria-label="breadcrumb">
      <button type="button" class="crumb" onclick={leaveChat}>
        {$t("study.library.telegram.crumb_root")}
      </button>
      <span class="crumb-sep" aria-hidden="true">›</span>
      <span class="crumb-current">{selectedChat.title}</span>
      {#if mediaSearchServer && mediaSearch.trim().length > 0}
        <span class="crumb-sep" aria-hidden="true">›</span>
        <button
          type="button"
          class="crumb crumb-search"
          onclick={() => { mediaSearch = ""; mediaSearchServer = false; void loadMedia(true); }}
          title={$t("study.library.telegram.crumb_clear_search")}
        >
          🔍 "{mediaSearch.trim()}"
          <span class="crumb-search-clear" aria-hidden="true">✕</span>
        </button>
      {/if}
      <div class="crumb-spacer"></div>
      {#if isOmnigetFolder(selectedChat)}
        <button
          type="button"
          class="ghost-btn small"
          onclick={openRename}
          title={$t("study.library.telegram.folder_rename")}
        >
          ✎ {$t("study.library.telegram.folder_rename")}
        </button>
        <button
          type="button"
          class="ghost-btn small danger"
          onclick={() => (deleteFolderConfirmOpen = true)}
          title={$t("study.library.telegram.folder_delete")}
        >
          🗑 {$t("study.library.telegram.folder_delete")}
        </button>
      {/if}
      <button
        type="button"
        class="ghost-btn small upload-btn"
        onclick={pickAndUpload}
        disabled={uploadBusy}
        title={$t("study.library.telegram.upload_hint")}
      >
        ↑ {uploadBusy ? $t("study.common.loading") : $t("study.library.telegram.upload_files")}
      </button>
    </nav>

    <div class="filter-row">
      <button
        type="button"
        class="chip"
        class:active={mediaFilter === "all"}
        onclick={() => setFilter("all")}
      >
        {$t("study.library.telegram.filter_all")}
      </button>
      {#each MEDIA_TYPES as mt (mt)}
        <button
          type="button"
          class="chip"
          class:active={mediaFilter === mt}
          onclick={() => setFilter(mt)}
          title={$t(`study.library.telegram.filter_${mt}`)}
        >
          <TgIcon name={MEDIA_TYPE_ICONS[mt]} size={14} />
          <span>{$t(`study.library.telegram.filter_${mt}`)}</span>
        </button>
      {/each}
    </div>

    <div class="filter-row">
      <input
        type="search"
        class="search-input"
        placeholder={$t("study.library.telegram.search_media")}
        bind:value={mediaSearch}
        onkeydown={(e) => {
          if (e.key === "Enter") void searchOnServer();
        }}
      />
      {#if mediaSearchServer}
        <button type="button" class="ghost-btn small" onclick={clearSearchServer}>
          {$t("study.library.telegram.clear_server_search")}
        </button>
      {:else if mediaSearch.trim().length >= 2}
        <button type="button" class="ghost-btn small" onclick={searchOnServer}>
          {$t("study.library.telegram.search_server")}
        </button>
      {/if}
    </div>

    {#if mediaError}
      <p class="error small">{mediaError}</p>
    {/if}

    {#if mediaLoading && mediaItems.length === 0}
      <div class="grid">
        {#each Array(8) as _, i (i)}
          <div class="card media-card skeleton" aria-hidden="true">
            <div class="thumb"><Skeleton block height={150} /></div>
            <div class="meta">
              <Skeleton lines={[80, 50]} height={10} gap={6} />
            </div>
          </div>
        {/each}
      </div>
    {:else if mediaItems.length === 0}
      <p class="muted">{$t("study.library.telegram.chat_empty")}</p>
    {:else if visibleMediaItems.length === 0}
      <p class="muted">{$t("study.library.telegram.search_media_empty")}</p>
    {:else}
      <div class="grid grid-with-dividers">
        {#each visibleMediaItems as it, idx (it.message_id)}
          {@const pb = playbackByMessage.get(it.message_id)}
          {@const prev = idx > 0 ? visibleMediaItems[idx - 1] : null}
          {@const curBucket = dateBucket(it.date)}
          {@const showDivider = !prev || dateBucket(prev.date).key !== curBucket.key}
          {#if showDivider}
            <div class="date-divider" aria-hidden="true">
              <span class="date-divider-pill">{curBucket.label}</span>
            </div>
          {/if}
          {@const variant = cardVariantForMedia(it)}
          <div
            class="card media-card ripple"
            class:selected={selectedIds.has(it.message_id)}
            class:variant-voice={variant === "voice"}
            class:variant-round={variant === "round"}
            class:variant-gif={variant === "gif"}
            data-media={it.media_type}
            data-variant={variant}
            role="button"
            tabindex="0"
            use:ripplePointer
            use:longPressSelect={it}
            onclick={(ev) => toggleSelect(it, idx, ev)}
            onkeydown={(ev) => {
              if (ev.key === " " || ev.key === "Enter") {
                toggleSelect(it, idx, ev as unknown as MouseEvent);
              }
            }}
            onmouseenter={() => { if (variant === "gif") hoveringGifId = it.message_id; }}
            onmouseleave={() => { if (hoveringGifId === it.message_id) hoveringGifId = null; }}
          >
            {#if selectedIds.has(it.message_id)}
              <span class="select-mark" aria-hidden="true">✓</span>
            {/if}
            {#if pb && pb.finished}
              <span class="watched-mark" aria-hidden="true" title={$t("study.library.telegram.watched")}>✓</span>
            {:else if pb && pb.position > 5}
              <span class="resume-badge" title={$t("study.library.telegram.resume_at", { time: fmtTime(pb.position) })}>
                ▶ {fmtTime(pb.position)}
              </span>
            {/if}
            {#if variant === "voice"}
              <div class="thumb voice-thumb" use:lazyMediaThumb={it}>
                <div class="voice-waveform" aria-hidden="true">
                  {#each voiceWaveformBars(it.message_id) as h, bi (bi)}
                    <span class="bar" style:height="{h}%"></span>
                  {/each}
                </div>
                <span class="voice-duration">{approxDuration(it.file_size)}</span>
                <span class="voice-play" aria-hidden="true">
                  <TgIcon name="play" size={16} />
                </span>
              </div>
            {:else if variant === "round"}
              <div class="thumb round-thumb" use:lazyMediaThumb={it}>
                {#if mediaThumbs.get(it.message_id)}
                  <img
                    class="thumb-img round-img"
                    src={`data:image/jpeg;base64,${mediaThumbs.get(it.message_id)}`}
                    alt={it.file_name}
                    loading="lazy"
                    decoding="async"
                  />
                {:else}
                  <span class="thumb-shimmer" aria-hidden="true"></span>
                {/if}
                <span class="round-play-overlay" aria-hidden="true">
                  <TgIcon name="play" size={22} />
                </span>
                <span class="round-duration">{approxDuration(it.file_size)}</span>
              </div>
            {:else if variant === "gif"}
              <div class="thumb gif-thumb" use:lazyMediaThumb={it}>
                {#if mediaThumbs.get(it.message_id)}
                  <img
                    class="thumb-img"
                    src={`data:image/jpeg;base64,${mediaThumbs.get(it.message_id)}`}
                    alt={it.file_name}
                    loading="lazy"
                    decoding="async"
                  />
                {:else}
                  <span class="thumb-shimmer" aria-hidden="true"></span>
                {/if}
                <span class="thumb-gif-badge">GIF</span>
                {#if hoveringGifId === it.message_id && !reducedMotion}
                  <span class="gif-playing-indicator" aria-hidden="true">▶ playing</span>
                {/if}
              </div>
            {:else}
            <div class="thumb" class:webpage-thumb={it.media_type === "webpage"} use:lazyMediaThumb={it}>
              {#if mediaThumbs.get(it.message_id)}
                <img
                  class="thumb-img"
                  src={`data:image/jpeg;base64,${mediaThumbs.get(it.message_id)}`}
                  alt={it.file_name}
                  loading="lazy"
                  decoding="async"
                />
              {:else if isThumbnailable(it)}
                <span class="thumb-shimmer" aria-hidden="true"></span>
                <span class="thumb-icon ghost" aria-hidden="true">
                  {mediaTypeIcon(it.media_type)}
                </span>
              {:else if it.media_type === "webpage"}
                <span class="webpage-globe" aria-hidden="true">
                  <TgIcon name="globe" size={28} />
                </span>
              {:else}
                <span class="thumb-icon" aria-hidden="true">
                  {mediaTypeIcon(it.media_type)}
                </span>
              {/if}
              {#if it.media_type === "video"}
                <span class="thumb-play-overlay" aria-hidden="true">
                  <TgIcon name="play" size={20} />
                </span>
                {#if it.file_size > 0 && it.file_size < 5 * 1024 * 1024}
                  <span class="thumb-gif-badge">GIF</span>
                {/if}
              {/if}
              {#if it.media_type === "webpage" && it.webpage?.site_name}
                <span class="webpage-site-badge">{it.webpage.site_name}</span>
              {/if}
              {#if it.grouped_id}
                <button
                  type="button"
                  class="album-badge"
                  title={$t("study.library.telegram.album_indicator", { n: 0 })}
                  onclick={(ev) => {
                    ev.stopPropagation();
                    if (selectedChat) {
                      albumModal = {
                        chatId: selectedChat.id,
                        chatType: selectedChat.chat_type,
                        messageId: it.message_id,
                      };
                    }
                  }}
                >
                  📚
                </button>
              {/if}
            </div>
            {/if}
            <div class="meta">
              {#if editingMessageId === it.message_id}
                <input
                  type="text"
                  class="title-input"
                  bind:value={editingValue}
                  disabled={editingBusy}
                  onkeydown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void commitEditTitle(it);
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      cancelEditTitle();
                    }
                  }}
                  onblur={() => void commitEditTitle(it)}
                  onclick={(e) => e.stopPropagation()}
                />
              {:else}
                <span
                  class="title"
                  role="textbox"
                  tabindex="0"
                  title={$t("study.library.telegram.title_edit_hint")}
                  ondblclick={(e) => startEditTitle(it, e)}
                  onkeydown={(e) => {
                    if (e.key === "F2") {
                      e.stopPropagation();
                      startEditTitle(it, e as unknown as MouseEvent);
                    }
                  }}
                >
                  {it.file_name}
                </span>
              {/if}
              {#if it.caption}
                <span class="caption" title={it.caption}>{it.caption}</span>
              {/if}
              {#if it.media_type === "webpage" && it.webpage}
                {#if it.webpage.description}
                  <span class="webpage-desc" title={it.webpage.description}>
                    {it.webpage.description}
                  </span>
                {/if}
                {#if it.webpage.url}
                  <a
                    href={it.webpage.url}
                    class="webpage-url muted small"
                    target="_blank"
                    rel="noreferrer noopener"
                    onclick={(e) => e.stopPropagation()}
                    title={it.webpage.url}
                  >
                    🔗 {(() => { try { return new URL(it.webpage.url).hostname; } catch { return it.webpage.url; } })()}
                  </a>
                {/if}
              {/if}
              <span class="sub muted small">
                {#if it.file_size}<span>{formatBytes(it.file_size)}</span>{/if}
                {#if it.file_size && it.date}<span aria-hidden="true">·</span>{/if}
                {#if it.date}<span>{formatDate(it.date)}</span>{/if}
              </span>
              <div class="actions">
                <button
                  type="button"
                  class="action star"
                  class:active={selectedChat && bookmarkKeys.has(bookmarkKey(selectedChat.id, it.message_id))}
                  onclick={(e) => { e.stopPropagation(); toggleBookmark(it); }}
                  title={selectedChat && bookmarkKeys.has(bookmarkKey(selectedChat.id, it.message_id))
                    ? $t("study.library.telegram.unbookmark")
                    : $t("study.library.telegram.bookmark")}
                  aria-label={$t("study.library.telegram.bookmark")}
                >
                  ★
                </button>
                <button
                  type="button"
                  class="action"
                  disabled={pendingDownloads.has(it.message_id)}
                  onclick={(e) => { e.stopPropagation(); downloadItem(it); }}
                  title={$t("study.library.telegram.downloads.download")}
                  aria-label={$t("study.library.telegram.downloads.download")}
                >
                  {pendingDownloads.has(it.message_id) ? "…" : "↓"}
                </button>
              </div>
            </div>
          </div>
        {/each}
      </div>

      {#if mediaHasMore && !mediaSearchServer}
        <div class="load-more-row">
          <button
            type="button"
            class="ghost-btn"
            disabled={mediaLoading}
            onclick={() => loadMedia(false)}
          >
            {mediaLoading
              ? $t("study.common.loading")
              : $t("study.library.telegram.load_more")}
          </button>
        </div>
      {/if}
    {/if}

    {#if playerOpen && selectedChat}
      <TelegramMediaPlayer
        chat={selectedChat}
        playlist={visibleMediaItems}
        initialIdx={playerIdx}
        onClose={closePlayer}
        onDownload={(it) => downloadItem(it)}
        originRect={playerOriginRect}
      />
    {/if}

    {#if albumModal}
      <AlbumCarouselModal
        chatId={albumModal.chatId}
        chatType={albumModal.chatType as "private" | "group" | "channel"}
        messageId={albumModal.messageId}
        onClose={() => (albumModal = null)}
      />
    {/if}

    {#if dragHover && selectedChat}
      <div class="drop-overlay" aria-live="polite">
        <div class="drop-card">
          <div class="drop-icon">↑</div>
          <h3>{$t("study.library.telegram.drop_to_upload_title")}</h3>
          <p class="muted small">
            {$t("study.library.telegram.drop_to_upload_hint", { chat: selectedChat.title })}
          </p>
        </div>
      </div>
    {/if}

    {#if selectedIds.size > 0}
      <div class="selection-bar" role="region" aria-label="bulk actions">
        <span class="count mono">
          {$t("study.library.telegram.selected", { n: selectedIds.size })}
        </span>
        <button
          type="button"
          class="bar-btn"
          onclick={selectAllVisible}
          disabled={bulkBusy}
        >
          {$t("study.library.telegram.select_all_visible")}
        </button>
        <button
          type="button"
          class="bar-btn primary"
          onclick={bulkDownload}
          disabled={bulkBusy}
        >
          ↓ {$t("study.library.telegram.bulk_download")}
        </button>
        <button
          type="button"
          class="bar-btn"
          onclick={bulkBookmark}
          disabled={bulkBusy}
        >
          ★ {$t("study.library.telegram.bulk_bookmark")}
        </button>
        <button
          type="button"
          class="bar-btn"
          onclick={() => openForwardModal("copy")}
          disabled={bulkBusy}
        >
          ⎘ {$t("study.library.telegram.bulk_copy")}
        </button>
        <button
          type="button"
          class="bar-btn"
          onclick={() => openForwardModal("move")}
          disabled={bulkBusy || !allowRemoteDelete}
          title={!allowRemoteDelete
            ? $t("study.library.telegram.move_needs_delete_perm")
            : undefined}
        >
          → {$t("study.library.telegram.bulk_move")}
        </button>
        {#if allowRemoteDelete}
          <button
            type="button"
            class="bar-btn danger"
            onclick={() => (deleteConfirmOpen = true)}
            disabled={bulkBusy}
          >
            🗑 {$t("study.library.telegram.bulk_delete")}
          </button>
        {/if}
        <button
          type="button"
          class="bar-btn ghost"
          onclick={clearSelection}
          disabled={bulkBusy}
          aria-label={$t("study.library.telegram.clear_selection")}
        >
          ✕
        </button>
      </div>
    {/if}

    <ConfirmDialog
      bind:open={deleteConfirmOpen}
      variant="danger"
      title={$t("study.library.telegram.delete_confirm_title")}
      message={$t("study.library.telegram.delete_confirm_body", {
        n: selectedIds.size,
      })}
      confirmLabel={$t("study.library.telegram.bulk_delete")}
      onConfirm={() => void bulkDelete()}
    />

    {#if forwardModalOpen}
      <div
        class="modal-overlay"
        role="presentation"
        onclick={() => (forwardModalOpen = false)}
      ></div>
      <div class="modal" role="dialog" aria-modal="true">
        <header class="modal-head">
          <h3>
            {forwardModalMode === "move"
              ? $t("study.library.telegram.move_to")
              : $t("study.library.telegram.copy_to")}
          </h3>
          <button
            type="button"
            class="ghost-btn"
            onclick={() => (forwardModalOpen = false)}
            aria-label={$t("study.common.close")}
          >
            ✕
          </button>
        </header>
        <input
          type="search"
          class="search-input"
          placeholder={$t("study.library.telegram.search_chats")}
          bind:value={forwardSearch}
        />
        <div class="modal-list">
          {#each forwardCandidates as c (chatKey(c))}
            <button
              type="button"
              class="modal-row"
              disabled={forwardBusy}
              onclick={() => void commitForward(c)}
            >
              <div
                class="avatar small"
                class:has-photo={chatPhotos.has(chatKey(c))}
                style:--avatar-bg-1={avatarGradient(c.id)[0]}
                style:--avatar-bg-2={avatarGradient(c.id)[1]}
              >
                {#if chatPhotos.get(chatKey(c))}
                  <img src={`data:image/jpeg;base64,${chatPhotos.get(chatKey(c))}`} alt={c.title} />
                {:else}
                  <span class="initial" aria-hidden="true">{chatInitial(c.title)}</span>
                {/if}
              </div>
              <span class="title">{c.title}</span>
              <span class="kind muted small">{chatTypeLabel(c.chat_type)}</span>
            </button>
          {/each}
          {#if forwardCandidates.length === 0}
            <p class="muted small">{$t("study.library.telegram.search_chats_empty")}</p>
          {/if}
        </div>
      </div>
    {/if}
    </div>
  </div>
{/if}

{#if renameOpen}
  <div
    class="modal-overlay"
    role="presentation"
    onclick={() => (renameOpen = false)}
  ></div>
  <div class="modal" role="dialog" aria-modal="true">
    <header class="modal-head">
      <h3>{$t("study.library.telegram.folder_rename_title")}</h3>
      <button
        type="button"
        class="ghost-btn"
        onclick={() => (renameOpen = false)}
        aria-label={$t("study.common.close")}
      >
        ✕
      </button>
    </header>
    <input
      type="text"
      class="search-input"
      placeholder={$t("study.library.telegram.create_folder_placeholder")}
      bind:value={renameValue}
      disabled={renameBusy}
      onkeydown={(e) => {
        if (e.key === "Enter") { e.preventDefault(); void commitRename(); }
        else if (e.key === "Escape") { renameOpen = false; }
      }}
    />
    <div class="settings-actions">
      <button type="button" class="ghost-btn" onclick={() => (renameOpen = false)}>
        {$t("study.common.cancel")}
      </button>
      <button
        type="button"
        class="bar-btn primary"
        onclick={() => void commitRename()}
        disabled={renameBusy || !renameValue.trim()}
      >
        {renameBusy ? $t("study.common.loading") : $t("study.library.telegram.folder_rename")}
      </button>
    </div>
  </div>
{/if}

<ConfirmDialog
  bind:open={deleteFolderConfirmOpen}
  variant="danger"
  title={$t("study.library.telegram.folder_delete_title")}
  message={$t("study.library.telegram.folder_delete_body")}
  confirmLabel={$t("study.library.telegram.folder_delete")}
  onConfirm={() => void commitDeleteFolder()}
/>

{#if createFolderOpen}
  <div
    class="modal-overlay"
    role="presentation"
    onclick={() => (createFolderOpen = false)}
  ></div>
  <div class="modal" role="dialog" aria-modal="true">
    <header class="modal-head">
      <h3>{$t("study.library.telegram.create_folder_title")}</h3>
      <button
        type="button"
        class="ghost-btn"
        onclick={() => (createFolderOpen = false)}
        aria-label={$t("study.common.close")}
      >
        ✕
      </button>
    </header>
    <p class="muted small">{$t("study.library.telegram.create_folder_hint")}</p>
    <input
      type="text"
      class="search-input"
      placeholder={$t("study.library.telegram.create_folder_placeholder")}
      bind:value={createFolderName}
      disabled={createFolderBusy}
      onkeydown={(e) => {
        if (e.key === "Enter") { e.preventDefault(); void commitCreateFolder(); }
        else if (e.key === "Escape") { createFolderOpen = false; }
      }}
    />
    <div class="settings-actions">
      <button type="button" class="ghost-btn" onclick={() => (createFolderOpen = false)}>
        {$t("study.common.cancel")}
      </button>
      <button
        type="button"
        class="bar-btn primary"
        onclick={() => void commitCreateFolder()}
        disabled={createFolderBusy || !createFolderName.trim()}
      >
        {createFolderBusy ? $t("study.common.loading") : $t("study.library.telegram.create_folder")}
      </button>
    </div>
  </div>
{/if}

{#if keymapOpen}
  <KeymapHint
    context={playerOpen ? "player" : selectedChat ? "media-grid" : "chat-list"}
    onClose={() => (keymapOpen = false)}
  />
{/if}

{#if chatCtxMenu}
  <ChatContextMenu
    x={chatCtxMenu.x}
    y={chatCtxMenu.y}
    items={chatCtxMenu.items}
    onClose={closeChatContext}
  />
{/if}

<style>
  .tg-browser {
    --tg-radius-card: 12px;
    --tg-radius-pill: 999px;
    --tg-radius-input: 10px;
    --tg-shadow-card: 0 1px 2px rgba(0, 0, 0, 0.04), 0 4px 12px rgba(0, 0, 0, 0.04);
    --tg-shadow-card-hover: 0 4px 12px rgba(0, 0, 0, 0.08), 0 12px 32px rgba(0, 0, 0, 0.08);
    --tg-card-bg: var(--surface);
    --tg-card-border: var(--content-border);
    --tg-accent-bg: color-mix(in oklab, var(--accent) 12%, transparent);
    --tg-accent-bg-hover: color-mix(in oklab, var(--accent) 20%, transparent);
    --tg-accent-border: color-mix(in oklab, var(--accent) 35%, var(--content-border));
    /* tdesktop-aligned motion tokens (chat.style + animations.h) */
    --tg-duration-fast: 150ms;     /* hover, ripple, online dot, slideDuration */
    --tg-duration-normal: 200ms;   /* fade in/out, msgFileOver, dateFade */
    --tg-duration-slow: 320ms;     /* zoom thumb hover, blur-to-sharp */
    --tg-duration-reveal: 700ms;   /* sineDuration */
    --tg-easing-default: cubic-bezier(0.22, 1, 0.36, 1);
    --tg-easing-bounce: cubic-bezier(0.34, 1.56, 0.64, 1);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
    padding: var(--padding) 0;
  }
  .tg-2col-shell {
    display: grid;
    grid-template-columns: var(--mini-list-width, 280px) auto minmax(0, 1fr);
    gap: 0;
    align-items: stretch;
    min-height: calc(100vh - 140px);
  }
  .tg-pane-content {
    min-width: 0;
    padding-left: var(--padding);
  }
  .tg-mini-list {
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--content-border);
    background: color-mix(in oklab, var(--surface) 60%, transparent);
    max-height: calc(100vh - 140px);
    position: sticky;
    top: 0;
    align-self: start;
  }
  .tg-mini-list-header {
    padding: 8px 10px;
    border-bottom: 1px solid var(--content-border);
    flex-shrink: 0;
  }
  .tg-mini-list-scroll {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 4px 0;
  }
  .tg-mini-row {
    display: grid;
    grid-template-columns: 36px minmax(0, 1fr) auto;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 6px 10px;
    background: transparent;
    border: 0;
    cursor: pointer;
    color: inherit;
    text-align: left;
    border-radius: 8px;
    margin: 1px 4px;
    transition: background var(--tg-duration-fast) var(--tg-easing-default);
  }
  .tg-mini-row:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .tg-mini-row.active {
    background: var(--accent);
    color: white;
  }
  .tg-mini-row.active .tg-mini-name {
    color: white;
    font-weight: 600;
  }
  .tg-mini-row .avatar.mini {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    overflow: hidden;
    position: relative;
    background: linear-gradient(135deg, var(--avatar-bg-1, #6cb5f9), var(--avatar-bg-2, #4a8dde));
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .tg-mini-row .avatar.mini img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .tg-mini-row .avatar.mini .initial {
    color: white;
    font-weight: 600;
    font-size: 14px;
    text-transform: uppercase;
    line-height: 1;
  }
  .online-dot-mini {
    position: absolute;
    bottom: 0;
    right: 0;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #4fb24f;
    box-shadow: 0 0 0 2px var(--surface);
  }
  .tg-mini-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .unread-badge.mini {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 9px;
    background: var(--accent);
    color: white;
    font-size: 10px;
    font-weight: 700;
    flex-shrink: 0;
  }
  .unread-badge.mini.muted {
    background: color-mix(in oklab, var(--secondary) 40%, transparent);
  }
  .tg-mini-row.active .unread-badge.mini {
    background: white;
    color: var(--accent);
  }
  @media (max-width: 900px) {
    .tg-2col-shell {
      grid-template-columns: 1fr;
    }
    .tg-mini-list {
      display: none;
    }
    .tg-pane-content {
      padding-left: 0;
    }
  }
  .tg-head h2 {
    margin: 0;
    font-size: 1.4rem;
    font-weight: 700;
    color: var(--secondary);
    letter-spacing: -0.02em;
  }
  .tg-head p {
    margin: 0.25rem 0 0;
    font-size: 13px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 14px;
  }
  .grid-with-dividers {
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    align-content: start;
  }
  .date-divider {
    grid-column: 1 / -1;
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    justify-content: center;
    pointer-events: none;
    margin: 4px 0 -4px;
    animation: date-divider-fade 200ms ease;
  }
  .date-divider-pill {
    display: inline-flex;
    align-items: center;
    height: 28px;
    padding: 0 14px;
    border-radius: 14px;
    background: color-mix(in oklab, var(--surface) 80%, transparent);
    color: var(--secondary);
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.01em;
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.08), 0 0 0 1px var(--content-border);
  }
  @keyframes date-divider-fade {
    from { opacity: 0; transform: translateY(-4px); }
    to   { opacity: 1; transform: translateY(0); }
  }
  @media (prefers-reduced-motion: reduce) {
    .date-divider { animation: none; }
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    background: var(--tg-card-bg);
    border: 1px solid var(--tg-card-border);
    border-radius: var(--tg-radius-card);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    box-shadow: var(--tg-shadow-card);
    transition:
      border-color var(--tg-duration-fast, 150ms) var(--tg-easing-default, ease),
      transform var(--tg-duration-fast, 150ms) var(--tg-easing-default, cubic-bezier(0.22, 1, 0.36, 1)),
      box-shadow var(--tg-duration-fast, 150ms) var(--tg-easing-default, ease),
      background var(--tg-duration-fast, 150ms) var(--tg-easing-default, ease);
  }
  .card:hover {
    border-color: var(--tg-accent-border);
    transform: translateY(-2px);
    box-shadow: var(--tg-shadow-card-hover);
  }
  .card:active {
    transform: translateY(0);
    transition-duration: var(--tg-duration-fast, 150ms);
  }
  .chat-card {
    flex-direction: row;
    align-items: center;
    gap: 12px;
    min-height: 62px;
    padding: 8px 12px;
  }
  .row-meta {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }
  .row-top {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .row-name {
    flex: 1;
    min-width: 0;
    font-weight: 500;
    font-size: 14px;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .verified-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    margin-left: 4px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 9px;
    font-weight: 700;
    vertical-align: middle;
  }
  .row-date {
    flex-shrink: 0;
    font-size: 11px;
    color: var(--tertiary);
    font-variant-numeric: tabular-nums;
  }
  .row-date-active {
    color: var(--accent);
    font-weight: 600;
  }
  .row-bot {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .row-preview {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    color: var(--tertiary);
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-badges {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .row-mute,
  .row-pin {
    font-size: 12px;
    opacity: 0.7;
  }
  .unread-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 19px;
    min-width: 19px;
    padding: 0 6px;
    border-radius: 10px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 12px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .unread-badge.muted {
    background: color-mix(in oklab, var(--tertiary) 60%, transparent);
  }
  .muted-chat .row-name {
    opacity: 0.85;
  }
  .chat-card.last-visited {
    border-color: color-mix(in oklab, var(--accent) 35%, var(--content-border));
    background: color-mix(in oklab, var(--accent) 5%, var(--surface));
  }
  .chat-card.last-visited::before {
    content: "";
    position: absolute;
    left: 0;
    top: 12px;
    bottom: 12px;
    width: 3px;
    background: var(--accent);
    border-radius: 0 2px 2px 0;
  }
  .pin-separator {
    grid-column: 1 / -1;
    height: 1px;
    background: var(--content-border);
    margin: 6px 14px;
    opacity: 0.7;
  }
  .pinned-chat::after {
    content: "📌";
    position: absolute;
    bottom: 8px;
    right: 10px;
    font-size: 10px;
    opacity: 0.55;
    pointer-events: none;
  }
  .avatar {
    --avatar-bg-1: var(--accent);
    --avatar-bg-2: var(--accent);
    width: 46px;
    height: 46px;
    border-radius: 50%;
    overflow: hidden;
    background: linear-gradient(180deg, var(--avatar-bg-1), var(--avatar-bg-2));
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
    position: relative;
    transition: transform var(--tg-duration-normal, 200ms) var(--tg-easing-default, cubic-bezier(0.22, 1, 0.36, 1));
  }
  .avatar.has-photo {
    background: var(--surface);
  }
  .chat-card:hover .avatar {
    transform: scale(1.04);
  }
  .avatar img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    animation: avatar-fade-in 240ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .online-dot {
    position: absolute;
    bottom: 0;
    right: 0;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #4dc920;
    box-shadow: 0 0 0 2px var(--surface);
    animation: online-fade-in 150ms ease;
  }
  @keyframes online-fade-in {
    from { opacity: 0; transform: scale(0.5); }
    to   { opacity: 1; transform: scale(1); }
  }
  @media (prefers-reduced-motion: reduce) {
    .online-dot { animation: none; }
  }
  @keyframes avatar-fade-in {
    from { opacity: 0; transform: scale(0.96); }
    to   { opacity: 1; transform: scale(1); }
  }
  .initial {
    font-weight: 600;
    color: #fff;
    font-size: 1.25rem;
    letter-spacing: -0.02em;
    text-shadow: 0 1px 1px rgba(0, 0, 0, 0.15);
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .title {
    font-weight: 500;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind {
    text-transform: capitalize;
  }
  .sub {
    display: flex;
    gap: 4px;
    align-items: baseline;
  }
  .caption {
    font-size: 12px;
    color: var(--tertiary);
    line-height: 1.35;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    white-space: normal;
    word-break: break-word;
  }
  .crumbs {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    position: sticky;
    top: 0;
    z-index: 10;
    background: color-mix(in oklab, var(--surface) 92%, transparent);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    padding: 8px 0;
    margin: -8px 0 0;
    transition: box-shadow var(--tg-duration-normal, 200ms) var(--tg-easing-default, ease),
      border-color var(--tg-duration-normal, 200ms) var(--tg-easing-default, ease);
  }
  .crumb {
    background: transparent;
    border: none;
    color: var(--accent);
    cursor: pointer;
    font-family: inherit;
    font-size: inherit;
    padding: 4px 0;
  }
  .crumb:hover {
    text-decoration: underline;
  }
  .crumb-sep {
    color: var(--tertiary);
  }
  .crumb-current {
    color: var(--secondary);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .crumb-search {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
    font-size: 12px;
    font-weight: 500;
    border: 0;
    cursor: pointer;
  }
  .crumb-search:hover {
    background: color-mix(in oklab, var(--accent) 22%, transparent);
  }
  .crumb-search-clear {
    opacity: 0.7;
    font-size: 10px;
  }
  .upload-btn {
    margin-left: 0;
  }
  .output-dir-actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  .crumb-spacer {
    flex: 1;
  }
  .ghost-btn.danger {
    color: var(--error);
    border-color: color-mix(in oklab, var(--error) 30%, var(--input-border));
  }
  .ghost-btn.danger:hover:not(:disabled) {
    background: color-mix(in oklab, var(--error) 10%, transparent);
    border-color: var(--error);
  }
  .crumbs {
    flex-wrap: wrap;
    gap: 8px;
  }
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: calc(var(--padding) * 4) calc(var(--padding) * 2);
    gap: 12px;
    text-align: center;
    color: var(--secondary);
  }
  .empty-state h3 {
    margin: 0;
    font-size: 1.1rem;
    font-weight: 600;
  }
  .empty-state .muted {
    max-width: 320px;
    line-height: 1.5;
  }
  .empty-illustration {
    width: 200px;
    height: 160px;
    color: var(--accent);
    margin-bottom: 8px;
  }
  .drop-overlay {
    position: fixed;
    inset: 0;
    background: color-mix(in oklab, var(--accent) 14%, rgba(0, 0, 0, 0.55));
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 90;
    animation: drop-overlay-in 160ms cubic-bezier(0.22, 1, 0.36, 1);
    pointer-events: none;
  }
  .drop-card {
    background: var(--surface);
    border: 2px dashed var(--accent);
    border-radius: calc(var(--border-radius) * 1.6);
    padding: 32px 48px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.3);
    max-width: 420px;
    text-align: center;
  }
  .drop-icon {
    width: 56px;
    height: 56px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 32px;
    font-weight: 700;
  }
  .drop-card h3 {
    margin: 0;
    color: var(--secondary);
  }
  @keyframes drop-overlay-in {
    from { opacity: 0; }
    to   { opacity: 1; }
  }
  @media (prefers-reduced-motion: reduce) {
    .drop-overlay { animation: none; }
  }
  .filter-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chat-tabs {
    display: flex;
    gap: 2px;
    overflow-x: auto;
    padding: 0 0 6px;
    border-bottom: 1px solid var(--content-border);
    scrollbar-width: none;
  }
  .chat-tabs::-webkit-scrollbar { display: none; }
  .chat-tab {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    background: transparent;
    border: none;
    color: var(--tertiary);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: color var(--tg-duration-fast, 150ms) ease;
    flex-shrink: 0;
    white-space: nowrap;
  }
  .chat-tab:hover:not(.active) {
    color: var(--secondary);
  }
  .chat-tab.active {
    color: var(--accent);
  }
  .chat-tab.active::after {
    content: "";
    position: absolute;
    bottom: -7px;
    left: 14px;
    right: 14px;
    height: 2px;
    background: var(--accent);
    border-radius: 2px 2px 0 0;
  }
  .chat-tab-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 18px;
    min-width: 18px;
    padding: 0 5px;
    border-radius: 9px;
    background: color-mix(in oklab, var(--tertiary) 18%, transparent);
    color: var(--tertiary);
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .chat-tab.active .chat-tab-count {
    background: color-mix(in oklab, var(--accent) 22%, transparent);
    color: var(--accent);
  }
  .chip {
    padding: 6px 14px;
    border-radius: var(--tg-radius-pill);
    border: 1px solid var(--content-border);
    background: var(--surface);
    color: var(--tertiary);
    font-family: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--tg-duration-fast, 150ms) ease;
  }
  .chip:hover:not(.active) {
    color: var(--secondary);
    border-color: var(--tg-accent-border);
    background: var(--tg-accent-bg);
  }
  .chip.active {
    background: var(--accent);
    color: var(--on-accent);
    border-color: var(--accent);
    box-shadow: 0 1px 2px color-mix(in oklab, var(--accent) 30%, transparent);
  }
  .media-card {
    flex-direction: column;
  }
  .thumb {
    width: 100%;
    aspect-ratio: 4 / 3;
    border-radius: 8px;
    background: linear-gradient(135deg,
      color-mix(in oklab, var(--accent) 10%, var(--surface)),
      color-mix(in oklab, var(--accent) 4%, var(--surface)));
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    position: relative;
  }
  .thumb::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(to top, rgba(0, 0, 0, 0.35) 0%, rgba(0, 0, 0, 0) 45%);
    pointer-events: none;
    opacity: 0;
    transition: opacity var(--tg-duration-normal, 200ms) var(--tg-easing-default, ease);
  }
  .media-card:hover .thumb::after {
    opacity: 1;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    transition: transform var(--tg-duration-slow, 320ms) var(--tg-easing-default, cubic-bezier(0.22, 1, 0.36, 1));
  }
  .thumb-img {
    animation: thumb-fade-in 260ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  @keyframes thumb-fade-in {
    from { opacity: 0; filter: blur(8px); transform: scale(1.02); }
    to   { opacity: 1; filter: blur(0); transform: scale(1); }
  }
  .media-card:hover .thumb img {
    transform: scale(1.04);
  }
  .thumb-icon {
    font-size: 2.4rem;
    opacity: 0.55;
    filter: drop-shadow(0 2px 4px rgba(0, 0, 0, 0.1));
    position: relative;
    z-index: 1;
  }
  .thumb-icon.ghost {
    opacity: 0.25;
  }
  .thumb-shimmer {
    position: absolute;
    inset: 0;
    background: linear-gradient(
      90deg,
      color-mix(in oklab, var(--content-border) 30%, transparent),
      color-mix(in oklab, var(--content-border) 60%, transparent),
      color-mix(in oklab, var(--content-border) 30%, transparent)
    );
    background-size: 200% 100%;
    animation: skeleton-shimmer 1.6s ease-in-out infinite;
    pointer-events: none;
  }
  .thumb-play-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    pointer-events: none;
    z-index: 2;
    filter: drop-shadow(0 1px 4px rgba(0, 0, 0, 0.5));
    opacity: 0.9;
    transition: transform var(--tg-duration-normal, 200ms) var(--tg-easing-default, cubic-bezier(0.22, 1, 0.36, 1)),
      opacity var(--tg-duration-normal, 200ms) var(--tg-easing-default, ease);
  }
  .media-card:hover .thumb-play-overlay {
    transform: scale(1.15);
    opacity: 1;
  }
  .webpage-thumb {
    background: linear-gradient(135deg,
      color-mix(in oklab, var(--accent) 18%, var(--surface)),
      color-mix(in oklab, var(--accent) 6%, var(--surface)));
  }
  .webpage-globe {
    color: var(--accent);
    opacity: 0.8;
  }
  .webpage-site-badge {
    position: absolute;
    bottom: 6px;
    left: 6px;
    padding: 2px 8px;
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.55);
    color: #fff;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.02em;
    z-index: 2;
    pointer-events: none;
    max-width: calc(100% - 12px);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .webpage-desc {
    font-size: 12px;
    color: var(--secondary);
    line-height: 1.35;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .webpage-url {
    display: inline-block;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--accent);
    text-decoration: none;
  }
  .webpage-url:hover {
    text-decoration: underline;
  }
  .thumb-gif-badge {
    position: absolute;
    top: 6px;
    left: 6px;
    padding: 1px 6px;
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.6);
    color: #fff;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.05em;
    z-index: 2;
    pointer-events: none;
  }
  .album-badge {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.6);
    border: 0;
    color: #fff;
    font-size: 14px;
    cursor: pointer;
    z-index: 3;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: transform var(--tg-duration-fast, 150ms) var(--tg-easing-default, ease);
  }
  .album-badge:hover {
    transform: scale(1.1);
    background: rgba(0, 0, 0, 0.8);
  }
  .voice-thumb {
    background: color-mix(in oklab, var(--accent) 8%, var(--surface));
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 12px;
    position: relative;
    aspect-ratio: 16 / 9;
  }
  .voice-waveform {
    flex: 1;
    height: 32px;
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .voice-waveform .bar {
    flex: 1;
    background: var(--accent);
    border-radius: 1px;
    min-width: 2px;
  }
  .voice-duration {
    font-size: 11px;
    color: var(--secondary);
    margin-left: 8px;
  }
  .voice-play {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: var(--accent);
    color: white;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .round-thumb {
    aspect-ratio: 1 / 1;
    border-radius: 50%;
    overflow: hidden;
    position: relative;
    background: color-mix(in oklab, var(--secondary) 10%, transparent);
  }
  .round-img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .round-play-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.3);
    color: white;
    pointer-events: none;
  }
  .round-duration {
    position: absolute;
    bottom: 6px;
    left: 50%;
    transform: translateX(-50%);
    padding: 1px 8px;
    border-radius: 8px;
    background: rgba(0, 0, 0, 0.6);
    color: white;
    font-size: 10px;
    font-weight: 600;
  }
  .gif-thumb {
    position: relative;
    aspect-ratio: 1 / 1;
    overflow: hidden;
  }
  .gif-thumb .thumb-img {
    transition: transform var(--tg-duration-slow, 320ms) var(--tg-easing-default, ease);
  }
  .variant-gif:hover .thumb-img {
    transform: scale(1.05);
  }
  .gif-playing-indicator {
    position: absolute;
    bottom: 6px;
    right: 6px;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--accent);
    color: white;
    font-size: 9px;
    font-weight: 700;
    z-index: 2;
  }
  @media (prefers-reduced-motion: reduce) {
    .gif-thumb .thumb-img {
      transition: none;
    }
    .variant-gif:hover .thumb-img {
      transform: none;
    }
  }
  .actions {
    display: flex;
    gap: 4px;
    margin-top: 4px;
  }
  .action {
    background: transparent;
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
    padding: 2px 8px;
    font-family: inherit;
    font-size: 13px;
    color: var(--secondary);
    cursor: pointer;
  }
  .action:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .action:disabled {
    opacity: 0.5;
    cursor: progress;
  }
  .action.star {
    color: var(--tertiary);
  }
  .action.star.active {
    color: var(--warning, #f59e0b);
    border-color: var(--warning, #f59e0b);
  }
  .action.star:hover:not(:disabled) {
    color: var(--warning, #f59e0b);
    border-color: var(--warning, #f59e0b);
  }
  .search-input {
    flex: 1;
    min-width: 200px;
    padding: 8px 14px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--input-border);
    border-radius: var(--tg-radius-input);
    font-family: inherit;
    font-size: 13px;
    transition: border-color var(--tg-duration-fast, 150ms) ease,
      box-shadow var(--tg-duration-fast, 150ms) ease;
  }
  .search-input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--tg-accent-bg);
  }
  .ghost-btn {
    background: transparent;
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-family: inherit;
    cursor: pointer;
    padding: 6px 12px;
    font-size: 13px;
  }
  .ghost-btn.small {
    padding: 4px 10px;
    font-size: 12px;
  }
  .ghost-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .favorites-card .avatar.fav {
    background: linear-gradient(135deg,
      color-mix(in oklab, var(--warning, #f59e0b) 90%, white),
      color-mix(in oklab, var(--warning, #f59e0b) 60%, black));
    color: #fff;
  }
  .smart-card .avatar.smart {
    background: linear-gradient(135deg,
      color-mix(in oklab, var(--accent) 95%, white),
      color-mix(in oklab, var(--accent) 65%, black));
    color: #fff;
  }
  .saved-card .avatar.saved {
    background: linear-gradient(135deg,
      color-mix(in oklab, var(--accent) 95%, white),
      color-mix(in oklab, var(--accent) 70%, black));
    color: #fff;
  }
  .tag-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }
  .tag-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid color-mix(in oklab, var(--accent) 30%, var(--content-border));
    background: color-mix(in oklab, var(--accent) 10%, transparent);
    color: var(--secondary);
    font-family: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  .tag-chip:hover {
    border-color: var(--error);
    color: var(--error);
  }
  .tag-chip.add {
    background: transparent;
    border-style: dashed;
    color: var(--tertiary);
  }
  .tag-chip.add:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .tag-input {
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--accent);
    background: var(--surface);
    color: var(--text);
    font-family: inherit;
    font-size: 11px;
    width: 110px;
  }
  .tag-input:focus {
    outline: none;
  }
  .head-row {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .head-row h2 {
    flex: 1;
  }
  .head-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .status-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 10px;
    border-radius: 999px;
    border: 1px solid var(--content-border);
    background: var(--surface);
    font-size: 11px;
    color: var(--tertiary);
  }
  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--tertiary);
  }
  .status-pill[data-status="connected"] .status-dot {
    background: var(--success);
    box-shadow: 0 0 0 2px color-mix(in oklab, var(--success) 20%, transparent);
  }
  .status-pill[data-status="connected"] {
    border-color: color-mix(in oklab, var(--success) 30%, var(--content-border));
  }
  .status-pill[data-status="checking"] .status-dot {
    background: var(--warning, #f59e0b);
    animation: pulse 1.4s ease-in-out infinite;
  }
  .status-spinner {
    width: 12px;
    height: 12px;
    color: var(--warning, #f59e0b);
    animation: status-spin 900ms linear infinite;
  }
  @keyframes status-spin {
    from { transform: rotate(0deg); }
    to   { transform: rotate(360deg); }
  }
  @media (prefers-reduced-motion: reduce) {
    .status-spinner { animation: none; }
  }
  .status-pill[data-status="disconnected"] .status-dot {
    background: var(--error);
  }
  .status-pill[data-status="disconnected"] {
    color: var(--error);
    border-color: color-mix(in oklab, var(--error) 30%, var(--content-border));
  }
  .bandwidth-pill {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 3px 10px;
    border-radius: var(--tg-radius-pill);
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    border: 1px solid color-mix(in oklab, var(--accent) 30%, var(--content-border));
    color: var(--accent);
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-size: 11px;
    animation: bw-fade-in 220ms ease-out;
  }
  .bw-segment {
    white-space: nowrap;
  }
  @keyframes bw-fade-in {
    from { opacity: 0; transform: translateY(-2px); }
    to { opacity: 1; transform: translateY(0); }
  }
  @media (prefers-reduced-motion: reduce) {
    .bandwidth-pill { animation: none; }
  }
  .list-trim-hint {
    grid-column: 1 / -1;
    text-align: center;
    margin: 12px 0 0;
    padding: 12px;
    border-top: 1px dashed var(--content-border);
  }
  .flood-wait-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 10px;
    border-radius: var(--tg-radius-pill);
    background: color-mix(in oklab, var(--warning, #f59e0b) 18%, transparent);
    border: 1px solid color-mix(in oklab, var(--warning, #f59e0b) 40%, var(--content-border));
    color: var(--warning, #f59e0b);
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    animation: bw-fade-in 220ms ease-out;
  }
  @media (prefers-reduced-motion: reduce) {
    .flood-wait-pill { animation: none; }
  }
  @keyframes pulse {
    0%, 100% { opacity: 0.5; }
    50% { opacity: 1; }
  }
  .icon-btn {
    background: transparent;
    border: 1px solid var(--input-border);
    border-radius: var(--tg-radius-pill);
    width: 32px;
    height: 32px;
    padding: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-family: inherit;
    font-size: 14px;
    color: var(--tertiary);
    cursor: pointer;
    transition: all var(--tg-duration-fast, 150ms) var(--tg-easing-default, ease);
  }
  .icon-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--tg-accent-bg);
    transform: scale(1.05);
  }
  .icon-btn.primary-icon-btn {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
    font-size: 18px;
  }
  .icon-btn.primary-icon-btn:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 88%, black);
    color: var(--on-accent);
    transform: scale(1.05);
  }
  .settings-panel {
    padding: var(--padding);
    background: var(--surface);
    border: 1px solid var(--content-border);
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    animation: panel-in 200ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  @keyframes panel-in {
    from { transform: translateY(-4px); opacity: 0; }
    to   { transform: translateY(0);    opacity: 1; }
  }
  .settings-panel h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--secondary);
  }
  .settings-dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 12px;
    margin: 0;
  }
  .settings-dl dt {
    color: var(--tertiary);
    font-size: 12px;
  }
  .settings-dl dd {
    margin: 0;
    color: var(--secondary);
    word-break: break-all;
  }
  .settings-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .ghost-btn.danger {
    color: var(--error);
    border-color: color-mix(in oklab, var(--error) 30%, var(--input-border));
  }
  .ghost-btn.danger:hover:not(:disabled) {
    border-color: var(--error);
    background: color-mix(in oklab, var(--error) 8%, transparent);
  }
  .skeleton {
    pointer-events: none;
    cursor: default;
    border-color: var(--content-border);
  }
  .skeleton:hover {
    transform: none;
    border-color: var(--content-border);
  }
  @media (prefers-reduced-motion: reduce) {
    .status-pill[data-status="checking"] .status-dot,
    .settings-panel {
      animation: none;
    }
  }
  .media-card {
    position: relative;
  }
  .media-card.selected {
    border-color: var(--accent);
    border-width: 2px;
    background: color-mix(in oklab, var(--accent) 8%, var(--surface));
  }
  .select-mark {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    font-weight: 700;
    z-index: 1;
    pointer-events: none;
  }
  .resume-badge {
    position: absolute;
    bottom: 8px;
    left: 8px;
    background: rgba(0, 0, 0, 0.78);
    color: white;
    padding: 3px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    z-index: 2;
    pointer-events: none;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.2);
  }
  .watched-mark {
    position: absolute;
    top: 6px;
    left: 6px;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--success);
    color: var(--on-accent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    font-weight: 700;
    z-index: 2;
    pointer-events: none;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.2);
  }
  .selection-bar {
    position: sticky;
    bottom: 0;
    margin-top: var(--padding);
    padding: 10px 12px;
    background: var(--button-elevated);
    border: 1px solid var(--accent);
    border-radius: calc(var(--border-radius) * 1.4);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.18);
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    z-index: 10;
    animation: bar-up 220ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  @keyframes bar-up {
    from { transform: translateY(8px); opacity: 0; }
    to   { transform: translateY(0);   opacity: 1; }
  }
  .selection-bar .count {
    font-size: 13px;
    color: var(--secondary);
    font-weight: 600;
    margin-right: auto;
  }
  .bar-btn {
    padding: 6px 14px;
    border-radius: var(--border-radius);
    border: 1px solid var(--input-border);
    background: var(--surface);
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .bar-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .bar-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .bar-btn.primary {
    background: var(--accent);
    color: var(--on-accent);
    border-color: var(--accent);
  }
  .bar-btn.primary:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 88%, black);
    color: var(--on-accent);
  }
  .bar-btn.ghost {
    width: 32px;
    padding: 6px 0;
  }
  .bar-btn.danger {
    color: var(--error);
    border-color: color-mix(in oklab, var(--error) 30%, var(--input-border));
  }
  .bar-btn.danger:hover:not(:disabled) {
    background: color-mix(in oklab, var(--error) 10%, transparent);
    border-color: var(--error);
  }
  .settings-toggle {
    display: grid;
    grid-template-columns: max-content 1fr;
    column-gap: 8px;
    row-gap: 2px;
    align-items: center;
    cursor: pointer;
    font-size: 12px;
  }
  .settings-toggle input {
    margin: 0;
    grid-row: 1 / span 2;
    grid-column: 1;
    width: 14px;
    height: 14px;
  }
  .settings-toggle > span {
    grid-column: 2;
    color: var(--secondary);
  }
  .settings-toggle > .muted {
    color: var(--tertiary);
  }
  .settings-num-row {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    margin-top: 4px;
  }
  .settings-num-row label {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--secondary);
  }
  .settings-num-row input[type="number"] {
    width: 80px;
    padding: 4px 8px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--input-border);
    border-radius: 6px;
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .settings-num-row input[type="number"]:focus {
    outline: none;
    border-color: var(--accent);
  }
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 80;
    cursor: pointer;
  }
  .modal {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(420px, 92vw);
    max-height: 80vh;
    background: var(--button-elevated);
    border: 1px solid var(--content-border);
    border-radius: calc(var(--border-radius) * 1.4);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.28);
    z-index: 81;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
  }
  .modal-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .modal-head h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--secondary);
    flex: 1;
  }
  .modal-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow-y: auto;
    padding-right: 2px;
  }
  .modal-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid transparent;
    border-radius: var(--border-radius);
    background: transparent;
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  .modal-row:hover:not(:disabled) {
    background: var(--surface);
    border-color: var(--accent);
  }
  .modal-row:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .avatar.small {
    width: 32px;
    height: 32px;
  }
  .avatar.small .initial {
    font-size: 0.95rem;
  }
  .keymap-modal {
    width: min(440px, 92vw);
  }
  .keymap-dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    column-gap: 16px;
    row-gap: 8px;
    margin: 0;
    padding: 4px 0 0;
  }
  .keymap-dl dt {
    color: var(--secondary);
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
  }
  .keymap-dl dd {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
    align-self: center;
  }
  .keymap-dl kbd {
    padding: 2px 6px;
    background: var(--surface);
    border: 1px solid var(--input-border);
    border-radius: 4px;
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-size: 11px;
    color: var(--secondary);
  }
  .title-input {
    width: 100%;
    padding: 2px 6px;
    border: 1px solid var(--accent);
    border-radius: var(--border-radius);
    background: var(--surface);
    color: var(--text);
    font-family: inherit;
    font-size: 13px;
  }
  .title-input:focus {
    outline: none;
  }
  .mono {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
  }
  @media (prefers-reduced-motion: reduce) {
    .selection-bar {
      animation: none;
    }
  }
  .load-more-row {
    display: flex;
    justify-content: center;
    padding: var(--padding) 0;
  }
  .ghost-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .muted {
    color: var(--tertiary);
  }
  .small {
    font-size: 11px;
  }
  .error {
    color: var(--error);
  }
  @media (prefers-reduced-motion: reduce) {
    .card {
      transition: none;
    }
    .chip {
      transition: none;
    }
    .thumb-img,
    .avatar img {
      animation: none;
    }
    .thumb-shimmer {
      animation: none;
    }
  }

  .ripple {
    position: relative;
    overflow: hidden;
    isolation: isolate;
  }
  .ripple::before {
    content: "";
    position: absolute;
    inset: 0;
    background: radial-gradient(
      circle at var(--ripple-x, 50%) var(--ripple-y, 50%),
      color-mix(in oklab, var(--accent) 32%, transparent) 0%,
      transparent 50%
    );
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--tg-duration-normal, 200ms) var(--tg-easing-default, cubic-bezier(0.22, 1, 0.36, 1));
    z-index: 0;
  }
  .ripple:active::before {
    opacity: 1;
    transition-duration: 0s;
  }
  .ripple > * {
    position: relative;
    z-index: 1;
  }

  .tg-browser :global(*) {
    scrollbar-width: thin;
    scrollbar-color: color-mix(in oklab, var(--tertiary) 40%, transparent) transparent;
  }
  .tg-browser :global(*::-webkit-scrollbar) {
    width: 8px;
    height: 8px;
  }
  .tg-browser :global(*::-webkit-scrollbar-track) {
    background: transparent;
  }
  .tg-browser :global(*::-webkit-scrollbar-thumb) {
    background: color-mix(in oklab, var(--tertiary) 30%, transparent);
    border-radius: 999px;
    border: 2px solid transparent;
    background-clip: padding-box;
  }
  .tg-browser :global(*::-webkit-scrollbar-thumb:hover) {
    background: color-mix(in oklab, var(--tertiary) 55%, transparent);
    background-clip: padding-box;
    border: 2px solid transparent;
  }
</style>
