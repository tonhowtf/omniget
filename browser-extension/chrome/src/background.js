import { extractCookiesForPlatform } from "./cookies.js";
import { detectSupportedMediaUrl } from "./detect.js";
import { createActionFeedbackController } from "./action-feedback.js";
import { registerSnifferListeners, getMediaCount, getMediaCountForPage, getDetectedMedia, getDetectedMediaForUrl, getPageKeyForTab, restoreMedia } from "./media-sniffer.js";
import { summarizeCookies } from "./cookie-summary.js";
import { loadSnifferState, isSnifferEnabled, setSnifferEnabled } from "./sniffer-toggle.js";
import { registerContextMenu, getContextMenuId } from "./context-menu.js";
import { openOmnigetScheme } from "./send-via-scheme.js";

const HOST_NAME = "wtf.tonho.omniget";
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

chrome.runtime.onInstalled.addListener(() => {
  registerContextMenu();
  refreshActiveTab().catch(() => {});
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

  try {
    const response = await sendNativeMessage(message);
    return { ok: response?.ok ?? false, cookieSummary };
  } catch (e) {
    const schemeResult = await openOmnigetScheme(url);
    if (schemeResult?.ok) {
      return { ok: true, viaScheme: true, cookieSummary };
    }
    return { ok: false, error: e.message };
  }
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

function sendNativeMessage(message) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendNativeMessage(HOST_NAME, message, (response) => {
      if (chrome.runtime.lastError) {
        reject(new Error(chrome.runtime.lastError.message));
        return;
      }
      resolve(response);
    });
  });
}
