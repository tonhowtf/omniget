import { extractCookiesForPlatform } from "./cookies.js";
import { detectSupportedMediaUrl } from "./detect.js";
import { createActionFeedbackController } from "./action-feedback.js";
import { registerSnifferListeners, getMediaCount, getMediaCountForPage, getDetectedMedia, getDetectedMediaForUrl, getPageKeyForTab, restoreMedia } from "./media-sniffer.js";
import { summarizeCookies } from "./cookie-summary.js";
import { loadSnifferState, isSnifferEnabled, setSnifferEnabled } from "./sniffer-toggle.js";
import { registerContextMenu, getContextMenuId } from "./context-menu.js";
import { openOmnigetScheme } from "./send-via-scheme.js";
import { sendViaBridge, sendCookiesViaBridge } from "./bridge-client.js";

const INSTALL_URL = "https://github.com/tonhowtf/omniget/releases/latest";
const PROTOCOL_VERSION = 1;

function getIconPath(iconSet) {
  return {
    16: chrome.runtime.getURL(iconSet[16]),
    24: chrome.runtime.getURL(iconSet[24]),
    32: chrome.runtime.getURL(iconSet[32]),
    48: chrome.runtime.getURL(iconSet[48]),
  };
}

const ACTIVE_ICON_PATHS = Object.freeze({
  16: "icons/active-16.png",
  24: "icons/active-24.png",
  32: "icons/active-32.png",
  48: "icons/active-48.png",
});

const INACTIVE_ICON_PATHS = Object.freeze({
  16: "icons/inactive-16.png",
  24: "icons/inactive-24.png",
  32: "icons/inactive-32.png",
  48: "icons/inactive-48.png",
});

const actionFeedback = createActionFeedbackController({
  setBadgeText: (details) => chrome.action.setBadgeText(details),
  setBadgeBackgroundColor: (details) => chrome.action.setBadgeBackgroundColor(details),
});

let snifferRegistered = false;

loadSnifferState().then(async (enabled) => {
  await restoreMedia();
  if (enabled) {
    registerSnifferListeners(onMediaDetected);
    snifferRegistered = true;
  }
});

chrome.runtime.onInstalled.addListener((details) => {
  registerContextMenu();
  refreshActiveTab().catch(() => {});
  // First install: surface the pairing page so the user can connect to the
  // desktop app without having to dig through chrome://extensions.
  if (details?.reason === "install" && typeof chrome.runtime.openOptionsPage === "function") {
    chrome.runtime.openOptionsPage().catch(() => {});
  }
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId !== getContextMenuId()) return;

  const url = info.linkUrl || info.srcUrl;
  if (!url) return;

  const result = await handleSendToApp({
    type: "sendToOmniGet",
    url,
    platform: "generic",
    referer: tab?.url || "",
  });

  if (result.ok) {
    actionFeedback.showSuccessBadge(tab?.id);
  }
});

chrome.runtime.onStartup.addListener(() => {
  refreshActiveTab().catch(() => {});
});

if (chrome.commands && chrome.commands.onCommand) {
  chrome.commands.onCommand.addListener(async (command) => {
    if (command !== "send-to-omniget") return;
    try {
      const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
      if (!tab?.url) return;
      const detected = detectSupportedMediaUrl(tab.url);
      if (detected?.supported) {
        const result = await handleSendToApp({
          type: "sendToOmniGet",
          url: tab.url,
          platform: detected.platform,
          referer: tab.url,
        });
        if (result?.ok && tab.id !== undefined) {
          actionFeedback.showSuccessBadge(tab.id);
        }
        return;
      }
      if (chrome.action && typeof chrome.action.openPopup === "function") {
        try { await chrome.action.openPopup(); } catch {}
      }
    } catch (error) {
      console.error("[OmniGet] command handler failed:", error);
    }
  });
}

chrome.tabs.onActivated.addListener(() => {
  refreshActiveTab().catch(() => {});
});

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (!changeInfo.url && !changeInfo.status) {
    return;
  }
  if (!tab?.url) {
    return;
  }
  refreshTabAction(tabId, tab).catch((error) => {
    console.error("[OmniGet] Failed to refresh tab action:", error);
  });
});

chrome.windows.onFocusChanged.addListener((windowId) => {
  if (windowId !== chrome.windows.WINDOW_ID_NONE) {
    refreshActiveTab().catch(() => {});
  }
});

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (msg.type === "getDetectedMedia") {
    chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
      const tabId = tabs[0]?.id;
      const pageUrl = tabs[0]?.url;
      if (!tabId) { sendResponse({ media: [], snifferEnabled: isSnifferEnabled() }); return; }

      const media = pageUrl
        ? getDetectedMediaForUrl(pageUrl)
        : getDetectedMedia(tabId);
      const list = Array.from(media.values()).sort((a, b) => b.detectedAt - a.detectedAt);

      const pageDetected = detectSupportedMediaUrl(pageUrl);

      sendResponse({
        media: list,
        pageDetected,
        snifferEnabled: isSnifferEnabled(),
        tabUrl: pageUrl,
      });
    });
    return true;
  }

  if (msg.type === "toggleSniffer") {
    setSnifferEnabled(msg.enabled).then((result) => {
      const effective = isSnifferEnabled();
      if (effective && !snifferRegistered) {
        registerSnifferListeners(onMediaDetected);
        snifferRegistered = true;
      }
      sendResponse({
        ok: result?.ok !== false,
        enabled: effective,
        reason: result?.reason,
      });
    });
    return true;
  }

  if (msg.type === "sendToOmniGet") {
    handleSendToApp(msg).then(sendResponse);
    return true;
  }
});

function onMediaDetected(tabId, _entry) {
  if (!isSnifferEnabled()) return;
  updateBadge(tabId);
  const pageKey = getPageKeyForTab(tabId);
  if (!pageKey) return;
  const count = getMediaCountForPage(pageKey);
  chrome.runtime.sendMessage({
    type: "media-detected",
    pageKey,
    count,
  }).catch(() => {});
}

function updateBadge(tabId) {
  const count = getMediaCount(tabId);
  chrome.action.setBadgeText({
    tabId,
    text: count > 0 ? String(count) : "",
  }).catch(() => {});
  chrome.action.setBadgeBackgroundColor({
    tabId,
    color: "#F04E23",
  }).catch(() => {});
}

async function handleSendToApp(msg) {
  const url = msg.url;
  const platform = msg.platform || "generic";

  let pageTitle = "";
  let pageThumbnail = "";
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    pageTitle = tab?.title || "";
    pageThumbnail = tab?.favIconUrl || "";
  } catch {}

  let cookies = null;
  try {
    const platformCookies = await extractCookiesForPlatform(platform);
    if (platformCookies && platformCookies.length > 0) {
      cookies = platformCookies;
    } else {
      const cookieMap = new Map();

      const cdnCookies = await chrome.cookies.getAll({ url });
      for (const c of cdnCookies) {
        cookieMap.set(`${c.domain}:${c.name}`, c);
      }

      if (msg.referer) {
        try {
          const pageCookies = await chrome.cookies.getAll({ url: msg.referer });
          for (const c of pageCookies) {
            cookieMap.set(`${c.domain}:${c.name}`, c);
          }
        } catch {}
      }

      if (cookieMap.size > 0) {
        cookies = [...cookieMap.values()].map(c => ({
          domain: c.domain,
          httpOnly: c.httpOnly,
          path: c.path,
          secure: c.secure,
          expires: c.expirationDate ? Math.floor(c.expirationDate) : 0,
          name: c.name,
          value: c.value,
          hostOnly: c.hostOnly,
          sameSite: c.sameSite,
        }));
      }
    }
  } catch {}

  const message = { type: "enqueue", url, protocolVersion: PROTOCOL_VERSION };
  if (cookies) message.cookies = cookies;
  if (msg.referer) message.referer = msg.referer;
  if (msg.title) message.title = msg.title;
  else if (pageTitle) message.title = pageTitle;
  if (msg.thumbnail) message.thumbnail = msg.thumbnail;
  else if (pageThumbnail) message.thumbnail = pageThumbnail;
  if (msg.mediaType) message.mediaType = msg.mediaType;
  if (msg.contentType) message.contentType = msg.contentType;
  if (msg.headers) message.headers = msg.headers;
  if (typeof msg.openApp === "boolean") message.openApp = msg.openApp;
  message.pageUrl = msg.referer || "";
  message.userAgent = navigator.userAgent;

  try {
    await chrome.storage.local.set({
      last_download_metadata: {
        url,
        referer: msg.referer || "",
        headers: msg.headers || {},
        cookies: cookies || [],
        userAgent: navigator.userAgent,
        timestamp: Date.now(),
      },
    }).catch(() => {});
  } catch {}

  const cookieSummary = summarizeCookies(cookies);

  // Primary path: localhost HTTP bridge (no extension-ID dependency, full
  // cookie + metadata payload).
  const bridgeResult = await sendViaBridge(message);
  if (bridgeResult?.ok) {
    return { ok: true, viaBridge: true, cookieSummary };
  }

  // Fallback: omniget:// scheme handler. The desktop app is launched (or
  // brought to focus) and the URL is queued, but cookies aren't forwarded
  // — the user can pair the bridge from the extension's options page to
  // get the full experience.
  const schemeResult = await openOmnigetScheme(url);
  if (schemeResult?.ok) {
    return { ok: true, viaScheme: true, cookieSummary, bridgeReason: bridgeResult?.reason };
  }
  return {
    ok: false,
    error: bridgeResult?.message || schemeResult?.message || "OmniGet is not reachable",
    bridgeReason: bridgeResult?.reason,
    schemeReason: schemeResult?.reason,
  };
}

async function refreshActiveTab() {
  const [tab] = await chrome.tabs.query({
    active: true,
    lastFocusedWindow: true,
  });

  if (tab?.id !== undefined) {
    await refreshTabAction(tab.id, tab);
  }
}

async function refreshTabAction(tabId, tab) {
  if (!tab?.url) {
    return;
  }

  const detected = detectSupportedMediaUrl(tab.url);
  const supported = Boolean(detected?.supported);
  const mediaCount = getMediaCount(tabId);

  try {
    const iconSet = supported ? ACTIVE_ICON_PATHS : INACTIVE_ICON_PATHS;
    await chrome.action.setIcon({ tabId, path: getIconPath(iconSet) });
  } catch (error) {
    if (isTabGoneError(error)) return;
  }

  if (mediaCount > 0) {
    updateBadge(tabId);
  } else {
    try { await actionFeedback.clearBadge(tabId); } catch {}
  }
}

function isTabGoneError(error) {
  const msg = error instanceof Error ? error.message : String(error);
  return msg.includes("No tab with id");
}

const COOKIE_AUTO_CAPTURE_DEBOUNCE_MS = 1500;
const COOKIE_AUTO_CAPTURE_MIN_INTERVAL_MS = 60_000;
const cookieDebounceTimers = new Map();
const cookieLastSentAt = new Map();

const TRACKED_COOKIE_NAMES = new Set([
  "__Secure-3PAPISID",
  "__Secure-1PAPISID",
  "__Secure-3PSID",
  "__Secure-1PSID",
  "SAPISID",
  "SID",
  "HSID",
  "SSID",
  "APISID",
  "LOGIN_INFO",
  "VISITOR_INFO1_LIVE",
  "PREF",
  "sessionid",
  "ds_user_id",
  "ig_did",
  "auth_token",
  "ct0",
  "kp",
  "tt_webid",
  "twid",
  "loid",
  "edgebucket",
  "oauth_token",
  "sc_anonymous_id",
  "moe_uuid",
  "datadome",
]);

const TRACKED_DOMAIN_SUFFIXES = [
  ".youtube.com",
  ".google.com",
  ".instagram.com",
  ".tiktok.com",
  ".x.com",
  ".twitter.com",
  ".reddit.com",
  ".twitch.tv",
  ".vimeo.com",
  ".bilibili.com",
  ".pinterest.com",
  ".hotmart.com",
  ".udemy.com",
  ".bsky.app",
  ".bsky.social",
  ".telegram.org",
  ".soundcloud.com",
];

function platformForDomain(domain) {
  const d = (domain || "").toLowerCase();
  if (d.endsWith(".youtube.com") || d.endsWith(".google.com") || d === "youtube.com")
    return "youtube";
  if (d.endsWith(".instagram.com") || d.endsWith(".cdninstagram.com")) return "instagram";
  if (d.endsWith(".tiktok.com")) return "tiktok";
  if (d.endsWith(".x.com") || d.endsWith(".twitter.com")) return "twitter";
  if (d.endsWith(".reddit.com")) return "reddit";
  if (d.endsWith(".twitch.tv")) return "twitch";
  if (d.endsWith(".vimeo.com")) return "vimeo";
  if (d.endsWith(".bilibili.com")) return "bilibili";
  if (d.endsWith(".pinterest.com")) return "pinterest";
  if (d.endsWith(".hotmart.com")) return "hotmart";
  if (d.endsWith(".udemy.com")) return "udemy";
  if (d.endsWith(".bsky.app") || d.endsWith(".bsky.social")) return "bluesky";
  if (d.endsWith(".telegram.org")) return "telegram";
  if (d.endsWith(".soundcloud.com") || d === "soundcloud.com") return "soundcloud";
  return null;
}

function shouldTrackCookieDomain(domain) {
  if (!domain) return false;
  const d = domain.toLowerCase();
  for (const suffix of TRACKED_DOMAIN_SUFFIXES) {
    if (d === suffix.slice(1) || d.endsWith(suffix)) return true;
  }
  return false;
}

function debounceCookieCapture(platform) {
  if (cookieDebounceTimers.has(platform)) {
    clearTimeout(cookieDebounceTimers.get(platform));
  }
  const timer = setTimeout(() => {
    cookieDebounceTimers.delete(platform);
    void capturePlatformCookies(platform);
  }, COOKIE_AUTO_CAPTURE_DEBOUNCE_MS);
  cookieDebounceTimers.set(platform, timer);
}

async function capturePlatformCookies(platform, force = false) {
  const lastSent = cookieLastSentAt.get(platform) || 0;
  if (!force && Date.now() - lastSent < COOKIE_AUTO_CAPTURE_MIN_INTERVAL_MS) {
    console.debug("[OmniGet] cookie capture throttled", platform);
    return { ok: false, reason: "throttled" };
  }
  cookieLastSentAt.set(platform, Date.now());

  let cookies = null;
  try {
    cookies = await extractCookiesForPlatform(platform);
  } catch (e) {
    console.warn("[OmniGet] cookie extract failed", platform, e);
    return { ok: false, reason: "extract_failed", error: e.message };
  }
  if (!cookies || cookies.length === 0) {
    console.debug("[OmniGet] no cookies for platform", platform);
    return { ok: false, reason: "no_cookies" };
  }

  const response = await sendCookiesViaBridge(cookies);
  if (response.ok) {
    console.info("[OmniGet] cookies exported", platform, cookies.length, response);
    return { ok: true, count: cookies.length, response };
  }
  console.warn("[OmniGet] cookie export failed", platform, response.reason ?? response.message);
  return { ok: false, reason: response.reason ?? "bridge_failed", response };
}

async function scanOpenTabsForCookies() {
  if (!chrome.tabs?.query) return;
  try {
    const tabs = await chrome.tabs.query({});
    const seen = new Set();
    for (const tab of tabs) {
      if (!tab.url) continue;
      let host;
      try {
        host = new URL(tab.url).hostname;
      } catch {
        continue;
      }
      const platform = platformForDomain(host);
      if (!platform || seen.has(platform)) continue;
      seen.add(platform);
      void capturePlatformCookies(platform, true);
    }
    if (seen.size === 0) {
      console.info("[OmniGet] no tracked tabs open at extension load");
    }
  } catch (e) {
    console.warn("[OmniGet] scan tabs failed", e);
  }
}

scanOpenTabsForCookies();

if (chrome.cookies?.onChanged) {
  chrome.cookies.onChanged.addListener((change) => {
    const cookie = change.cookie;
    if (!cookie) return;
    if (!TRACKED_COOKIE_NAMES.has(cookie.name)) return;
    if (!shouldTrackCookieDomain(cookie.domain)) return;
    const platform = platformForDomain(cookie.domain);
    if (!platform) return;
    debounceCookieCapture(platform);
  });
}

if (chrome.tabs?.onUpdated) {
  chrome.tabs.onUpdated.addListener((_tabId, changeInfo, tab) => {
    if (changeInfo.status !== "complete") return;
    if (!tab?.url) return;
    let host;
    try {
      host = new URL(tab.url).hostname;
    } catch {
      return;
    }
    if (!shouldTrackCookieDomain(host)) return;
    const platform = platformForDomain(host);
    if (!platform) return;
    debounceCookieCapture(platform);
  });
}
