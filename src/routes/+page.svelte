<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import SupportedServices from "$components/services/SupportedServices.svelte";
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
  import HomeUrlBar from "$components/home/HomeUrlBar.svelte";
  import HomeInspector from "$components/home/HomeInspector.svelte";
  import {
    type OmniState,
    type PlatformInfo,
    type SearchResult,
    type HomeInputMode,
    showInspectorForState,
    showOmniboxForState,
    isUrl,
  } from "$lib/home/omnibox-controller";
  import { getDownloads, formatBytes } from "$lib/stores/download-store.svelte";
  import { getDownloadStats } from "$lib/stores/download-stats.svelte";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { onClipboardUrl } from "$lib/stores/clipboard-monitor";
  import { getMediaPreview, clearMediaPreview } from "$lib/stores/media-preview-store.svelte";
  import { clearPendingExternalPrefill, getPendingExternalPrefill, type ExternalUrlEvent } from "$lib/stores/external-url-store.svelte";
  import { getOmniboxDraftUrl, setOmniboxDraftUrl } from "$lib/stores/omnibox-draft-store.svelte";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { platformDisplayName } from "$lib/platform-display-names";
  import { STUDY_MAINTENANCE_NOTICE } from "$lib/study-feature-flags";

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

  // Platforms whose content is downloaded via the Courses page (courses
  // plugin + logged-in account), not from a pasted URL.
  const COURSE_PLATFORMS = new Set(["hotmart", "udemy"]);

  let url = $state(getOmniboxDraftUrl());
  let homeInputMode = $state<HomeInputMode>("url");
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

  // Derived quality data from real yt-dlp format info.
  // These update after the user loads formats via FormatSelector.
  let availableHeights = $derived(
    formats.length > 0
      ? [...new Set(
          formats
            .filter(f => f.has_video && typeof f.height === "number" && f.height > 0)
            .map(f => f.height as number)
        )].sort((a, b) => b - a)
      : null
  );
  let hasAudioOnly = $derived(
    formats.some(f => f.has_audio && !f.has_video)
  );

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
  let dlStats = $derived(getDownloadStats());
  let coursesPluginInstalled = $state<boolean | null>(null);

  onMount(() => {
    invoke<{ id: string; enabled: boolean }[]>("list_plugins")
      .then((plugins) => {
        coursesPluginInstalled = plugins.some((p) => p.id === "courses" && p.enabled);
      })
      .catch(() => {});
  });
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
    if (url.trim()) {
      queueMicrotask(() => handleInput());
    }
    return () => {
      onClipboardUrl(null);
    };
  });

  $effect(() => {
    setOmniboxDraftUrl(url);
  });

  const AUTO_DOWNLOAD_DELAY_MS = 2000;

  $effect(() => {
    if (!pendingAutoDownload) return;
    if (omniState.kind === "detected") {
      const info = omniState.info;
      if (COURSE_PLATFORMS.has(info.platform) || info.platform === "p2p") {
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
  let isStage = $derived(omniState.kind === "idle" && !advancedMode);

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


  let showOmnibox = $derived(showOmniboxForState(omniState));
  let showInspector = $derived(showInspectorForState(omniState));

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
    const ms = new Date(v.includes("T") ? v : `${v}T00:00`).getTime();
    if (Number.isNaN(ms)) return null;
    return ms;
  }

  function schedulePart(v: string, part: "date" | "time"): string {
    if (!v) return "";
    const [date, time = ""] = v.split("T");
    return part === "date" ? date : time;
  }

  function withSchedulePart(current: string, part: "date" | "time", value: string): string {
    const pad = (n: number) => String(n).padStart(2, "0");
    let [date, time = ""] = current ? current.split("T") : ["", ""];
    if (part === "date") date = value;
    else time = value;
    if (!date && !time) return "";
    if (!date) {
      const now = new Date();
      date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
    }
    if (!time) time = "00:00";
    return `${date}T${time}`;
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
        if (!COURSE_PLATFORMS.has(result.platform)) {
          invoke("prefetch_media_info", { url: value }).catch(() => {});
        }
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

    if (COURSE_PLATFORMS.has(info.platform)) {
      goto(`/courses/${encodeURIComponent(info.platform)}`);
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

    // B40: a regra do usuário decide antes de a gente perguntar de novo. Só
    // preenche o que ele não escolheu explicitamente nesta sessão — uma regra
    // não pode sobrescrever a escolha feita agora, na frente dele.
    let ruleQuality = selectedQuality;
    try {
      const hit = await invoke<{ name: string; then: { output_dir?: string | null; quality?: string | null } } | null>(
        "preview_rule_match",
        { url: currentUrl, platform },
      );
      if (hit) {
        if (hit.then.output_dir) outputDir = hit.then.output_dir;
        if (hit.then.quality && !selectedQuality) ruleQuality = hit.then.quality;
        showToast("info", $t("omnibox.rule_applied", { name: hit.name }) as string);
      }
    } catch {
      // Regra é conveniência: se falhar, o download segue com as escolhas manuais.
    }

    // B39: comparar com o que esta URL era da última vez, antes de sobrescrever.
    // Depois do download é tarde: o arquivo antigo já foi.
    const snapshot = {
      duration_secs: mediaPreview?.duration_seconds ?? null,
      chapters: [],
      sha256: null,
      title: mediaPreview?.title ?? null,
    };
    try {
      const mudou = await invoke<string | null>("check_media_changed", {
        url: currentUrl,
        current: snapshot,
      });
      if (mudou) {
        showToast("info", $t("omnibox.media_changed", { summary: mudou }) as string);
      }
    } catch {
      // Aviso é cortesia: se falhar, o download segue como sempre seguiu.
    }
    void invoke("record_media_snapshot", { url: currentUrl, snapshot }).catch(() => {});

    omniState = { kind: "preparing", platform };
    url = "";

    try {
      await invoke<DownloadStarted>("download_from_url", {
        url: currentUrl,
        outputDir,
        downloadMode: downloadMode === "auto" ? null : downloadMode,
        quality: ruleQuality,
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

  type PreflightReport = {
    total: number;
    ready: number;
    verdict: "go" | "go_with_skips" | "stop";
    problems: { url: string; problem: string | null }[];
  };

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

    // B34: conferir antes de enfileirar. Sem isto, uma URL sem suporte ou
    // repetida entra na fila e so falha depois, uma por uma — o usuario
    // descobre item a item o que dava para saber de uma vez.
    let paraBaixar = batchUrls;
    try {
      const report = await invoke<PreflightReport>("preflight_batch", {
        urls: batchUrls,
        outputDir,
      });
      if (report.verdict === "stop") {
        showToast("error", $t("omnibox.preflight_stop", { total: report.total }) as string);
        return;
      }
      if (report.problems.length > 0) {
        const ruins = new Set(report.problems.map(p => p.url));
        paraBaixar = batchUrls.filter(u => !ruins.has(u));
        showToast("info", $t("omnibox.preflight_skips", {
          skipped: report.problems.length,
          total: report.total,
        }) as string);
      }
    } catch {
      // A conferencia e uma cortesia, nao um portao: se ela falhar, o lote
      // segue como seguia antes.
    }

    omniState = { kind: "idle" };
    url = "";

    const results = await Promise.allSettled(
      paraBaixar.map(u => invoke<DownloadStarted>("download_from_url", {
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

  function handleHomeModeChange(mode: HomeInputMode) {
    // batch/torrent/p2p are momentary actions, not persistent input modes:
    // return to "url" so the tab bar never strands on an empty state when
    // the picker/dialog is cancelled
    if (mode === "p2p") {
      showP2pSendDialog = true;
      homeInputMode = "url";
    } else if (mode === "torrent") {
      void openTorrentFile().finally(() => {
        homeInputMode = "url";
      });
    } else if (mode === "batch") {
      void openBatchFile().finally(() => {
        homeInputMode = "url";
      });
    }
  }

  function handleDismiss() {
    clearMediaPreview();
    omniState = { kind: "idle" };
    url = "";
  }
</script>

<div class="home-mac" class:home-mac--stage={isStage}>
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

  {#if isStage}
    <div class="home-stage">
      <HomeHero emotion={mascotEmotion} stage celebrate={mascotEmotion === "amazed"} />
      <div class="home-stage-head">
        <h1 class="home-stage-title">{$t('home.hero_title')}</h1>
        <p class="home-stage-copy">{$t('home.hero_subtitle')}</p>
      </div>
      <HomeUrlBar
        variant="stage"
        bind:url
        bind:mode={homeInputMode}
        onInput={handleInput}
        onModeChange={handleHomeModeChange}
        onAdvanced={() => { advancedMode = true; }}
      />
      <p class="home-value">
        {#if dlStats.totalDownloads > 0}
          {@html $t('home.value_line', { count: `<strong>${dlStats.totalDownloads.toLocaleString()}</strong>`, size: `<strong>${formatBytes(dlStats.totalBytes)}</strong>` })}
        {:else}
          {$t('home.value_first')}
        {/if}
      </p>
      <SupportedServices />
    </div>
  {:else}
  <div class="home-mac-workspace" class:home-mac-workspace--idle={omniState.kind === "idle"}>
    <div class="home-mac-hero">
      <HomeHero
        emotion={mascotEmotion}
        compact={mascotCompact}
        bubbleText={bubbleText || undefined}
        celebrate={mascotEmotion === "amazed"}
      />
    </div>
    <div class="home-mac-main">
      {#if advancedMode}
        <div class="home-secondary home-secondary--start">
          <button type="button" class="home-secondary-link" onclick={() => { advancedMode = false; }}>
            {$t('home.action_simple')}
          </button>
        </div>
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
      <HomeUrlBar
        bind:url
        bind:mode={homeInputMode}
        onInput={handleInput}
        onModeChange={handleHomeModeChange}
        onAdvanced={() => { advancedMode = true; }}
      />
    {/if}

    {#if omniState.kind === "batch"}
      <div class="batch-options">
        <DownloadModeSelector bind:downloadMode />
        <BatchDownload count={omniState.urls.length} onDownload={handleBatchDownload} />
      </div>

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
    {/if}
    {/if}

      {#if omniState.kind === "idle"}
        <SupportedServices />
      {/if}

    <HomeInspector open={showInspector} title={$t('home.inspector_title')}>
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
                      <input type="checkbox" checked={selectedPlaylistItems.has(entry.index)} onchange={() => togglePlaylistItem(entry.index)} />
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
                      <input type="checkbox" checked={selectedTorrentFiles.has(entry.index)} onchange={() => toggleTorrentFile(entry.index)} />
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
            <span>{cookieHint === "expired" ? $t("omnibox.cookie_hint_expired") : $t("omnibox.cookie_hint_stale")}</span>
            <button type="button" class="cookie-hint-link" onclick={() => goto("/settings?tab=cookies")}>{$t("omnibox.cookie_hint_action")}</button>
          </p>
        {/if}
        {#if COURSE_PLATFORMS.has(omniState.info.platform)}
          {#if coursesPluginInstalled === false}
            <p class="course-upsell">{$t('omnibox.courses_plugin_needed')}</p>
            <button class="download-primary-btn" onclick={() => goto("/marketplace")}>{$t('omnibox.install_courses_plugin')}</button>
          {:else}
            <p class="course-upsell">{$t('omnibox.courses_plugin_ready')}</p>
            <button class="download-primary-btn" onclick={handleAction}>{$t(omniState.info.platform === "udemy" ? 'omnibox.go_to_udemy' : 'omnibox.go_to_hotmart')}</button>
          {/if}
        {:else}
          {@const playlistBlocked = omniState.info.content_type === "playlist" && playlistEntries.length > 0 && selectedPlaylistItems.size === 0}
          {@const torrentBlocked = torrentEntries.length > 0 && selectedTorrentFiles.size === 0}
          {#if omniState.info.platform === "bilibili"}
            <BilibiliPreviewExtras {url} accountSlug={selectedCookieSlug && selectedCookieSlug !== "_anonymous" ? selectedCookieSlug : null} />
          {/if}
          <button class="download-primary-btn" disabled={playlistBlocked || torrentBlocked} onclick={handleAction}>{$t('omnibox.download')}</button>
          {#if omniState.info.platform !== "direct_file"}
            <details class="options-panel">
              <summary class="options-toggle">{$t('omnibox.options')}</summary>
              <div class="options-content">
                <DownloadModeSelector bind:downloadMode onChange={() => { selectedFormatId = null; }} />
                <QualityPicker bind:selectedQuality selectedFormatId {availableHeights} {hasAudioOnly} />
                {#if cookieAccounts.length > 1}
                  <CookieAccountPicker accounts={cookieAccounts} bind:selectedSlug={selectedCookieSlug} />
                {/if}
                <details class="options-panel">
                  <summary class="options-toggle">{$t('omnibox.advanced')}</summary>
                  <div class="options-content">
                    {#if omniState.info.platform === "vimeo" || omniState.info.platform === "generic"}
                      <div class="referer-input-wrapper">
                        <label class="referer-label" for="referer-input">{$t('omnibox.referer_label')}</label>
                        <input id="referer-input" class="referer-input" type="text" placeholder={$t('omnibox.referer_placeholder')} bind:value={referer} spellcheck="false" />
                      </div>
                    {/if}
                    {#if omniState.info.content_type !== "playlist"}
                      <div class="timerange-wrapper">
                        <span class="timerange-label">{$t('omnibox.timerange_label')}</span>
                        <div class="timerange-inputs">
                          <input class="timerange-input" type="text" placeholder={$t('omnibox.timerange_start')} bind:value={clipStart} spellcheck="false" inputmode="numeric" aria-label={$t('omnibox.timerange_start') as string} />
                          <span class="timerange-sep" aria-hidden="true">—</span>
                          <input class="timerange-input" type="text" placeholder={$t('omnibox.timerange_end')} bind:value={clipEnd} spellcheck="false" inputmode="numeric" aria-label={$t('omnibox.timerange_end') as string} />
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
                      <div class="timerange-inputs schedule-row">
                        <input class="timerange-input schedule-date" type="date" value={schedulePart(scheduleAt, "date")} oninput={(e) => { scheduleAt = withSchedulePart(scheduleAt, "date", e.currentTarget.value); }} aria-label={$t('omnibox.schedule_start') as string} />
                        <input class="timerange-input schedule-time" type="time" step="60" value={schedulePart(scheduleAt, "time")} oninput={(e) => { scheduleAt = withSchedulePart(scheduleAt, "time", e.currentTarget.value); }} aria-label={$t('omnibox.schedule_start') as string} />
                        <span class="timerange-sep" aria-hidden="true">—</span>
                        <input class="timerange-input schedule-date" type="date" value={schedulePart(scheduleStop, "date")} oninput={(e) => { scheduleStop = withSchedulePart(scheduleStop, "date", e.currentTarget.value); }} aria-label={$t('omnibox.schedule_stop') as string} />
                        <input class="timerange-input schedule-time" type="time" step="60" value={schedulePart(scheduleStop, "time")} oninput={(e) => { scheduleStop = withSchedulePart(scheduleStop, "time", e.currentTarget.value); }} aria-label={$t('omnibox.schedule_stop') as string} />
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
        {/if}
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
            <span class="card-title card-error-text">{omniState.message}</span>
          </div>
          <div class="card-row card-actions">
            <button class="button card-action-btn" onclick={handleRetry}>{$t('omnibox.retry')}</button>
          </div>
        </div>
      {/if}
    </HomeInspector>
    </div>

  </div>
  {/if}

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

  <div class="terms-note">
    {$t('terms_note.agreement')}
    <a href="/about/terms" class="terms-link">{$t('terms_note.link')}</a>
  </div>
</div>

<style>
  .home-mac {
    width: 100%;
  }

  .study-maintenance-banner {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    max-width: 640px;
    margin: var(--space-3) auto 0;
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-3);
    background: color-mix(in srgb, var(--warning) 9%, transparent);
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--warning) 22%, transparent);
    border-radius: var(--radius-lg);
    color: var(--text-muted);
    font-size: var(--text-sm);
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
    font-weight: 600;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .study-maintenance-text span {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .study-maintenance-dismiss {
    background: transparent;
    border: none;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm);
    color: var(--text-dim);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    transition: background var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
  }

  .study-maintenance-dismiss:hover {
    color: var(--text);
    background: var(--fill-2);
  }

  .batch-options {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  /* detection line under the omnibox */
  .feedback {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 20px;
    padding: 0 var(--space-1);
  }

  .feedback-icon {
    flex-shrink: 0;
    pointer-events: none;
  }

  .feedback[data-supported="true"] {
    color: var(--success);
  }

  .feedback[data-supported="false"] {
    color: var(--text-dim);
  }

  .feedback-text {
    font-size: var(--text-sm);
    font-weight: 500;
  }

  .feedback-sep {
    opacity: 0.5;
    margin: 0 2px;
  }

  .feedback-spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--fill-3);
    border-top-color: var(--text-muted);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .feedback-enter {
    animation: feedbackEnter var(--duration-base) var(--ease-out);
  }

  @keyframes feedbackEnter {
    from { opacity: 0; transform: translateY(-2px); }
    to { opacity: 1; transform: translateY(0); }
  }

  /* the one primary action on the page */
  .download-primary-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    background: var(--cta);
    color: var(--on-cta);
    font-size: var(--text-md);
    font-weight: 600;
    letter-spacing: var(--track-snug);
    height: 40px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-md);
    border: none;
    cursor: pointer;
    width: 100%;
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--on-cta) 12%, transparent);
    transition: background var(--duration-fast) var(--ease-out), transform var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .download-primary-btn:hover:not(:disabled) {
      background: var(--cta-hover);
    }
  }

  .download-primary-btn:active:not(:disabled) {
    background: var(--cta-press);
    transform: scale(0.99);
  }

  .download-primary-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .download-primary-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  /* playlist / torrent pickers: a grouped list */
  .playlist-picker {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface-mut);
    border-radius: var(--radius-md);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .playlist-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .playlist-count {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }

  .playlist-bulk {
    display: flex;
    gap: var(--space-3);
  }

  .playlist-link {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--accent-hi);
    cursor: pointer;
  }

  .playlist-link:hover {
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .playlist-status {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .playlist-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 220px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  .playlist-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 28px;
    padding: 3px var(--space-1);
    border-radius: var(--radius-xs);
    cursor: pointer;
    font-size: var(--text-base);
    color: var(--text);
  }

  .playlist-item:hover {
    background: var(--fill-1);
  }

  .playlist-idx {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
    font-size: var(--text-sm);
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
    padding-left: var(--space-2);
    color: var(--text-dim);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }

  /* disclosure: "Options" / "Advanced" */
  .options-panel {
    width: 100%;
  }

  .options-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-muted);
    cursor: pointer;
    list-style: none;
    padding: var(--space-1) 0;
    user-select: none;
  }

  .options-toggle::-webkit-details-marker {
    display: none;
  }

  .options-toggle::marker {
    content: "";
  }

  .options-toggle::before {
    content: "";
    width: 9px;
    height: 9px;
    background: currentColor;
    clip-path: polygon(30% 10%, 75% 50%, 30% 90%, 22% 82%, 58% 50%, 22% 18%);
    transition: transform var(--duration-fast) var(--ease-out);
    opacity: 0.7;
  }

  .options-panel[open] > .options-toggle::before {
    transform: rotate(90deg);
  }

  @media (hover: hover) {
    .options-toggle:hover {
      color: var(--text);
    }
  }

  .options-content {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-3) 0 var(--space-1);
    width: 100%;
  }

  .referer-input-wrapper,
  .timerange-wrapper {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .referer-label,
  .timerange-label {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-dim);
  }

  .referer-input,
  .timerange-input {
    height: var(--control-h);
    padding: 0 var(--space-2);
    font-size: var(--text-base);
    background: var(--control-bg);
    border: none;
    border-radius: var(--radius-sm);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    transition: box-shadow var(--duration-fast) var(--ease-out);
  }

  .referer-input::placeholder,
  .timerange-input::placeholder {
    color: var(--text-dim);
  }

  .referer-input:focus-visible,
  .timerange-input:focus-visible {
    outline: none;
    box-shadow:
      inset 0 0 0 var(--hairline) var(--accent),
      0 0 0 3px var(--accent-soft);
  }

  .timerange-inputs {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .timerange-input {
    width: 96px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }

  .timerange-sep {
    color: var(--text-dim);
  }

  .timerange-hint {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .schedule-presets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 2px;
  }

  .schedule-preset {
    height: 24px;
    padding: 0 var(--space-3);
    font-size: var(--text-sm);
    font-weight: 500;
    background: var(--fill-1);
    border: none;
    border-radius: var(--radius-full);
    color: var(--text-muted);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
  }

  .schedule-preset:hover {
    background: var(--fill-2);
    color: var(--text);
  }

  .schedule-row {
    flex-wrap: wrap;
  }

  .schedule-date {
    width: auto;
    flex: 1 1 auto;
    min-width: 8.6em;
  }

  .schedule-time {
    width: auto;
    flex: 0 1 auto;
    min-width: 5.4em;
  }

  .course-upsell {
    margin: 0;
    font-size: var(--text-base);
    line-height: var(--leading-base);
    color: var(--text-muted);
  }

  .cookie-hint {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--warning);
  }
  .cookie-hint.expired {
    color: var(--error);
  }
  .cookie-hint-link {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--accent-hi);
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .feedback-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    background: var(--surface-mut);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .feedback-card[data-status="error"] {
    background: color-mix(in srgb, var(--error) 10%, var(--surface-mut));
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--error) 30%, transparent);
  }

  .external-url-card {
    width: 100%;
    background: color-mix(in srgb, var(--accent) 10%, var(--surface-mut));
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--accent) 30%, transparent);
  }

  .card-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .card-text {
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--text);
  }

  .card-title {
    font-size: var(--text-base);
    font-weight: 600;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-error-text {
    color: var(--text);
    white-space: normal;
    line-height: var(--leading-base);
  }

  .card-status-icon {
    flex-shrink: 0;
    pointer-events: none;
    color: var(--accent-hi);
  }

  .card-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .card-subtext {
    font-size: var(--text-sm);
    font-weight: 400;
    color: var(--text-dim);
  }

  .external-url-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .card-action-btn {
    height: var(--control-h);
    padding: 0 var(--space-3);
    font-size: var(--text-base);
  }

  .dismiss-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    background: transparent;
    border: none;
    border-radius: var(--radius-full);
    cursor: pointer;
    color: var(--text-dim);
    padding: 0;
  }

  .dismiss-btn:hover {
    background: var(--fill-2);
    color: var(--text);
  }

  .search-hint {
    color: var(--text-dim);
  }

  .terms-note {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-faint);
    text-align: center;
    padding: var(--space-2) var(--space-1) var(--space-3);
  }

  .terms-link {
    color: var(--text-dim);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  @media (prefers-reduced-motion: reduce) {
    .feedback-enter {
      animation: none;
    }

    .feedback-spinner {
      animation-duration: 1.5s;
    }
  }
</style>
