<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import SupportedServices from "$components/services/SupportedServices.svelte";
  import OmniboxInput from "$components/omnibox/OmniboxInput.svelte";
  import BilibiliPreviewExtras from "$components/omnibox/BilibiliPreviewExtras.svelte";
  import DownloadModeSelector from "$components/omnibox/DownloadModeSelector.svelte";
  import QualityPicker from "$components/omnibox/QualityPicker.svelte";
  import FormatSelector from "$components/omnibox/FormatSelector.svelte";
  import CookieAccountPicker from "$components/omnibox/CookieAccountPicker.svelte";
  import OmniboxAdvanced from "$components/omnibox/OmniboxAdvanced.svelte";
  import MediaPreview from "$components/omnibox/MediaPreview.svelte";
  import BatchDownload from "$components/omnibox/BatchDownload.svelte";
  import SearchResults from "$components/omnibox/SearchResults.svelte";
  import P2pSendDialog from "$components/p2p/P2pSendDialog.svelte";
  import P2pReceiveDialog from "$components/p2p/P2pReceiveDialog.svelte";
  import HomeHero from "$components/home/HomeHero.svelte";
  // Mascot is consumed by HomeHero; keep direct import path live for backward compat if needed
  import { getDownloads, formatBytes } from "$lib/stores/download-store.svelte";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { onClipboardUrl } from "$lib/stores/clipboard-monitor";
  import { getMediaPreview, clearMediaPreview } from "$lib/stores/media-preview-store.svelte";
  import { clearPendingExternalPrefill, getPendingExternalPrefill, type ExternalUrlEvent } from "$lib/stores/external-url-store.svelte";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { platformDisplayName } from "$lib/platform-display-names";
  import { STUDY_MAINTENANCE_NOTICE } from "$lib/study-feature-flags";

  type PlatformInfo = {
    platform: string;
    supported: boolean;
    content_id: string | null;
    content_type: string | null;
  };

  type DownloadStarted = {
    id: number;
    title: string;
  };

  type FormatInfo = {
    format_id: string;
    ext: string;
    resolution: string | null;
    width: number | null;
    height: number | null;
    fps: number | null;
    vcodec: string | null;
    acodec: string | null;
    filesize: number | null;
    tbr: number | null;
    has_video: boolean;
    has_audio: boolean;
    format_note: string | null;
  };

  type SearchResult = {
    id: string;
    title: string;
    author: string;
    duration: number | null;
    thumbnail_url: string | null;
    url: string;
    platform: string;
  };

  type OmniState =
    | { kind: "idle" }
    | { kind: "detecting" }
    | { kind: "detected"; info: PlatformInfo }
    | { kind: "unsupported" }
    | { kind: "preparing"; platform: string }
    | { kind: "batch"; urls: string[] }
    | { kind: "searching" }
    | { kind: "search-results"; results: SearchResult[] }
    | { kind: "search-empty" }
    | { kind: "error"; message: string; originalUrl: string; platform: string };

  let url = $state("");
  let omniState = $state<OmniState>({ kind: "idle" });
  let debounceTimer = $state<ReturnType<typeof setTimeout> | null>(null);
  let downloadMode = $state<"auto" | "audio" | "mute">("auto");
  let selectedQuality = $state("best");
  let clipStart = $state("");
  let clipEnd = $state("");
  let scheduleAt = $state("");
  let scheduleStop = $state("");
  let playlistEntries = $state<{ index: number; title: string; url: string }[]>([]);
  let selectedPlaylistItems = $state<Set<number>>(new Set());
  let playlistLoading = $state(false);
  let torrentEntries = $state<{ index: number; path: string; size_bytes: number }[]>([]);
  let selectedTorrentFiles = $state<Set<number>>(new Set());
  let torrentLoading = $state(false);
  let selectedFormatId = $state<string | null>(null);
  let formats = $state<FormatInfo[]>([]);
  let loadingFormats = $state(false);
  let formatError = $state<string | null>(null);
  let formatFetchGeneration = $state(0);
  let referer = $state("");

  type CookieAccount = {
    slug: string;
    alias: string;
    captured_at_ms: number;
    cookie_count: number;
    last_used_at_ms: number | null;
  };
  let cookieAccounts = $state<CookieAccount[]>([]);
  let selectedCookieSlug = $state<string | null>(null);
  let cookieHint = $state<"stale" | "expired" | null>(null);
  let advancedMode = $state(false);
  const STUDY_NOTICE_DISMISS_KEY = "omniget.study_maintenance_notice_dismissed_v1";
  let studyNoticeDismissed = $state(
    typeof localStorage !== "undefined"
      && localStorage.getItem(STUDY_NOTICE_DISMISS_KEY) === "1"
  );
  function dismissStudyNotice() {
    studyNoticeDismissed = true;
    try { localStorage.setItem(STUDY_NOTICE_DISMISS_KEY, "1"); } catch {}
  }
  let mediaPreview = $derived(getMediaPreview());
  let pendingExternalPrefill = $derived(getPendingExternalPrefill());
  let previewImageLoading = $state(true);
  let showP2pSendDialog = $state(false);
  let p2pReceiveCode = $state<string | null>(null);
  let p2pReceiveUrl = $state("");
  let externalNotice = $state<ExternalUrlEvent | null>(null);
  let lastExternalPrefillId = $state<number | null>(null);
  let pendingAutoDownload = $state(false);

  onMount(() => {
    onClipboardUrl((detectedUrl) => {
      if (omniState.kind === "preparing") return;
      url = detectedUrl;
      const settings = getSettings();
      const autoDownload = !!(settings?.download.auto_download_on_paste && settings?.download.clipboard_detection);
      pendingAutoDownload = autoDownload;
      handleInput();
      showToast("info", $t(autoDownload ? "toast.auto_download_started" : "toast.clipboard_url_detected"));
    });
    return () => {
      onClipboardUrl(null);
    };
  });

  const AUTO_DOWNLOAD_DELAY_MS = 2000;

  $effect(() => {
    if (!pendingAutoDownload) return;
    if (omniState.kind === "detected") {
      const info = omniState.info;
      if (info.platform === "hotmart" || info.platform === "p2p") {
        pendingAutoDownload = false;
        return;
      }
      pendingAutoDownload = false;
      const snapshotUrl = url;
      setTimeout(() => {
        if (url === snapshotUrl && omniState.kind === "detected") {
          handleAction();
        }
      }, AUTO_DOWNLOAD_DELAY_MS);
    } else if (
      omniState.kind === "unsupported" ||
      omniState.kind === "error" ||
      omniState.kind === "batch" ||
      omniState.kind === "search-results" ||
      omniState.kind === "search-empty" ||
      omniState.kind === "idle"
    ) {
      pendingAutoDownload = false;
    }
  });

  const STALL_THRESHOLD = 30_000;
  let downloads = $derived(getDownloads());
  let stallTick = $state(0);
  let lastCompletionAt = $state(0);
  let firstCompletionOfSession = $state(false);
  let completionSeenIds = new Set<string | number>();

  $effect(() => {
    for (const [id, item] of downloads.entries()) {
      if (item.status === "complete" && !completionSeenIds.has(id)) {
        completionSeenIds.add(id);
        lastCompletionAt = Date.now();
        if (!firstCompletionOfSession) {
          firstCompletionOfSession = true;
        }
        break;
      }
    }
  });

  $effect(() => {
    const interval = setInterval(() => { stallTick++; }, 5000);
    return () => clearInterval(interval);
  });


  $effect(() => {
    if (mediaPreview) {
      previewImageLoading = true;
    }
  });

  $effect(() => {
    const incoming = pendingExternalPrefill;
    if (!incoming || incoming.id === lastExternalPrefillId) {
      return;
    }

    lastExternalPrefillId = incoming.id;
    clearPendingExternalPrefill(incoming.id);
    externalNotice = incoming;

    if (incoming.action !== "prefill") {
      return;
    }

    url = incoming.url;
    if (getSettings()?.download.auto_download_on_paste) {
      pendingAutoDownload = true;
    }
    handleInput();
  });

  let mascotEmotion = $derived.by((): "idle" | "downloading" | "error" | "stalled" | "queue" | "complete" | "amazed" => {
    void stallTick;

    if (lastCompletionAt > 0 && Date.now() - lastCompletionAt < 5000) {
      return firstCompletionOfSession && completionSeenIds.size === 1 ? "amazed" : "complete";
    }

    if (omniState.kind === "preparing") return "downloading";
    if (omniState.kind === "error") return "error";

    let hasActiveDownloading = false;
    let hasActiveStalled = false;
    let hasItems = false;
    for (const item of downloads.values()) {
      hasItems = true;
      if (item.status === "downloading") {
        hasActiveDownloading = true;
        const elapsed = Date.now() - item.lastUpdateAt;
        if (elapsed > STALL_THRESHOLD) {
          hasActiveStalled = true;
        }
      }
    }

    if (hasActiveStalled) return "stalled";
    if (hasActiveDownloading) return "downloading";
    if (hasItems) return "queue";
    return "idle";
  });

  let mascotCompact = $derived(omniState.kind !== "idle");

  function pickRandom(raw: string): string {
    if (raw.includes("|")) {
      const opts = raw.split("|");
      return opts[Math.floor(Math.random() * opts.length)];
    }
    return raw;
  }

  let lastBubbleKey = $state("");
  let bubbleText = $state("");

  $effect(() => {
    let key: string;
    if (mascotEmotion === "amazed") key = "amazed";
    else if (mascotEmotion === "complete") key = "complete";
    else if (mascotEmotion === "queue") key = "queue";
    else if (mascotEmotion === "downloading") key = "downloading";
    else if (mascotEmotion === "stalled") key = "stalled";
    else {
      switch (omniState.kind) {
        case "idle": key = "idle"; break;
        case "detecting": key = "detecting"; break;
        case "detected": key = "detected"; break;
        case "preparing": key = "preparing"; break;
        case "searching":
        case "search-results": key = "search"; break;
        case "error": key = "error"; break;
        default: key = ""; break;
      }
    }
    if (key && key !== lastBubbleKey) {
      lastBubbleKey = key;
      bubbleText = pickRandom($t(`mascot.${key}`));
    } else if (!key) {
      lastBubbleKey = "";
      bubbleText = "";
    }
  });

  let showLoopIcon = $derived(
    omniState.kind === "detected" ||
    omniState.kind === "preparing" ||
    omniState.kind === "batch"
  );

  let showOmnibox = $derived(
    omniState.kind === "idle" ||
    omniState.kind === "detecting" ||
    omniState.kind === "detected" ||
    omniState.kind === "unsupported" ||
    omniState.kind === "batch" ||
    omniState.kind === "searching" ||
    omniState.kind === "search-results" ||
    omniState.kind === "search-empty"
  );

  function isUrl(value: string): boolean {
    return value.startsWith("http://") || value.startsWith("https://") || value.startsWith("magnet:") || value.startsWith("p2p:") || value.endsWith(".torrent");
  }

  function isValidTimeBound(v: string): boolean {
    return /^(\d+:)?\d{1,2}:\d{1,2}(\.\d+)?$|^\d+(\.\d+)?$/.test(v.trim());
  }

  function buildTimeRange(): string | null {
    const s = clipStart.trim();
    const e = clipEnd.trim();
    if (!s && !e) return null;
    if (s && !isValidTimeBound(s)) return null;
    if (e && !isValidTimeBound(e)) return null;
    return `${s || "0"}-${e || "inf"}`;
  }

  function toEpochMs(v: string): number | null {
    if (!v) return null;
    const ms = new Date(v).getTime();
    if (Number.isNaN(ms)) return null;
    return ms;
  }

  function setSchedulePreset(kind: "1h" | "tonight" | "1d") {
    const d = new Date();
    if (kind === "1h") d.setHours(d.getHours() + 1);
    else if (kind === "1d") d.setDate(d.getDate() + 1);
    else {
      d.setHours(22, 0, 0, 0);
      if (d.getTime() < Date.now()) d.setDate(d.getDate() + 1);
    }
    const pad = (n: number) => String(n).padStart(2, "0");
    scheduleAt = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
  }

  function handleInput() {
    if (debounceTimer) clearTimeout(debounceTimer);
    clearMediaPreview();
    clipStart = "";
    clipEnd = "";
    scheduleAt = "";
    scheduleStop = "";
    cookieHint = null;
    playlistEntries = [];
    selectedPlaylistItems = new Set();
    playlistLoading = false;
    torrentEntries = [];
    selectedTorrentFiles = new Set();
    torrentLoading = false;
    const currentSettings = getSettings();
    const saved = currentSettings?.last_download_options;
    const savedMode = saved?.mode;
    const settingsQuality = currentSettings?.download.video_quality;
    selectedQuality = settingsQuality && typeof settingsQuality === "string"
      ? settingsQuality
      : "best";
    downloadMode = savedMode === "audio" || savedMode === "mute" ? savedMode : "auto";
    selectedFormatId = null;
    formats = [];
    loadingFormats = false;
    formatError = null;
    formatFetchGeneration++;
    referer = "";

    const trimmed = url.trim();
    if (!trimmed) {
      omniState = { kind: "idle" };
      return;
    }

    const urls = trimmed.split(/[\s\n]+/).filter(isUrl);

    if (urls.length > 1) {
      omniState = { kind: "batch", urls };
      return;
    }

    if (isUrl(trimmed)) {
      omniState = { kind: "detecting" };
      if (getSettings()?.download.auto_download_on_paste) {
        pendingAutoDownload = true;
      }
      debounceTimer = setTimeout(() => {
        detectPlatform(trimmed);
      }, 500);
      return;
    }

    if (trimmed.length >= 2) {
      omniState = { kind: "searching" };
      debounceTimer = setTimeout(() => {
        performSearch(trimmed);
      }, 600);
    } else {
      omniState = { kind: "idle" };
    }
  }

  async function detectPlatform(value: string) {
    try {
      const result = await invoke<PlatformInfo>("detect_platform", { url: value });
      if (result.supported) {
        omniState = { kind: "detected", info: result };
        invoke("prefetch_media_info", { url: value }).catch(() => {});
        loadCookieAccounts(value);
        if (result.content_type === "playlist") {
          loadPlaylistEntries(value);
        }
        if (isTorrentUrl(value)) {
          loadTorrentContents(value);
        }
      } else {
        omniState = { kind: "unsupported" };
      }
    } catch {
      omniState = { kind: "unsupported" };
    }
  }

  async function loadPlaylistEntries(targetUrl: string) {
    playlistEntries = [];
    selectedPlaylistItems = new Set();
    playlistLoading = true;
    try {
      const entries = await invoke<{ index: number; title: string; url: string }[]>(
        "playlist_entries",
        { url: targetUrl }
      );
      playlistEntries = entries;
      selectedPlaylistItems = new Set(entries.map((e) => e.index));
    } catch {
      playlistEntries = [];
    } finally {
      playlistLoading = false;
    }
  }

  function togglePlaylistItem(idx: number) {
    const next = new Set(selectedPlaylistItems);
    if (next.has(idx)) next.delete(idx);
    else next.add(idx);
    selectedPlaylistItems = next;
  }

  function selectAllPlaylist() {
    selectedPlaylistItems = new Set(playlistEntries.map((e) => e.index));
  }

  function selectNonePlaylist() {
    selectedPlaylistItems = new Set();
  }

  function isTorrentUrl(value: string): boolean {
    const v = value.trim().toLowerCase();
    return v.startsWith("magnet:") || v.endsWith(".torrent");
  }

  async function loadTorrentContents(targetUrl: string) {
    torrentEntries = [];
    selectedTorrentFiles = new Set();
    torrentLoading = true;
    try {
      const entries = await invoke<{ index: number; path: string; size_bytes: number }[]>(
        "torrent_contents",
        { url: targetUrl }
      );
      torrentEntries = entries;
      selectedTorrentFiles = new Set(entries.map((e) => e.index));
    } catch {
      torrentEntries = [];
    } finally {
      torrentLoading = false;
    }
  }

  function toggleTorrentFile(idx: number) {
    const next = new Set(selectedTorrentFiles);
    if (next.has(idx)) next.delete(idx);
    else next.add(idx);
    selectedTorrentFiles = next;
  }

  function selectAllTorrent() {
    selectedTorrentFiles = new Set(torrentEntries.map((e) => e.index));
  }

  function selectNoneTorrent() {
    selectedTorrentFiles = new Set();
  }

  async function loadCookieAccounts(targetUrl: string) {
    try {
      const result = await invoke<{ domain: string; accounts: CookieAccount[] }>(
        "cookies_accounts_for_url",
        { url: targetUrl }
      );
      const accounts = result.accounts.slice().sort((a, b) => {
        const lhs = a.last_used_at_ms ?? a.captured_at_ms;
        const rhs = b.last_used_at_ms ?? b.captured_at_ms;
        return rhs - lhs;
      });
      cookieAccounts = accounts;
      selectedCookieSlug = accounts[0]?.slug ?? null;
      cookieHint = null;
      if (accounts.length > 0) {
        try {
          const h = await invoke<{ items: { domain: string; slug: string; status: string }[] }>(
            "cookies_health"
          );
          const slug = selectedCookieSlug ?? "_default";
          const item =
            h.items.find((it) => it.domain === result.domain && it.slug === slug) ??
            h.items.find((it) => it.domain === result.domain);
          if (item && item.status !== "fresh") {
            cookieHint = item.status === "expired" ? "expired" : "stale";
          }
        } catch {}
      }
    } catch {
      cookieAccounts = [];
      selectedCookieSlug = null;
      cookieHint = null;
    }
  }

  async function performSearch(query: string) {
    try {
      const results = await invoke<SearchResult[]>("search_videos", {
        query,
        platform: "youtube",
        maxResults: 6,
      });
      if (url.trim() !== query) return;
      if (results.length > 0) {
        omniState = { kind: "search-results", results };
      } else {
        omniState = { kind: "search-empty" };
      }
    } catch {
      if (url.trim() === query) {
        omniState = { kind: "search-empty" };
      }
    }
  }

  function selectSearchResult(result: SearchResult) {
    url = result.url;
    omniState = { kind: "detecting" };
    detectPlatform(result.url);
  }

  function getContentTypeLabel(contentType: string | null): string {
    if (!contentType) return $t("omnibox.content_type.unknown");
    const key = `omnibox.content_type.${contentType}`;
    const result = $t(key);
    if (result === key) return $t("omnibox.content_type.unknown");
    return result;
  }


  async function loadFormats() {
    if (loadingFormats) return;
    if (formats.length > 0) {
      formats = [];
      selectedFormatId = null;
      formatError = null;
      return;
    }
    const targetUrl = url.trim();
    if (!targetUrl) {
      formatError = $t("omnibox.formats_error");
      return;
    }
    loadingFormats = true;
    formatError = null;
    const gen = ++formatFetchGeneration;
    try {
      const result = await invoke<FormatInfo[]>("get_media_formats", { url: targetUrl });
      if (gen !== formatFetchGeneration) return;
      formats = result;
      if (result.length === 0) {
        formatError = $t("omnibox.no_formats");
      }
    } catch (e: any) {
      if (gen !== formatFetchGeneration) return;
      formats = [];
      const msg = typeof e === "string" ? e : e.message ?? "";
      formatError = msg ? translateBackendError(msg, $t) : $t("omnibox.formats_error");
    } finally {
      loadingFormats = false;
    }
  }

  function selectFormat(formatId: string) {
    selectedFormatId = formatId;
  }

  function clearFormatSelection() {
    selectedFormatId = null;
  }

  function presetBest() {
    selectedFormatId = null;
    downloadMode = "auto";
    selectedQuality = "best";
  }

  function presetMusic() {
    selectedFormatId = null;
    downloadMode = "audio";
  }

  function persistLastDownloadOptions() {
    const saved = getSettings()?.last_download_options;
    const nextMode = downloadMode;
    if (saved?.mode === nextMode) return;
    updateSettings({
      last_download_options: {
        mode: nextMode,
        quality: saved?.quality ?? "best",
      },
    }).catch(() => {});
  }

  async function handleAction() {
    if (omniState.kind !== "detected") return;
    const info = omniState.info;

    if (info.platform === "hotmart") {
      goto(`/courses?platform=${encodeURIComponent(info.platform)}`);
      return;
    }

    if (info.platform === "p2p") {
      const trimmed = url.trim();
      const code = trimmed.replace(/^p2p:/, "");
      p2pReceiveUrl = trimmed;
      p2pReceiveCode = code;
      return;
    }

    const isPlaylist = info.content_type === "playlist" && playlistEntries.length > 0;
    if (isPlaylist && selectedPlaylistItems.size === 0) {
      showToast("error", $t("omnibox.playlist_none_selected") as string);
      return;
    }

    const isTorrent = torrentEntries.length > 0;
    if (isTorrent && selectedTorrentFiles.size === 0) {
      showToast("error", $t("omnibox.torrent_none_selected") as string);
      return;
    }

    const settings = getSettings();
    let outputDir = settings?.download.default_output_dir ?? "";

    if ((settings?.download.always_ask_path && !settings?.download.auto_download_on_paste) || !outputDir) {
      const selected = await open({
        directory: true,
        title: $t("settings.download.default_output_dir"),
      });
      if (!selected) return;
      outputDir = selected;
    }

    const currentUrl = url.trim();
    const platform = info.platform;
    omniState = { kind: "preparing", platform };
    url = "";

    try {
      await invoke<DownloadStarted>("download_from_url", {
        url: currentUrl,
        outputDir,
        downloadMode: downloadMode === "auto" ? null : downloadMode,
        quality: selectedQuality,
        formatId: selectedFormatId,
        referer: referer.trim() || null,
        cookieSlug: selectedCookieSlug,
        timeRange: buildTimeRange(),
        playlistItems: isPlaylist ? [...selectedPlaylistItems] : null,
        torrentFiles: isTorrent ? [...selectedTorrentFiles] : null,
        scheduledAt: toEpochMs(scheduleAt),
        stopAt: toEpochMs(scheduleStop),
      });
      persistLastDownloadOptions();
      omniState = { kind: "idle" };
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("omnibox.error");
      omniState = {
        kind: "error",
        message: msg,
        originalUrl: currentUrl,
        platform,
      };
    }
  }

  async function handleBatchDownload() {
    if (omniState.kind !== "batch") return;
    const batchUrls = omniState.urls;

    const settings = getSettings();
    let outputDir = settings?.download.default_output_dir ?? "";

    if ((settings?.download.always_ask_path && !settings?.download.auto_download_on_paste) || !outputDir) {
      const selected = await open({
        directory: true,
        title: $t("settings.download.default_output_dir"),
      });
      if (!selected) return;
      outputDir = selected;
    }

    omniState = { kind: "idle" };
    url = "";

    const results = await Promise.allSettled(
      batchUrls.map(u => invoke<DownloadStarted>("download_from_url", {
        url: u,
        outputDir,
        downloadMode: downloadMode === "auto" ? null : downloadMode,
        quality: selectedQuality,
        formatId: null,
        referer: null,
        cookieSlug: null,
      }))
    );

    const queued = results.filter(r => r.status === "fulfilled").length;
    if (queued > 0) {
      showToast("info", $t("omnibox.batch_queued", { count: queued }));
      persistLastDownloadOptions();
    }
  }

  function handleRetry() {
    if (omniState.kind !== "error") return;
    url = omniState.originalUrl;
    omniState = { kind: "detecting" };
    detectPlatform(url.trim());
  }

  async function handleP2pAccept() {
    const currentUrl = p2pReceiveUrl;
    p2pReceiveCode = null;
    p2pReceiveUrl = "";

    const settings = getSettings();
    let outputDir = settings?.download.default_output_dir ?? "";

    if ((settings?.download.always_ask_path && !settings?.download.auto_download_on_paste) || !outputDir) {
      const selected = await open({
        directory: true,
        title: $t("settings.download.default_output_dir"),
      });
      if (!selected) return;
      outputDir = selected;
    }

    omniState = { kind: "preparing", platform: "p2p" };
    url = "";

    try {
      await invoke("download_from_url", {
        url: currentUrl,
        outputDir,
        downloadMode: null,
        quality: "best",
        formatId: null,
        referer: null,
        cookieSlug: null,
      });
      omniState = { kind: "idle" };
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("omnibox.error");
      omniState = { kind: "error", message: msg, originalUrl: currentUrl, platform: "p2p" };
    }
  }

  function handleP2pReject() {
    p2pReceiveCode = null;
    p2pReceiveUrl = "";
  }

  async function openTorrentFile() {
    const selected = await open({
      title: "Select .torrent file",
      filters: [{ name: "Torrent", extensions: ["torrent"] }],
      multiple: false,
    });
    if (selected && typeof selected === "string") {
      url = selected;
      handleInput();
    }
  }

  async function openBatchFile() {
    const selected = await open({
      title: $t("omnibox.batch_file_title"),
      filters: [{ name: "Text", extensions: ["txt"] }],
      multiple: false,
    });
    if (!selected || typeof selected !== "string") return;
    try {
      const urls = await invoke<string[]>("parse_batch_file", { path: selected });
      if (urls.length === 0) {
        showToast("info", $t("omnibox.batch_file_empty"));
        return;
      }
      if (urls.length === 1) {
        url = urls[0];
        handleInput();
        return;
      }
      url = urls.join("\n");
      omniState = { kind: "batch", urls };
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("omnibox.error");
      showToast("error", msg);
    }
  }

  function handleDismiss() {
    clearMediaPreview();
    omniState = { kind: "idle" };
    url = "";
  }
</script>

<div class="home">
  {#if STUDY_MAINTENANCE_NOTICE && !studyNoticeDismissed}
    <div class="study-maintenance-banner" role="status">
      <div class="study-maintenance-text">
        <strong>{$t("study.maintenance.home_banner_title")}</strong>
        <span>{$t("study.maintenance.home_banner_body")}</span>
      </div>
      <button
        type="button"
        class="study-maintenance-dismiss"
        onclick={dismissStudyNotice}
        aria-label={$t('common.close')}
      >
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M18 6L6 18M6 6l12 12" />
        </svg>
      </button>
    </div>
  {/if}

  <HomeHero
    emotion={mascotEmotion}
    compact={mascotCompact}
    bubbleText={bubbleText || undefined}
    celebrate={mascotEmotion === "amazed"}
  />

  <div class="omnibox-area">
    <div class="mode-toggle-row">
      <button
        type="button"
        class="mode-toggle-btn"
        class:active={!advancedMode}
        onclick={() => { advancedMode = false; }}
      >
        {$t('omnibox.mode_normal')}
      </button>
      <button
        type="button"
        class="mode-toggle-btn"
        class:active={advancedMode}
        onclick={() => { advancedMode = true; }}
      >
        {$t('omnibox.mode_advanced')}
      </button>
    </div>

    {#if advancedMode}
      <OmniboxAdvanced />
    {:else}
    {#if externalNotice}
      <div class="feedback-card feedback-enter external-url-card">
        <div class="card-row">
          <svg class="card-status-icon" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 5v14" />
            <path d="M5 12h14" />
          </svg>
          <span class="card-title">{$t('omnibox.external_url_ready')}</span>
          <button class="dismiss-btn" onclick={() => { externalNotice = null; }} aria-label={$t('common.close')}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        </div>
        <div class="card-row">
          <span class="card-subtext external-url-text">{externalNotice.url}</span>
        </div>
      </div>
    {/if}

    {#if showLoopIcon}
      <img
        src="/loop.png"
        alt=""
        width="40"
        height="40"
        class="loop-icon"
        class:loop-bounce={omniState.kind === "detected" || omniState.kind === "batch"}
        class:loop-pulse={omniState.kind === "preparing"}
      />
    {/if}

    {#if omniState.kind === "detecting"}
      <div class="feedback feedback-enter">
        <span class="feedback-spinner"></span>
      </div>

    {:else if omniState.kind === "detected"}
      <div class="feedback feedback-enter" data-supported="true">
        <span class="feedback-text">
          {platformDisplayName(omniState.info.platform)}
          {#if omniState.info.content_type}
            <span class="feedback-sep">&middot;</span>
            {getContentTypeLabel(omniState.info.content_type)}
          {/if}
        </span>
      </div>
    {/if}

    {#if showOmnibox}
      <OmniboxInput bind:url onInput={handleInput} />
    {/if}

    {#if omniState.kind === "detected"}
      <MediaPreview bind:mediaPreview bind:imageLoading={previewImageLoading} />

      {#if omniState.info.content_type === "playlist"}
        <div class="playlist-picker">
          <div class="playlist-head">
            <span class="playlist-count">
              {$t('omnibox.playlist_selected', { selected: selectedPlaylistItems.size, total: playlistEntries.length })}
            </span>
            {#if !playlistLoading && playlistEntries.length > 0}
              <div class="playlist-bulk">
                <button type="button" class="playlist-link" onclick={selectAllPlaylist}>{$t('omnibox.playlist_all')}</button>
                <button type="button" class="playlist-link" onclick={selectNonePlaylist}>{$t('omnibox.playlist_none')}</button>
              </div>
            {/if}
          </div>
          {#if playlistLoading}
            <div class="playlist-status"><span class="feedback-spinner"></span> {$t('omnibox.playlist_loading')}</div>
          {:else if playlistEntries.length === 0}
            <span class="playlist-status">{$t('omnibox.playlist_empty')}</span>
          {:else}
            <ul class="playlist-list">
              {#each playlistEntries as entry (entry.index)}
                <li>
                  <label class="playlist-item">
                    <input
                      type="checkbox"
                      checked={selectedPlaylistItems.has(entry.index)}
                      onchange={() => togglePlaylistItem(entry.index)}
                    />
                    <span class="playlist-idx">{entry.index}.</span>
                    <span class="playlist-title">{entry.title}</span>
                  </label>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}

      {#if torrentLoading || torrentEntries.length > 0}
        <div class="playlist-picker">
          <div class="playlist-head">
            <span class="playlist-count">
              {$t('omnibox.torrent_selected', { selected: selectedTorrentFiles.size, total: torrentEntries.length })}
            </span>
            {#if !torrentLoading && torrentEntries.length > 0}
              <div class="playlist-bulk">
                <button type="button" class="playlist-link" onclick={selectAllTorrent}>{$t('omnibox.playlist_all')}</button>
                <button type="button" class="playlist-link" onclick={selectNoneTorrent}>{$t('omnibox.playlist_none')}</button>
              </div>
            {/if}
          </div>
          {#if torrentLoading}
            <div class="playlist-status"><span class="feedback-spinner"></span> {$t('omnibox.torrent_loading')}</div>
          {:else}
            <ul class="playlist-list">
              {#each torrentEntries as entry (entry.index)}
                <li>
                  <label class="playlist-item">
                    <input
                      type="checkbox"
                      checked={selectedTorrentFiles.has(entry.index)}
                      onchange={() => toggleTorrentFile(entry.index)}
                    />
                    <span class="playlist-title">{entry.path}</span>
                    <span class="torrent-size">{formatBytes(entry.size_bytes)}</span>
                  </label>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}

      {#if cookieHint}
        <p class="cookie-hint" class:expired={cookieHint === "expired"} role="status">
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
          <span>
            {cookieHint === "expired"
              ? $t("omnibox.cookie_hint_expired")
              : $t("omnibox.cookie_hint_stale")}
          </span>
          <button type="button" class="cookie-hint-link" onclick={() => goto("/settings?tab=cookies")}>
            {$t("omnibox.cookie_hint_action")}
          </button>
        </p>
      {/if}

      {#if omniState.info.platform === "hotmart"}
        <button class="button action-btn" onclick={handleAction}>
          {$t('omnibox.go_to_hotmart')}
        </button>
      {:else}
        {@const playlistBlocked = omniState.info.content_type === "playlist" && playlistEntries.length > 0 && selectedPlaylistItems.size === 0}
        {@const torrentBlocked = torrentEntries.length > 0 && selectedTorrentFiles.size === 0}
        {#if omniState.info.platform === "bilibili"}
          <BilibiliPreviewExtras {url} accountSlug={selectedCookieSlug} />
        {/if}
        <button class="download-primary-btn" disabled={playlistBlocked || torrentBlocked} onclick={handleAction}>
          {$t('omnibox.download')}
        </button>

        <details class="options-panel">
          <summary class="options-toggle">{$t('omnibox.options')}</summary>
          <div class="options-content">
            <DownloadModeSelector bind:downloadMode onChange={() => { selectedFormatId = null; }} />
            <QualityPicker bind:selectedQuality selectedFormatId />
            {#if cookieAccounts.length > 1}
              <CookieAccountPicker accounts={cookieAccounts} bind:selectedSlug={selectedCookieSlug} />
            {/if}

            <details class="options-panel">
              <summary class="options-toggle">{$t('omnibox.advanced')}</summary>
              <div class="options-content">
                {#if omniState.info.platform === "vimeo" || omniState.info.platform === "generic"}
                  <div class="referer-input-wrapper">
                    <label class="referer-label" for="referer-input">{$t('omnibox.referer_label')}</label>
                    <input
                      id="referer-input"
                      class="referer-input"
                      type="text"
                      placeholder={$t('omnibox.referer_placeholder')}
                      bind:value={referer}
                      spellcheck="false"
                    />
                  </div>
                {/if}

                {#if omniState.info.content_type !== "playlist"}
                  <div class="timerange-wrapper">
                    <span class="timerange-label">{$t('omnibox.timerange_label')}</span>
                    <div class="timerange-inputs">
                      <input
                        class="timerange-input"
                        type="text"
                        placeholder={$t('omnibox.timerange_start')}
                        bind:value={clipStart}
                        spellcheck="false"
                        inputmode="numeric"
                        aria-label={$t('omnibox.timerange_start') as string}
                      />
                      <span class="timerange-sep" aria-hidden="true">—</span>
                      <input
                        class="timerange-input"
                        type="text"
                        placeholder={$t('omnibox.timerange_end')}
                        bind:value={clipEnd}
                        spellcheck="false"
                        inputmode="numeric"
                        aria-label={$t('omnibox.timerange_end') as string}
                      />
                    </div>
                    <span class="timerange-hint">{$t('omnibox.timerange_hint')}</span>
                  </div>
                {/if}

                <div class="timerange-wrapper">
                  <span class="timerange-label">{$t('omnibox.schedule_label')}</span>
                  <div class="schedule-presets">
                    <button type="button" class="schedule-preset" onclick={() => setSchedulePreset('1h')}>{$t('omnibox.schedule_1h')}</button>
                    <button type="button" class="schedule-preset" onclick={() => setSchedulePreset('tonight')}>{$t('omnibox.schedule_tonight')}</button>
                    <button type="button" class="schedule-preset" onclick={() => setSchedulePreset('1d')}>{$t('omnibox.schedule_1d')}</button>
                    {#if scheduleAt || scheduleStop}
                      <button type="button" class="schedule-preset" onclick={() => { scheduleAt = ""; scheduleStop = ""; }}>{$t('omnibox.schedule_clear')}</button>
                    {/if}
                  </div>
                  <div class="timerange-inputs">
                    <input
                      class="timerange-input schedule-input"
                      type="datetime-local"
                      bind:value={scheduleAt}
                      aria-label={$t('omnibox.schedule_start') as string}
                    />
                    <span class="timerange-sep" aria-hidden="true">—</span>
                    <input
                      class="timerange-input schedule-input"
                      type="datetime-local"
                      bind:value={scheduleStop}
                      aria-label={$t('omnibox.schedule_stop') as string}
                    />
                  </div>
                  <span class="timerange-hint">{$t('omnibox.schedule_hint')}</span>
                </div>

                <FormatSelector
                  platform={omniState.info.platform}
                  isPlaylist={omniState.info.content_type === "playlist"}
                  bind:formats
                  bind:selectedFormatId
                  {loadingFormats}
                  {formatError}
                  onLoadFormats={loadFormats}
                  onSelectFormat={selectFormat}
                  onClearFormat={clearFormatSelection}
                  onPresetBest={presetBest}
                  onPresetMusic={presetMusic}
                />
              </div>
            </details>
          </div>
        </details>
      {/if}

    {:else if omniState.kind === "batch"}
      <BatchDownload count={omniState.urls.length} onDownload={handleBatchDownload} />

    {:else if omniState.kind === "searching"}
      <div class="feedback feedback-enter">
        <span class="feedback-spinner"></span>
        <span class="feedback-text search-hint">{$t('omnibox.searching')}</span>
      </div>

    {:else if omniState.kind === "search-results"}
      <SearchResults results={omniState.results} onSelect={selectSearchResult} />

    {:else if omniState.kind === "search-empty"}
      <div class="feedback feedback-enter" data-supported="false">
        <svg class="feedback-icon" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="11" cy="11" r="8" />
          <path d="M21 21l-4.35-4.35" />
        </svg>
        <span class="feedback-text">{$t('omnibox.search_empty')}</span>
      </div>

    {:else if omniState.kind === "unsupported"}
      <div class="feedback feedback-enter" data-supported="false">
        <svg class="feedback-icon" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10" />
          <path d="M12 8v4m0 4h.01" />
        </svg>
        <span class="feedback-text">{$t('omnibox.unsupported')}</span>
      </div>

    {:else if omniState.kind === "preparing"}
      <div class="feedback-card feedback-enter">
        <div class="card-row">
          <span class="feedback-spinner"></span>
          <span class="card-text">{$t('omnibox.preparing')}</span>
        </div>
      </div>

    {:else if omniState.kind === "error"}
      <div class="feedback-card feedback-enter" data-status="error">
        <div class="card-row">
          <svg class="card-status-icon error" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <path d="M12 8v4m0 4h.01" />
          </svg>
          <span class="card-title card-error-text">{omniState.message}</span>
          <button class="dismiss-btn" onclick={handleDismiss} aria-label={$t('common.close')}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        </div>
        <div class="card-row card-actions">
          <span class="card-subtext">{$t('omnibox.error')}</span>
          <button class="button card-action-btn" onclick={handleRetry}>
            {$t('omnibox.retry')}
          </button>
        </div>
      </div>
    {/if}
    {/if}
  </div>

  {#if showP2pSendDialog}
    <P2pSendDialog onClose={() => { showP2pSendDialog = false; }} />
  {/if}

  {#if p2pReceiveCode}
    <P2pReceiveDialog
      code={p2pReceiveCode}
      onAccept={handleP2pAccept}
      onReject={handleP2pReject}
    />
  {/if}

  {#if omniState.kind === "idle"}
    <SupportedServices />
  {/if}

  <div class="quick-actions">
    <button class="quick-action-btn" onclick={() => { showP2pSendDialog = true; }}>
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M22 2L11 13" />
        <path d="M22 2L15 22 11 13 2 9z" />
      </svg>
      {$t("p2p.send_file")}
    </button>
    <button class="quick-action-btn" onclick={openTorrentFile}>
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="3" />
        <circle cx="5" cy="6" r="2" />
        <circle cx="19" cy="6" r="2" />
        <path d="M9.5 10.5L6.5 7.5" />
        <path d="M14.5 10.5l3-3" />
      </svg>
      {$t("torrent.open_file")}
    </button>
    <button class="quick-action-btn" onclick={openBatchFile}>
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
        <polyline points="14 2 14 8 20 8" />
        <line x1="8" y1="13" x2="16" y2="13" />
        <line x1="8" y1="17" x2="16" y2="17" />
      </svg>
      {$t("omnibox.batch_file_open")}
    </button>
  </div>

  <div class="terms-note">
    {$t('terms_note.agreement')}
    <a href="/about/terms" class="terms-link">{$t('terms_note.link')}</a>
  </div>
</div>

<style>
  .home {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--space-7));
    gap: var(--space-6);
    padding: var(--space-5) var(--space-3);
  }

  .study-maintenance-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    max-width: 640px;
    padding: 8px 12px;
    background: color-mix(in oklab, var(--secondary) 8%, transparent);
    border: 1px solid color-mix(in oklab, var(--secondary) 18%, transparent);
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-size: 12px;
    line-height: 1.4;
  }

  .study-maintenance-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
    min-width: 0;
  }

  .study-maintenance-text strong {
    font-weight: 500;
    font-size: 12.5px;
    color: var(--text);
  }

  .study-maintenance-text span {
    color: var(--secondary);
    font-size: 11.5px;
  }

  .study-maintenance-dismiss {
    background: transparent;
    border: none;
    padding: 4px;
    margin: -4px;
    border-radius: 4px;
    color: var(--secondary);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0.55;
    transition: opacity 0.15s ease, background 0.15s ease;
  }

  .study-maintenance-dismiss:hover {
    opacity: 1;
    background: color-mix(in oklab, var(--text) 6%, transparent);
  }

  .omnibox-area {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    max-width: 640px;
  }

  .mode-toggle-row {
    display: inline-flex;
    background: var(--button);
    border-radius: var(--border-radius);
    padding: 3px;
    gap: 2px;
    margin-bottom: 4px;
  }
  .mode-toggle-btn {
    padding: 5px 14px;
    font-size: 11.5px;
    font-weight: 500;
    color: var(--gray);
    background: transparent;
    border: none;
    border-radius: calc(var(--border-radius) - 3px);
    cursor: pointer;
  }
  .mode-toggle-btn.active {
    background: var(--cta);
    color: var(--on-cta);
  }
  .mode-toggle-btn:not(.active):hover {
    color: var(--secondary);
    background: var(--button-elevated);
  }

  .loop-icon {
    pointer-events: none;
    border-radius: 10px;
    user-select: none;
  }

  .loop-bounce {
    animation: loopBounce 400ms cubic-bezier(0.34, 1.56, 0.64, 1);
  }

  @keyframes loopBounce {
    0% {
      opacity: 0;
      transform: scale(0.6) translateY(6px);
    }
    60% {
      opacity: 1;
      transform: scale(1.08) translateY(-2px);
    }
    100% {
      transform: scale(1) translateY(0);
    }
  }

  .loop-pulse {
    animation: loopPulse 1.8s ease-in-out infinite;
  }

  @keyframes loopPulse {
    0%, 100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.7;
      transform: scale(0.95);
    }
  }

  .feedback {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }

  .feedback-icon {
    flex-shrink: 0;
    pointer-events: none;
  }

  .feedback[data-supported="true"] {
    color: var(--green);
  }

  .feedback[data-supported="false"] {
    color: var(--gray);
  }

  .feedback-text {
    font-size: 12.5px;
    font-weight: 500;
  }

  .feedback-sep {
    opacity: 0.5;
    margin: 0 2px;
  }

  .feedback-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--input-border);
    border-top-color: var(--secondary);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .feedback-enter {
    animation: feedbackEnter 300ms cubic-bezier(0.34, 1.56, 0.64, 1);
  }

  @keyframes feedbackEnter {
    from {
      opacity: 0;
      transform: scale(0.9);
    }
    to {
      opacity: 1;
      transform: scale(1);
    }
  }

  .download-primary-btn {
    background: var(--cta);
    color: var(--on-cta);
    font-size: 15px;
    font-weight: 500;
    padding: 12px 32px;
    border-radius: var(--border-radius);
    border: none;
    cursor: pointer;
    width: 100%;
    max-width: 300px;
    transition: background 150ms;
  }

  @media (hover: hover) {
    .download-primary-btn:hover {
      background: var(--cta-hover);
    }
  }

  .download-primary-btn:active {
    background: var(--cta-press);
  }

  .download-primary-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .download-primary-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .playlist-picker {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
  }

  .playlist-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .playlist-count {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--secondary);
  }

  .playlist-bulk {
    display: flex;
    gap: 10px;
  }

  .playlist-link {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    font-size: 12px;
    color: var(--accent);
    cursor: pointer;
  }

  .playlist-link:hover {
    text-decoration: underline;
  }

  .playlist-status {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--gray);
  }

  .playlist-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 220px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .playlist-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 4px;
    border-radius: calc(var(--border-radius) - 3px);
    cursor: pointer;
    font-size: 13px;
    color: var(--secondary);
  }

  .playlist-item:hover {
    background: var(--button-elevated);
  }

  .playlist-idx {
    color: var(--gray);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .playlist-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .torrent-size {
    margin-left: auto;
    flex-shrink: 0;
    padding-left: 8px;
    color: var(--gray);
    font-variant-numeric: tabular-nums;
  }

  .options-panel {
    width: 100%;
  }

  .options-toggle {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    cursor: pointer;
    list-style: none;
    text-align: center;
    padding: 4px 0;
    user-select: none;
  }

  .options-toggle::-webkit-details-marker {
    display: none;
  }

  .options-toggle::marker {
    content: "";
  }

  @media (hover: hover) {
    .options-toggle:hover {
      color: var(--secondary);
    }
  }

  .options-content {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding-top: var(--padding);
    width: 100%;
  }

  .referer-input-wrapper {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .referer-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .referer-input {
    padding: 6px var(--padding);
    font-size: 13px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--secondary);
  }

  .referer-input::placeholder {
    color: var(--gray);
  }

  .referer-input:focus-visible {
    border-color: var(--secondary);
    outline: none;
  }

  .timerange-wrapper {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .timerange-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .timerange-inputs {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .timerange-input {
    width: 96px;
    padding: 6px var(--padding);
    font-size: 13px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--secondary);
    text-align: center;
  }

  .timerange-input::placeholder {
    color: var(--gray);
  }

  .timerange-input:focus-visible {
    border-color: var(--secondary);
    outline: none;
  }

  .timerange-sep {
    color: var(--gray);
  }

  .timerange-hint {
    font-size: 11.5px;
    color: var(--gray);
  }

  .schedule-presets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 6px;
  }

  .schedule-preset {
    padding: 4px 10px;
    font-size: 12px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--text);
    cursor: pointer;
  }

  .schedule-preset:hover {
    background: var(--button-elevated);
  }

  .schedule-input {
    width: auto;
    flex: 1;
    min-width: 0;
  }

  .cookie-hint {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: 12px;
    color: #f4a72b;
  }
  .cookie-hint.expired {
    color: #e0564f;
  }
  .cookie-hint svg {
    flex-shrink: 0;
  }
  .cookie-hint-link {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    font-size: 12px;
    color: var(--accent);
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .cookie-hint-link:hover {
    filter: brightness(1.15);
  }

  .action-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: var(--padding) calc(var(--padding) * 2);
    font-size: 14.5px;
    font-weight: 500;
    background: var(--button);
    border: none;
    border-radius: var(--border-radius);
    color: var(--button-text);
    cursor: pointer;
    box-shadow: var(--button-box-shadow);
  }

  @media (hover: hover) {
    .action-btn:hover {
      background: var(--button-hover);
    }
  }

  .action-btn:active {
    background: var(--button-press);
  }

  .action-btn:disabled {
    cursor: default;
  }

  .feedback-card {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: var(--padding) calc(var(--padding) * 1.5);
    background: var(--button-elevated);
    border-radius: var(--border-radius);
    border-left: 3px solid var(--blue);
  }

  .feedback-card[data-status="error"] {
    border-left-color: var(--red);
  }

  .external-url-card {
    width: 100%;
    border-left-color: var(--accent);
  }

  .card-row {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }

  .card-text {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
  }

  .card-title {
    font-size: 13px;
    font-weight: 500;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-error-text {
    color: var(--secondary);
  }

  .card-status-icon {
    flex-shrink: 0;
    pointer-events: none;
    color: var(--blue);
  }

  .card-status-icon.error {
    color: var(--red);
  }

  .card-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
  }

  .card-subtext {
    font-size: 11.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .external-url-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-action-btn {
    padding: 6px 12px;
    font-size: 13px;
  }

  .dismiss-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--secondary);
    padding: 0;
  }

  .search-hint {
    color: var(--gray);
  }

  .quick-actions {
    display: flex;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }

  .quick-action-btn {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    font-size: var(--text-sm);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out), transform var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .quick-action-btn:hover {
      background: var(--surface-hi);
      transform: translateY(-1px);
    }
  }

  .quick-action-btn:active {
    transform: scale(0.98);
    background: var(--surface-hi);
  }

  .quick-action-btn svg {
    opacity: 0.7;
    flex-shrink: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .quick-action-btn {
      transition: none;
    }
    .quick-action-btn:hover,
    .quick-action-btn:active {
      transform: none;
    }
  }

  .terms-note {
    font-size: 9px;
    color: var(--gray);
    text-align: center;
    opacity: 0.3;
  }

  .terms-link {
    color: var(--blue);
    text-decoration: none;
  }

  @media (hover: hover) {
    .terms-link:hover {
      text-decoration: underline;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .feedback-enter {
      animation: none;
    }

    .feedback-spinner {
      animation-duration: 1.5s;
    }

    .loop-bounce {
      animation: none;
    }

    .loop-pulse {
      animation: none;
    }
  }
</style>
