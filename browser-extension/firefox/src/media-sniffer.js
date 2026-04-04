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

const BLOCKED_HOSTS = [
  "google-analytics.com",
  "googletagmanager.com",
  "facebook.com",
  "doubleclick.net",
  "analytics",
  "ads.google.com",
  "ad.doubleclick.net",
  "adservice.google.com",
  "pagead2.googlesyndication.com",
  "cdn.mxpnl.com",
  "stats.wp.com",
  "pixel.facebook.com",
  "connect.facebook.net",
  "scorecardresearch.com",
  "hotjar.com",
  "sentry.io",
  "newrelic.com",
  "nr-data.net",
  "segment.io",
  "segment.com",
  "amplitude.com",
  "mixpanel.com",
  "cdn.heapanalytics.com",
];

const PLATFORM_CDN_HOSTS = [
  "cdninstagram.com",
  "fbcdn.net",
  "instagram.fna.fbcdn.net",
  "tiktokcdn.com",
  "musical.ly",
  "tiktokcdn-us.com",
];

const MIN_CONTENT_LENGTH = 50 * 1024;

const detectedMedia = new Map();
const pendingRequests = new Map();

export function getDetectedMedia(tabId) {
  return detectedMedia.get(tabId) || new Map();
}

export function clearTabMedia(tabId) {
  detectedMedia.delete(tabId);
}

export function getMediaCount(tabId) {
  const media = detectedMedia.get(tabId);
  return media ? media.size : 0;
}

function isBlockedHost(url) {
  try {
    const host = new URL(url).hostname;
    return BLOCKED_HOSTS.some(b => host.includes(b));
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

function persistMedia(tabId) {
  const media = detectedMedia.get(tabId);
  if (!media || media.size === 0) return;
  const arr = Array.from(media.entries());
  chrome.storage.local.set({ [`media_${tabId}`]: arr }).catch(() => {});
}

export async function restoreMedia() {
  try {
    const data = await chrome.storage.local.get(null);
    for (const [key, value] of Object.entries(data)) {
      if (!key.startsWith("media_")) continue;
      const tabId = parseInt(key.replace("media_", ""));
      if (isNaN(tabId)) continue;
      const map = new Map(value);
      detectedMedia.set(tabId, map);
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
      if (isBlockedHost(url)) return;

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

      if (contentLength > 0 && contentLength < MIN_CONTENT_LENGTH) {
        if (!url.includes(".m3u8") && !url.includes(".mpd") &&
            !contentType.includes("mpegurl") && !contentType.includes("dash")) {
          pendingRequests.delete(details.requestId);
          return;
        }
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

      if (!detectedMedia.has(details.tabId)) {
        detectedMedia.set(details.tabId, new Map());
      }
      detectedMedia.get(details.tabId).set(url, entry);
      persistMedia(details.tabId);

      onMediaDetected(details.tabId, entry);
    },
    { urls: ["http://*/*", "https://*/*"] },
    ["responseHeaders"]
  );

  chrome.webRequest.onErrorOccurred.addListener(
    (details) => { pendingRequests.delete(details.requestId); },
    { urls: ["http://*/*", "https://*/*"] }
  );

  chrome.tabs.onRemoved.addListener((tabId) => {
    detectedMedia.delete(tabId);
    chrome.storage.local.remove(`media_${tabId}`).catch(() => {});
  });

  chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
    if (changeInfo.url) {
      detectedMedia.delete(tabId);
      chrome.storage.local.remove(`media_${tabId}`).catch(() => {});
    }
  });
}
