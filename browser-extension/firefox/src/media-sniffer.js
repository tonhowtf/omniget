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

import { formatBytes } from "./format-size.js";
import { isDashManifest, isHlsManifest } from "./sniffer-filters.js";
import { isSnifferEnabled, loadSnifferState } from "./sniffer-toggle.js";
import { applyRegexRules, evaluateCapture } from "./capture-rules.js";
import { getCaptureRules, getCompiledRegexRules } from "./capture-store.js";
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

// Entries are normally removed in onHeadersReceived / onErrorOccurred, but a
// redirected or cancelled request can leave one behind forever. Cap the map and
// drop the oldest half when it fills up — insertion order is chronological.
export const MAX_PENDING_REQUESTS = 4096;

export function trimPendingRequests(pending, max = MAX_PENDING_REQUESTS) {
  if (pending.size <= max) return 0;
  const dropCount = Math.floor(pending.size / 2);
  let dropped = 0;
  for (const key of pending.keys()) {
    if (dropped >= dropCount) break;
    pending.delete(key);
    dropped++;
  }
  return dropped;
}

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
  if (isHlsManifest(url, contentType)) return "hls";
  if (isDashManifest(url, contentType)) return "dash";
  if (ct.includes("video/")) return "video";
  if (ct.includes("audio/")) return "audio";
  return "media";
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

// Folds a stored page back into whatever is already in memory. Restoring by
// replacing the page's Map would lose live detections: the listeners are
// registered synchronously and start recording the moment a request wakes the
// service worker, while this reads every storage key and lands much later. A
// live entry is also the fresher one, so it wins on conflict.
export function mergeStoredMedia(target, pageKey, storedEntries) {
  let existing = target.get(pageKey);
  if (!existing) {
    existing = new Map();
    target.set(pageKey, existing);
  }
  let added = 0;
  for (const [url, entry] of storedEntries) {
    if (existing.has(url)) continue;
    existing.set(url, entry);
    added++;
  }
  return added;
}

export async function restoreMedia() {
  try {
    const data = await chrome.storage.local.get(null);
    const { valid, stale } = classifyStoredPages(data, Date.now());
    for (const entry of valid) {
      try {
        mergeStoredMedia(detectedMedia, entry.pageKey, entry.media);
      } catch {
        stale.push(entry.storageKey);
      }
    }
    if (stale.length > 0) {
      await chrome.storage.local.remove(stale).catch(() => {});
    }
  } catch {}
}

// Must be called synchronously while the service worker module is evaluated.
// In MV3 an event only wakes a terminated worker back up if its listener was
// registered during that first synchronous pass, so registration can never wait
// on a promise — the enable check lives inside each handler instead.
// Files the entry under the tab's page key and persists the page. Shared by the
// webRequest sniffer and by deep search, which finds media the network layer
// never sees. Resolves to false when the tab has no usable page key (an
// internal page, or a tab that closed while we were looking it up).
export async function recordDetectedMedia(tabId, entry) {
  if (!entry?.url) return false;
  const pageKey = await resolveTabPageKey(tabId);
  if (!pageKey) return false;
  if (!detectedMedia.has(pageKey)) {
    detectedMedia.set(pageKey, new Map());
  }
  detectedMedia.get(pageKey).set(entry.url, entry);
  persistPage(pageKey);
  return true;
}

export function registerSnifferListeners(onMediaDetected) {
  chrome.webRequest.onSendHeaders.addListener(
    (details) => {
      if (!isSnifferEnabled()) return;
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
      trimPendingRequests(pendingRequests);
    },
    { urls: ["http://*/*", "https://*/*"] },
    ["requestHeaders", "extraHeaders"]
  );

  chrome.webRequest.onHeadersReceived.addListener(
    (details) => {
      if (!isSnifferEnabled()) return;
      if (details.tabId < 0) return;
      if (details.statusCode < 200 || details.statusCode >= 300) return;

      const url = details.url;
      if (isBlockedHost(url) || isBlockedPath(url)) {
        pendingRequests.delete(details.requestId);
        return;
      }

      const contentType = getContentType(details.responseHeaders);
      const contentLength = getContentLength(details.responseHeaders);

      // The user's regex table gets the first and last word. A block rule kills
      // the request outright; an accept rule bypasses every heuristic below,
      // because its whole purpose is to catch what the heuristics miss — an
      // Instagram byte-range fragment, say, which isPlatformCdnFragment drops
      // and which the accept rule rewrites into the URL of the whole file.
      const regexResult = applyRegexRules(url, getCompiledRegexRules());
      if (regexResult?.action === "block") {
        pendingRequests.delete(details.requestId);
        return;
      }

      const captureUrl = regexResult?.action === "accept" ? regexResult.url : url;

      if (!regexResult) {
        if (isPlatformCdnFragment(url, contentType, contentLength)) {
          pendingRequests.delete(details.requestId);
          return;
        }

        const tables = getCaptureRules();
        const verdict = evaluateCapture({
          url,
          contentType,
          contentLength,
          extensions: tables.extensions,
          contentTypes: tables.contentTypes,
        });
        if (!verdict.capture) {
          pendingRequests.delete(details.requestId);
          return;
        }

        if (isHlsSegment(url, contentType)) {
          pendingRequests.delete(details.requestId);
          return;
        }
      }

      const reqData = pendingRequests.get(details.requestId);
      pendingRequests.delete(details.requestId);

      const mediaType = getMediaType(contentType, captureUrl);

      const entry = {
        url: captureUrl,
        contentType,
        contentLength,
        mediaType,
        sizeText: formatBytes(contentLength),
        detectedAt: Date.now(),
        tabId: details.tabId,
        requestHeaders: reqData?.requestHeaders || [],
        responseHeaders: details.responseHeaders || [],
      };

      // isSnifferEnabled() above is the optimistic fast path; this is the
      // authoritative one. Nothing is filed until the stored flag is known,
      // so a boot-window request cannot be kept against the user's setting.
      loadSnifferState().then((allowed) => {
        if (!allowed) return null;
        return recordDetectedMedia(details.tabId, entry).then((recorded) => {
          if (recorded) onMediaDetected(details.tabId, entry);
        });
      }).catch(() => {});
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
