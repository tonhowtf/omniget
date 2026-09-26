<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import BilibiliPreviewExtras from "$components/omnibox/BilibiliPreviewExtras.svelte";
  import DownloadModeSelector from "$components/omnibox/DownloadModeSelector.svelte";
  import QualityPicker from "$components/omnibox/QualityPicker.svelte";
  import FormatSelector from "$components/omnibox/FormatSelector.svelte";
  import CookieAccountPicker from "$components/omnibox/CookieAccountPicker.svelte";
  import OutputLocationPicker from "$components/omnibox/OutputLocationPicker.svelte";
  import OmniboxAdvanced from "$components/omnibox/OmniboxAdvanced.svelte";
  import MediaPreview from "$components/omnibox/MediaPreview.svelte";
  import SearchResults from "$components/omnibox/SearchResults.svelte";
  import P2pSendDialog from "$components/p2p/P2pSendDialog.svelte";
  import P2pReceiveDialog from "$components/p2p/P2pReceiveDialog.svelte";
  import HomeHero from "$components/home/HomeHero.svelte";
  import HomeUrlBar from "$components/home/HomeUrlBar.svelte";
  import HomeInspector from "$components/home/HomeInspector.svelte";
  import HomeDropOverlay from "$components/home/HomeDropOverlay.svelte";
  import {
    type OmniState,
    type PlatformInfo,
    type SearchResult,
    type HomeInputMode,
    type MoreAction,
    type HomeArt,
    isUrl,
  } from "$lib/home/omnibox-controller";
  import { formatBytes } from "$lib/stores/download-store.svelte";
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
  import { classifyError, type ErrorKind } from "$lib/home/friendly-error";
  import { pluralKey, splitLink } from "$lib/home/plural";

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
  let selectedOutputDir = $state("");

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

  // First run = until the first download starts. The terms line and the sites
  // line live only there (RCD: put friction where it is justified, once).
  const FIRST_RUN_KEY = "omniget.home.terms_seen_v1";
  const SITES_URL = "https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md";
  let firstRun = $state(readFirstRun());
  function readFirstRun(): boolean {
    try { return localStorage.getItem(FIRST_RUN_KEY) !== "1"; } catch { return false; }
  }
  function markStarted() {
    firstRun = false;
    try { localStorage.setItem(FIRST_RUN_KEY, "1"); } catch {}
  }
  function openSupportedSites() {
    void openExternal(SITES_URL).catch(() => {});
  }

  // A short "added" moment after a download starts, instead of snapping back
  // to an empty page (21st Morph Button idea: the state morphs, then settles).
  let success = $state<{ first: boolean } | null>(null);
  let successTimer: ReturnType<typeof setTimeout> | null = null;
  function flashSuccess(first: boolean) {
    success = { first };
    if (successTimer) clearTimeout(successTimer);
    successTimer = setTimeout(() => { success = null; }, first ? 6000 : 4000);
  }

  let urlInput = $state<HTMLInputElement | null>(null);
  let dragging = $state(false);
  let errorCopied = $state(false);
  let mediaPreview = $derived(getMediaPreview());
  let dlStats = $derived(getDownloadStats());
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
      if (info.platform === "p2p") {
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

  function handleAnalyze() {
    handleInput();
    pendingAutoDownload = false;
  }

  function handleInput() {
    if (debounceTimer) clearTimeout(debounceTimer);
    success = null;
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

    if (info.platform === "p2p") {
      const trimmed = url.trim();
      const code = trimmed.replace(/^p2p:/, "");
      p2pReceiveUrl = trimmed;
      p2pReceiveCode = code;
      return;
    }

    const isPlaylist =
      info.content_type === "playlist" && playlistEntries.length > 0;

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
    const hasExplicitOutputDir = !!selectedOutputDir;

    let outputDir =
      selectedOutputDir || settings?.download.default_output_dir || "";

    if (
      (!hasExplicitOutputDir &&
        settings?.download.always_ask_path &&
        !settings?.download.auto_download_on_paste) ||
      !outputDir
    ) {
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
      const hit = await invoke<{
        name: string;
        then: {
          output_dir?: string | null;
          quality?: string | null;
        };
      } | null>("preview_rule_match", {
        url: currentUrl,
        platform,
      });

      if (hit) {
        if (hit.then.output_dir && !hasExplicitOutputDir) {
          outputDir = hit.then.output_dir;
        }

        if (hit.then.quality && !selectedQuality) {
          ruleQuality = hit.then.quality;
        }

        showToast(
          "info",
          $t("omnibox.rule_applied", { name: hit.name }) as string,
        );
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
        showToast(
          "info",
          $t("omnibox.media_changed", { summary: mudou }) as string,
        );
      }
    } catch {
      // Aviso é cortesia: se falhar, o download segue como sempre seguiu.
    }

    void invoke("record_media_snapshot", {
      url: currentUrl,
      snapshot,
    }).catch(() => {});

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
      const first = firstRun;
      markStarted();
      flashSuccess(first);
    } catch (e: any) {
      const msg =
        typeof e === "string" ? e : e.message ?? $t("omnibox.error");

    // keep the link in the field so the user can fix or retry it
    url = currentUrl;
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
    let outputDir = selectedOutputDir || settings?.download.default_output_dir || "";

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
      showToast("info", $t(pluralKey("omnibox.batch_queued", queued), { count: queued }));
      persistLastDownloadOptions();
      const first = firstRun;
      markStarted();
      flashSuccess(first);
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
    // P2P receive has no omnibox location picker; never inherit selectedOutputDir.
    let outputDir = settings?.download.default_output_dir || "";

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
      markStarted();
      flashSuccess(false);
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

  function handleMore(action: MoreAction) {
    if (action === "advanced") {
      advancedMode = true;
      return;
    }
    handleHomeModeChange(action);
  }

  // Enter / the arrow: download when the link is already understood, else
  // check it right away instead of waiting for the paste debounce.
  function handleSubmit() {
    if (omniState.kind === "detected") {
      void handleAction();
      return;
    }
    if (omniState.kind === "batch") {
      void handleBatchDownload();
      return;
    }
    const trimmed = url.trim();
    if (!trimmed) return;
    if (isUrl(trimmed) && !/[\s\n]/.test(trimmed)) {
      if (debounceTimer) clearTimeout(debounceTimer);
      omniState = { kind: "detecting" };
      pendingAutoDownload = false;
      void detectPlatform(trimmed);
      return;
    }
    handleAnalyze();
  }

  async function handleDroppedPaths(paths: string[]) {
    const path = paths[0];
    if (!path) return;
    const lower = path.toLowerCase();
    if (lower.endsWith(".torrent")) {
      url = path;
      handleInput();
      return;
    }
    if (lower.endsWith(".txt")) {
      try {
        const urls = await invoke<string[]>("parse_batch_file", { path });
        if (urls.length === 0) {
          showToast("info", $t("omnibox.batch_file_empty"));
        } else if (urls.length === 1) {
          url = urls[0];
          handleInput();
        } else {
          url = urls.join("\n");
          omniState = { kind: "batch", urls };
        }
      } catch (e: any) {
        showToast("error", typeof e === "string" ? e : e.message ?? $t("omnibox.error"));
      }
      return;
    }
    showToast("info", $t("home.drop_unsupported") as string);
  }

  function handleDroppedText(text: string) {
    const trimmed = text.trim();
    if (!trimmed) return;
    url = trimmed;
    handleInput();
    urlInput?.focus();
  }

  // Files arrive through Tauri (the webview never sees their paths); links
  // dragged from a browser arrive as HTML5 text drops.
  onMount(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    try {
      getCurrentWebview()
        .onDragDropEvent((event) => {
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter") dragging = true;
          else if (p.type === "leave") dragging = false;
          else if (p.type === "drop") {
            dragging = false;
            if (p.paths && p.paths.length > 0) void handleDroppedPaths(p.paths);
          }
        })
        .then((fn) => {
          if (disposed) fn();
          else unlisten = fn;
        })
        .catch(() => {});
    } catch {
      // not running inside Tauri (tests, shots): HTML5 handlers below still work
    }

    let depth = 0;
    const isTextDrag = (e: DragEvent) =>
      !!e.dataTransfer && [...e.dataTransfer.types].some((ty) => ty === "text/uri-list" || ty === "text/plain");
    const onEnter = (e: DragEvent) => {
      if (!isTextDrag(e)) return;
      depth++;
      dragging = true;
    };
    const onOver = (e: DragEvent) => {
      if (!isTextDrag(e)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    };
    const onLeave = (e: DragEvent) => {
      if (!isTextDrag(e)) return;
      depth = Math.max(0, depth - 1);
      if (depth === 0) dragging = false;
    };
    const onDrop = (e: DragEvent) => {
      if (!isTextDrag(e)) return;
      e.preventDefault();
      depth = 0;
      dragging = false;
      const text = e.dataTransfer?.getData("text/uri-list") || e.dataTransfer?.getData("text/plain") || "";
      handleDroppedText(text.split("\n").filter((l) => !l.startsWith("#")).join("\n"));
    };
    window.addEventListener("dragenter", onEnter);
    window.addEventListener("dragover", onOver);
    window.addEventListener("dragleave", onLeave);
    window.addEventListener("drop", onDrop);
    return () => {
      disposed = true;
      unlisten?.();
      window.removeEventListener("dragenter", onEnter);
      window.removeEventListener("dragover", onOver);
      window.removeEventListener("dragleave", onLeave);
      window.removeEventListener("drop", onDrop);
      if (successTimer) clearTimeout(successTimer);
    };
  });

  // Loop stays in one fixed slot through every state, so the input never
  // jumps when a link is pasted (Shazam: the hero object changes, the page
  // does not). The pose carries the state; the text next to it says it.
  let homeArt = $derived.by((): HomeArt | null => {
    if (advancedMode) return null;
    if (success) return "success";
    switch (omniState.kind) {
      case "detecting":
      case "preparing":
      case "searching":
      case "search-results": return "analyzing";
      case "error": return "error";
      case "unsupported":
      case "search-empty": return "unsupported";
      default: return "idle";
    }
  });
  let heroSize = $derived(omniState.kind === "idle" && !success ? 136 : 104);
  let busy = $derived(omniState.kind === "detecting" || omniState.kind === "searching" || omniState.kind === "preparing");
  let busyLabel = $derived(
    omniState.kind === "detecting" ? ($t("omnibox.checking") as string)
    : omniState.kind === "searching" ? ($t("omnibox.searching") as string)
    : omniState.kind === "preparing" ? ($t(firstRun ? "omnibox.preparing_first" : "omnibox.preparing") as string)
    : ""
  );
  let errorKind = $derived<ErrorKind>(omniState.kind === "error" ? classifyError(omniState.message) : "generic");

  function errorAction(kind: ErrorKind) {
    if (kind === "login" || kind === "age") goto("/settings?tab=cookies");
    else if (kind === "ffmpeg") goto("/settings");
    else if (kind === "unsupported") openSupportedSites();
    else if (kind === "removed") { handleDismiss(); urlInput?.focus(); }
  }

  async function copyErrorDetails(message: string) {
    try {
      await navigator.clipboard.writeText(message);
      errorCopied = true;
      setTimeout(() => { errorCopied = false; }, 1600);
    } catch {}
  }

  function downloadLabel(info: PlatformInfo): string {
    if (info.content_type === "playlist" && playlistEntries.length > 0) {
      const n = selectedPlaylistItems.size;
      return $t(pluralKey("omnibox.download_playlist", n), { count: n }) as string;
    }
    if (torrentEntries.length > 0) {
      const n = selectedTorrentFiles.size;
      return $t(pluralKey("omnibox.download_files", n), { count: n }) as string;
    }
    if (downloadMode === "audio" || info.content_type === "audio") return $t("omnibox.download_audio") as string;
    if (info.content_type === "image" || info.content_type === "post") return $t("omnibox.download_image") as string;
    if (info.content_type === "video" || info.content_type === "reel" || info.content_type === "short" || info.content_type === "clip") return $t("omnibox.download_video") as string;
    return $t("omnibox.download") as string;
  }
</script>

<div class="home" class:home--advanced={advancedMode}>
  <div class="home-backdrop" aria-hidden="true"></div>
  <h1 class="sr-only">{$t("home.sr_title")}</h1>

  {#if advancedMode}
    <div class="home-column home-column--wide">
      <div class="adv-head">
        <button type="button" class="back-link" onclick={() => { advancedMode = false; }}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 18l-6-6 6-6" /></svg>
          {$t("home.action_simple")}
        </button>
        <h2 class="adv-title">{$t("home.adv_title")}</h2>
      </div>
      <OmniboxAdvanced />
    </div>
  {:else}
    <div class="home-column">
      {#if homeArt}
        <div class="home-art">
          <HomeHero art={homeArt} size={heroSize} celebrate={!!success?.first} />
        </div>
      {/if}

      {#if externalNotice}
        <div class="external-card">
          <span class="external-title">{$t('omnibox.external_url_ready')}</span>
          <span class="external-url">{externalNotice.url}</span>
          <button class="dismiss-btn" onclick={() => { externalNotice = null; }} aria-label={$t('common.close')}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 6L6 18M6 6l12 12" /></svg>
          </button>
        </div>
      {/if}

      <HomeUrlBar
        bind:url
        bind:inputEl={urlInput}
        {busy}
        {busyLabel}
        showSubmit={omniState.kind === "idle"}
        onInput={handleInput}
        onSubmit={handleSubmit}
        onMore={handleMore}
      />

      {#if omniState.kind === "detected"}
        <p class="detected-chip">
          {platformDisplayName(omniState.info.platform)}
          {#if omniState.info.content_type && getContentTypeLabel(omniState.info.content_type) !== $t("omnibox.content_type.unknown")}
            <span class="feedback-sep" aria-hidden="true">&middot;</span>
            {getContentTypeLabel(omniState.info.content_type)}
          {/if}
        </p>
      {/if}

      {#if success}
        <div class="state-line state-line--success" role="status">
          <strong>{success.first ? $t("home.success_first") : $t("home.success_title")}</strong>
          <a href="/downloads" class="quiet-link">{$t("home.success_open")}</a>
        </div>
      {:else if omniState.kind === "idle"}
        {#if firstRun}
          {@const sites = splitLink($t("home.first_sites", { count: (1000).toLocaleString() + "+" }) as string)}
          {@const terms = splitLink($t("home.first_terms") as string)}
          <div class="first-run">
            <p>{sites.before}<button type="button" class="inline-link" onclick={openSupportedSites}>{sites.link}</button>{sites.after}</p>
            <p class="first-terms">{terms.before}<a href="/about/terms" class="inline-link">{terms.link}</a>{terms.after}</p>
          </div>
        {:else if dlStats.totalDownloads > 0}
          <a href="/downloads" class="stats-line">
            {$t("home.saved_line", {
              files: $t(pluralKey("home.saved_count", dlStats.totalDownloads), { count: dlStats.totalDownloads.toLocaleString() }) as string,
              size: formatBytes(dlStats.totalBytes),
            })}
          </a>
        {/if}
      {:else if omniState.kind === "batch"}
        <div class="batch-panel">
          <p class="state-line">{$t(pluralKey("omnibox.batch_detected", omniState.urls.length), { count: omniState.urls.length })}</p>
          <DownloadModeSelector bind:downloadMode />
          <button class="download-primary-btn" onclick={handleBatchDownload}>{$t("omnibox.batch_download_all", { count: omniState.urls.length })}</button>
        </div>
      {:else if omniState.kind === "search-results"}
        <p class="state-line">{$t("omnibox.search_results", { query: url.trim() })}</p>
        <SearchResults results={omniState.results} onSelect={selectSearchResult} />
      {:else if omniState.kind === "search-empty"}
        <div class="state-block" role="status">
          <p class="state-title">{$t("omnibox.search_empty", { query: url.trim() })}</p>
          <p class="state-body">{$t("omnibox.search_empty_hint")}</p>
        </div>
      {:else if omniState.kind === "unsupported"}
        <div class="state-block" role="status">
          <p class="state-title">{$t("omnibox.unsupported")}</p>
          <button type="button" class="secondary-btn" onclick={openSupportedSites}>{$t("omnibox.see_all")}</button>
        </div>
      {:else if omniState.kind === "error"}
        {@const kind = errorKind}
        {@const action = $t(`omnibox.err.${kind}.action`) as string}
        <div class="error-card" role="alert">
          <p class="state-title">{$t(`omnibox.err.${kind}.title`)}</p>
          <p class="state-body">{$t(`omnibox.err.${kind}.body`)}</p>
          <div class="error-actions">
            {#if action}
              <button type="button" class="download-primary-btn download-primary-btn--auto" onclick={() => errorAction(kind)}>{action}</button>
              <button type="button" class="secondary-btn" onclick={handleRetry}>{$t("omnibox.retry")}</button>
            {:else}
              <button type="button" class="download-primary-btn download-primary-btn--auto" onclick={handleRetry}>{$t("omnibox.retry")}</button>
            {/if}
          </div>
          <details class="error-details">
            <summary>{$t("omnibox.error_details")}</summary>
            <div class="error-raw">
              <code>{omniState.message}</code>
              <button type="button" class="quiet-link" onclick={() => omniState.kind === "error" && copyErrorDetails(omniState.message)}>
                {errorCopied ? $t("omnibox.error_copied") : $t("omnibox.error_copy")}
              </button>
            </div>
          </details>
        </div>
      {/if}

      <HomeInspector open={omniState.kind === "detected"} title={$t('home.inspector_title')}>
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
          {@const playlistBlocked = omniState.info.content_type === "playlist" && playlistEntries.length > 0 && selectedPlaylistItems.size === 0}
          {@const torrentBlocked = torrentEntries.length > 0 && selectedTorrentFiles.size === 0}
          {#if omniState.info.platform === "bilibili"}
            <BilibiliPreviewExtras {url} accountSlug={selectedCookieSlug && selectedCookieSlug !== "_anonymous" ? selectedCookieSlug : null} />
          {/if}
          <button class="download-primary-btn" disabled={playlistBlocked || torrentBlocked} onclick={handleAction}>{downloadLabel(omniState.info)}</button>
          {#if omniState.info.platform !== "direct_file" && omniState.info.platform !== "p2p"}
            <details class="options-panel">
              <summary class="options-toggle">{$t('omnibox.options')}</summary>
              <div class="options-content">
                <DownloadModeSelector bind:downloadMode onChange={() => { selectedFormatId = null; }} />
                <QualityPicker bind:selectedQuality selectedFormatId {availableHeights} {hasAudioOnly} />
                <OutputLocationPicker bind:selectedOutputDir />
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
      </HomeInspector>
    </div>
  {/if}

  <HomeDropOverlay visible={dragging} />

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

</div>

<style>
  /* detection line under the omnibox */
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

  /* ---------- layout ---------- */
  .home {
    position: relative;
    min-height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: clamp(28px, 17vh, 190px) 20px 48px;
    isolation: isolate;
  }

  .home--advanced {
    padding-top: clamp(24px, 6vh, 64px);
  }

  /* warmth that follows any theme: an accent glow + a faint dot grid, both
     static (21st Radial Glow #7422 + Animated Grid #29376, motion removed) */
  .home-backdrop {
    position: absolute;
    inset: 0;
    z-index: -1;
    pointer-events: none;
    overflow: hidden;
  }

  .home-backdrop::before {
    content: "";
    position: absolute;
    left: 50%;
    top: clamp(28px, 17vh, 190px);
    width: min(820px, 110%);
    aspect-ratio: 1.5;
    transform: translate(-50%, -38%);
    border-radius: 50%;
    background: radial-gradient(closest-side, color-mix(in srgb, var(--accent) 17%, transparent), transparent 72%);
  }

  .home-backdrop::after {
    content: "";
    position: absolute;
    inset: 0;
    background-image: radial-gradient(color-mix(in srgb, var(--text) 9%, transparent) 1px, transparent 1.2px);
    background-size: 22px 22px;
    -webkit-mask-image: radial-gradient(ellipse 55% 42% at 50% 30%, #000 20%, transparent 75%);
    mask-image: radial-gradient(ellipse 55% 42% at 50% 30%, #000 20%, transparent 75%);
  }

  .home-column {
    width: 100%;
    max-width: 620px;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 14px;
    animation: blur-fade 320ms var(--ease-out);
  }

  .home-column--wide {
    max-width: 680px;
  }

  .home-art {
    display: flex;
    justify-content: center;
    align-items: flex-end;
    height: 136px;
    margin-bottom: 4px;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  /* ---------- lines under the input ---------- */
  .first-run {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    text-align: center;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .first-run p { margin: 0; }

  .first-terms {
    font-size: 12px;
    color: var(--text-dim);
  }

  .inline-link {
    display: inline-block;
    padding: 3px 0;
    line-height: 1.2;
    border: none;
    background: none;
    font: inherit;
    color: inherit;
    text-decoration: underline;
    text-decoration-color: color-mix(in srgb, currentColor 40%, transparent);
    text-underline-offset: 3px;
    cursor: pointer;
  }

  .inline-link:hover { color: var(--text); text-decoration-color: currentColor; }

  .stats-line {
    align-self: center;
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    text-decoration: none;
    padding: 2px 8px;
    border-radius: 999px;
  }

  .stats-line:hover { color: var(--text-muted); background: var(--fill-1); }

  .state-line {
    margin: 0;
    text-align: center;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .state-line--success {
    display: flex;
    justify-content: center;
    align-items: baseline;
    gap: 10px;
    animation: blur-fade 240ms var(--ease-out);
  }

  .state-line--success strong {
    color: var(--text);
    font-weight: 600;
  }

  .quiet-link {
    padding: 0;
    border: none;
    background: none;
    font: inherit;
    font-size: var(--text-sm);
    color: var(--accent-text, var(--accent));
    text-decoration: none;
    cursor: pointer;
  }

  .quiet-link:hover { text-decoration: underline; }

  .detected-chip {
    margin: 0 0 -4px;
    align-self: center;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-muted);
  }

  .state-block,
  .error-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    text-align: center;
  }

  .state-title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .state-body {
    margin: 0;
    max-width: 52ch;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .error-actions {
    display: flex;
    gap: 8px;
    margin-top: 8px;
  }

  .secondary-btn {
    height: 36px;
    padding: 0 14px;
    border: none;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
  }

  .secondary-btn:hover { background: var(--fill-2); }

  .secondary-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .download-primary-btn--auto {
    width: auto;
    height: 36px;
    font-size: var(--text-sm);
  }

  .error-details {
    width: 100%;
    margin-top: 4px;
    font-size: 12px;
    color: var(--text-dim);
  }

  .error-details summary {
    cursor: pointer;
    list-style: none;
  }

  .error-details summary::-webkit-details-marker { display: none; }

  .error-raw {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    margin-top: 8px;
    padding: 10px 12px;
    text-align: left;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }

  .error-raw code {
    flex: 1;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text-muted);
    word-break: break-word;
  }

  .batch-panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .external-card {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 14px;
    border-radius: var(--radius-lg);
    background: var(--accent-soft);
    font-size: var(--text-sm);
  }

  .external-title { font-weight: 600; color: var(--text); white-space: nowrap; }

  .external-url {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-muted);
  }

  .dismiss-btn {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }

  .dismiss-btn:hover { color: var(--text); background: var(--fill-2); }

  /* ---------- advanced ---------- */
  .adv-head {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    margin-bottom: 8px;
  }

  .back-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px 4px 4px;
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-muted);
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
  }

  .back-link:hover { color: var(--text); background: var(--fill-1); }

  .adv-title {
    margin: 0;
    font-family: var(--font-display);
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  /* 21st Blur Fade (#1079), CSS only */
  @keyframes blur-fade {
    from { opacity: 0; transform: translateY(6px); filter: blur(4px); }
    to { opacity: 1; transform: none; filter: none; }
  }

  @media (prefers-reduced-motion: reduce) {
    .home-column,
    .state-line--success {
      animation: none;
    }

    .feedback-spinner {
      animation-duration: 1.5s;
    }
  }
</style>
