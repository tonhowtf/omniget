const MEDIA_CONTENT_TYPES = [
  "video/mp4",
  "video/webm",
  "video/x-flv",
  "video/ogg",
  "video/x-matroska",
  "video/3gpp",
  "video/mpeg",
  "video/x-msvideo",
  "video/x-ms-wmv",
  "video/quicktime",
  "audio/mpeg",
  "audio/ogg",
  "audio/mp4",
  "audio/webm",
  "audio/aac",
  "audio/wav",
  "audio/x-wav",
  "audio/flac",
  "audio/x-flac",
  "audio/x-m4a",
  "audio/x-ms-wma",
  "audio/opus",
  "application/vnd.apple.mpegurl",
  "application/x-mpegurl",
  "application/dash+xml",
  "application/f4m+xml",
];

const MEDIA_EXTENSIONS = [
  ".mp4", ".webm", ".m3u8", ".mpd",
  ".flv", ".ogg", ".mp3", ".m4a", ".m4v",
  ".mkv", ".avi", ".mov", ".wmv",
  ".wav", ".flac", ".aac", ".opus",
  ".3gp", ".mpg", ".mpeg", ".divx",
  ".f4m", ".f4v", ".ts",
];

const PLATFORM_CDN_HOSTS = [
  "cdninstagram.com",
  "fbcdn.net",
  "instagram.fna.fbcdn.net",
  "tiktokcdn.com",
  "musical.ly",
  "tiktokcdn-us.com",
];

const BLOCKED_PATH_PATTERNS = [
  /\/analytics(\/|\?|$)/i,
  /\/pixel(\/|\?|\.|$)/i,
  /\/collect(\/|\?|$)/i,
  /\/beacon(\/|\?|$)/i,
  /\/telemetry(\/|\?|$)/i,
  /\/track(\/|\?|$|er)/i,
  /1x1\.gif$/i,
  /\/spacer\.gif$/i,
  /\/transparent\.gif$/i,
];

import { shouldDropBySize } from "./sniffer-filters.js";
import {
  classifyStoredPages,
  normalizePageKey,
  storageKeyForPage,
} from "./sniffer-storage.js";
import {
  DEFAULT_BLOCKED_HOSTS,
  USER_BLOCKED_HOSTS_KEY,
  mergeBlocklists,
  isUrlHostBlocked,
} from "./blocked-hosts.js";

let activeBlocklist = mergeBlocklists(DEFAULT_BLOCKED_HOSTS, []);

async function loadUserBlocklist() {
  try {
    const data = await chrome.storage.local.get(USER_BLOCKED_HOSTS_KEY);
    const user = data?.[USER_BLOCKED_HOSTS_KEY];
    activeBlocklist = mergeBlocklists(DEFAULT_BLOCKED_HOSTS, user);
  } catch {
    activeBlocklist = mergeBlocklists(DEFAULT_BLOCKED_HOSTS, []);
  }
}

export function getActiveBlocklist() {
  return activeBlocklist.slice();
}

export async function refreshUserBlocklist() {
  await loadUserBlocklist();
  return getActiveBlocklist();
}

if (typeof chrome !== "undefined" && chrome?.storage?.local) {
  loadUserBlocklist();
  if (chrome.storage.onChanged && typeof chrome.storage.onChanged.addListener === "function") {
    chrome.storage.onChanged.addListener((changes, areaName) => {
      if (areaName !== "local") return;
      if (!(USER_BLOCKED_HOSTS_KEY in changes)) return;
      loadUserBlocklist();
    });
  }
}

const detectedMedia = new Map();
const pendingRequests = new Map();
const tabPageKeys = new Map();

export function getDetectedMedia(tabId) {
  const pageKey = tabPageKeys.get(tabId);
  if (!pageKey) return new Map();
  return detectedMedia.get(pageKey) || new Map();
}

export function getDetectedMediaForUrl(url) {
  const pageKey = normalizePageKey(url);
  if (!pageKey) return new Map();
  return detectedMedia.get(pageKey) || new Map();
}

export function clearTabMedia(tabId) {
  const pageKey = tabPageKeys.get(tabId);
  if (!pageKey) return;
  detectedMedia.delete(pageKey);
  chrome.storage.local.remove(storageKeyForPage(pageKey)).catch(() => {});
}

export function getMediaCount(tabId) {
  const pageKey = tabPageKeys.get(tabId);
  if (!pageKey) return 0;
  const media = detectedMedia.get(pageKey);
  return media ? media.size : 0;
}

export function getPageKeyForTab(tabId) {
  return tabPageKeys.get(tabId) || null;
}

export function getMediaCountForPage(pageKey) {
  if (!pageKey) return 0;
  const media = detectedMedia.get(pageKey);
  return media ? media.size : 0;
}

function isBlockedHost(url) {
  return isUrlHostBlocked(url, activeBlocklist);
}

function isBlockedPath(url) {
  try {
    const u = new URL(url);
    const target = u.pathname + u.search;
    return BLOCKED_PATH_PATTERNS.some(rx => rx.test(target));
  } catch { return false; }
}

function isMediaByExtension(url) {
  try {
    const path = new URL(url).pathname.toLowerCase();
    return MEDIA_EXTENSIONS.some(ext => path.includes(ext));
  } catch { return false; }
}

function isMediaByContentType(contentType) {
  if (!contentType) return false;
  const lower = contentType.toLowerCase();
  return MEDIA_CONTENT_TYPES.some(mt => lower.includes(mt));
}

function getContentLength(headers) {
  const header = headers?.find(h => h.name.toLowerCase() === "content-length");
  return header ? parseInt(header.value, 10) : 0;
}

function getContentType(headers) {
  const header = headers?.find(h => h.name.toLowerCase() === "content-type");
  return header?.value || "";
}

function getMediaType(contentType, url) {
  const ct = contentType.toLowerCase();
  if (ct.includes("mpegurl") || url.includes(".m3u8")) return "hls";
  if (ct.includes("dash") || url.includes(".mpd")) return "dash";
  if (ct.includes("video/")) return "video";
  if (ct.includes("audio/")) return "audio";
  return "media";
}

function formatSize(bytes) {
  if (!bytes || bytes <= 0) return "";
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function isHlsSegment(url, contentType) {
  const ct = contentType.toLowerCase();
  if (ct.includes("mp2t") || ct.includes("mpeg2-ts") || ct.includes("mpeg-ts") || ct.includes("video/mp2t")) {
    return true;
  }
  try {
    const path = new URL(url).pathname.toLowerCase();
    if (path.match(/\/seg-\d+/)) return true;
    if (path.match(/\/segment\d+/)) return true;
    if (path.match(/\/chunk-/)) return true;
    if (path.match(/\/media_\d+/)) return true;
    if (path.endsWith(".ts") && !path.endsWith(".m3u8.ts")) {
      const parts = path.split("/");
      const filename = parts[parts.length - 1];
      if (/^\d+\.ts$/.test(filename) || /^seg[-_]\d+\.ts$/.test(filename)) return true;
      if (/video\d+\.ts/.test(filename)) return true;
    }
  } catch {}
  return false;
}

function isPlatformCdnFragment(url, contentType, contentLength) {
  try {
    const host = new URL(url).hostname;
    const isPlatformCdn = PLATFORM_CDN_HOSTS.some(cdn => host.includes(cdn));
    if (!isPlatformCdn) return false;
    if (contentLength > 0 && contentLength < 5 * 1024 * 1024) return true;
    const ct = contentType.toLowerCase();
    if (ct.includes("octet-stream")) return true;
    return false;
  } catch { return false; }
}

function persistPage(pageKey) {
  const media = detectedMedia.get(pageKey);
  if (!media || media.size === 0) return;
  const arr = Array.from(media.entries());
  const payload = { media: arr, savedAt: Date.now() };
  chrome.storage.local.set({ [storageKeyForPage(pageKey)]: payload }).catch(() => {});
}

async function resolveTabPageKey(tabId) {
  if (tabPageKeys.has(tabId)) return tabPageKeys.get(tabId);
  try {
    const tab = await chrome.tabs.get(tabId);
    const pageKey = normalizePageKey(tab?.url);
    if (pageKey) tabPageKeys.set(tabId, pageKey);
    return pageKey;
  } catch {
    return null;
  }
}

export async function restoreMedia() {
  try {
    const data = await chrome.storage.local.get(null);
    const { valid, stale } = classifyStoredPages(data, Date.now());
    for (const entry of valid) {
      try {
        detectedMedia.set(entry.pageKey, new Map(entry.media));
      } catch {
        stale.push(entry.storageKey);
      }
    }
    if (stale.length > 0) {
      await chrome.storage.local.remove(stale).catch(() => {});
    }
  } catch {}
}

export function registerSnifferListeners(onMediaDetected) {
  chrome.webRequest.onSendHeaders.addListener(
    (details) => {
      if (details.tabId < 0) return;
      if (details.method !== "GET") {
        try {
          const host = new URL(details.url).hostname;
          if (!host.includes("googlevideo")) return;
        } catch { return; }
      }

      pendingRequests.set(details.requestId, {
        requestHeaders: details.requestHeaders || [],
        tabId: details.tabId,
      });
    },
    { urls: ["http://*/*", "https://*/*"] },
    ["requestHeaders", "extraHeaders"]
  );

  chrome.webRequest.onHeadersReceived.addListener(
    (details) => {
      if (details.tabId < 0) return;
      if (details.statusCode < 200 || details.statusCode >= 300) return;

      const url = details.url;
      if (isBlockedHost(url) || isBlockedPath(url)) {
        pendingRequests.delete(details.requestId);
        return;
      }

      const contentType = getContentType(details.responseHeaders);
      const contentLength = getContentLength(details.responseHeaders);

      if (isPlatformCdnFragment(url, contentType, contentLength)) {
        pendingRequests.delete(details.requestId);
        return;
      }
      const isOctetStream = contentType.toLowerCase().includes("application/octet-stream");
      const isMedia = isMediaByContentType(contentType) || isMediaByExtension(url);

      if (!isMedia && !(isOctetStream && isMediaByExtension(url))) {
        pendingRequests.delete(details.requestId);
        return;
      }

      if (isHlsSegment(url, contentType)) {
        pendingRequests.delete(details.requestId);
        return;
      }

      if (shouldDropBySize(url, contentType, contentLength)) {
        pendingRequests.delete(details.requestId);
        return;
      }

      const reqData = pendingRequests.get(details.requestId);
      pendingRequests.delete(details.requestId);

      const mediaType = getMediaType(contentType, url);

      const entry = {
        url,
        contentType,
        contentLength,
        mediaType,
        sizeText: formatSize(contentLength),
        detectedAt: Date.now(),
        tabId: details.tabId,
        requestHeaders: reqData?.requestHeaders || [],
        responseHeaders: details.responseHeaders || [],
      };

      resolveTabPageKey(details.tabId).then((pageKey) => {
        if (!pageKey) return;
        if (!detectedMedia.has(pageKey)) {
          detectedMedia.set(pageKey, new Map());
        }
        detectedMedia.get(pageKey).set(url, entry);
        persistPage(pageKey);
        onMediaDetected(details.tabId, entry);
      });
    },
    { urls: ["http://*/*", "https://*/*"] },
    ["responseHeaders"]
  );

  chrome.webRequest.onErrorOccurred.addListener(
    (details) => { pendingRequests.delete(details.requestId); },
    { urls: ["http://*/*", "https://*/*"] }
  );

  chrome.tabs.onRemoved.addListener((tabId) => {
    tabPageKeys.delete(tabId);
  });

  chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
    if (changeInfo.url) {
      const pageKey = normalizePageKey(changeInfo.url);
      if (pageKey) {
        tabPageKeys.set(tabId, pageKey);
      } else {
        tabPageKeys.delete(tabId);
      }
    } else if (tab?.url && !tabPageKeys.has(tabId)) {
      const pageKey = normalizePageKey(tab.url);
      if (pageKey) tabPageKeys.set(tabId, pageKey);
    }
  });
}
