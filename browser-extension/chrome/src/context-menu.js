const MENU_ID = "omniget-download";
const PAGE_MENU_ID = "omniget-download-page";
const SELECTION_MENU_ID = "omniget-download-selection";

const MAX_SELECTION_LINKS = 20;

function title(key, fallback) {
  try {
    return chrome.i18n?.getMessage(key) || fallback;
  } catch {
    return fallback;
  }
}

export function registerContextMenu() {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: MENU_ID,
      title: title("context_menu_download", "Download with OmniGet"),
      contexts: ["link", "video", "audio", "image"],
    });
    chrome.contextMenus.create({
      id: PAGE_MENU_ID,
      title: title("context_menu_download_page", "Download this page with OmniGet"),
      contexts: ["page"],
      documentUrlPatterns: ["http://*/*", "https://*/*"],
    });
    chrome.contextMenus.create({
      id: SELECTION_MENU_ID,
      title: title("context_menu_download_selection", "Download the links in the selection with OmniGet"),
      contexts: ["selection"],
    });
  });
}

export function isContextMenuId(id) {
  return id === MENU_ID || id === PAGE_MENU_ID || id === SELECTION_MENU_ID;
}

function httpUrl(raw) {
  try {
    const url = new URL(raw);
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
  } catch {
    return null;
  }
}

export function linksInText(text, max = MAX_SELECTION_LINKS) {
  if (typeof text !== "string" || !text) return [];
  const out = [];
  for (const match of text.matchAll(/\b(?:https?:\/\/|www\.)[^\s<>"'`]+/gi)) {
    let raw = match[0].replace(/[.,;:!?]+$/, "");
    while (/[)\]}]$/.test(raw)) {
      const close = raw.at(-1);
      const open = { ")": "(", "]": "[", "}": "{" }[close];
      if (raw.split(open).length >= raw.split(close).length) break;
      raw = raw.slice(0, -1);
    }
    const url = httpUrl(/^www\./i.test(raw) ? `https://${raw}` : raw);
    if (url && !out.includes(url)) out.push(url);
    if (out.length >= max) break;
  }
  return out;
}

export function urlsForMenuClick(info) {
  if (!info) return [];
  if (info.menuItemId === MENU_ID) {
    const url = httpUrl(info.linkUrl || info.srcUrl || "");
    return url ? [url] : [];
  }
  if (info.menuItemId === PAGE_MENU_ID) {
    const url = httpUrl(info.pageUrl || "");
    return url ? [url] : [];
  }
  if (info.menuItemId === SELECTION_MENU_ID) {
    return linksInText(info.selectionText);
  }
  return [];
}
