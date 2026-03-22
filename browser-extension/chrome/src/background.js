import { extractCookiesForPlatform } from "./cookies.js";
import { detectSupportedMediaUrl } from "./detect.js";
import { handleSupportedActionClick } from "./action-click.js";
import { createActionFeedbackController } from "./action-feedback.js";
import { resolveActionTitle } from "./action-title.js";

const HOST_NAME = "wtf.tonho.omniget";
const INSTALL_URL = "https://github.com/tonhowtf/omniget/releases/latest";

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

chrome.runtime.onInstalled.addListener(() => {
  refreshActiveTab().catch(() => {});
});

chrome.runtime.onStartup.addListener(() => {
  refreshActiveTab().catch(() => {});
});

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

chrome.action.onClicked.addListener(async (tab) => {
  const detected = detectSupportedMediaUrl(tab?.url);
  if (!detected?.supported) {
    return;
  }

  await handleSupportedActionClick({
    tabId: tab?.id,
    url: detected.url,
    platform: detected.platform,
    getCookies: extractCookiesForPlatform,
    sendNativeMessage,
    clearBadge: (tabId) => actionFeedback.clearBadge(tabId),
    showSuccessBadge: (tabId) => actionFeedback.showSuccessBadge(tabId),
    openErrorPage,
    mapChromeErrorCode,
  });
});

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
  try {
    await actionFeedback.clearBadge(tabId);
  } catch (error) {
    if (isTabGoneError(error)) return;
    console.error("[OmniGet] Failed to clear badge:", error);
  }

  if (!tab?.url) {
    return;
  }

  const detected = detectSupportedMediaUrl(tab.url);
  const supported = Boolean(detected?.supported);

  try {
    await chrome.action.setIcon({
      tabId,
      path: supported ? getIconPath(ACTIVE_ICON_PATHS) : getIconPath(INACTIVE_ICON_PATHS),
    });
  } catch (error) {
    if (isTabGoneError(error)) return;
    console.error("[OmniGet] Failed to set icon:", error);
  }

  try {
    await chrome.action.setTitle({
      tabId,
      title: resolveActionTitle(supported),
    });
  } catch (error) {
    if (isTabGoneError(error)) return;
    console.error("[OmniGet] Failed to set title:", error);
  }

  try {
    if (supported) {
      await chrome.action.enable(tabId);
    } else {
      await chrome.action.disable(tabId);
    }
  } catch (error) {
    if (isTabGoneError(error)) return;
    console.error("[OmniGet] Failed to set enabled state:", error);
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

function mapChromeErrorCode(error) {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes("Specified native messaging host not found")) {
    return "HOST_MISSING";
  }
  if (message.includes("Access to the specified native messaging host is forbidden")) {
    return "HOST_MISSING";
  }
  return "LAUNCH_FAILED";
}

function openErrorPage({ code, message, url }) {
  const params = new URLSearchParams({
    code,
    url,
  });

  if (message) {
    params.set("message", message);
  }

  params.set("installUrl", INSTALL_URL);

  return chrome.tabs.create({
    url: `${chrome.runtime.getURL("pages/error.html")}?${params.toString()}`,
  });
}
