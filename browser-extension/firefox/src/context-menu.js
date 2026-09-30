const MENU_ID = "omniget-download";
const PAGE_MENU_ID = "omniget-download-page";
const SELECTION_MENU_ID = "omniget-download-selection";

// A selection can hold a whole list of links; past this many it is more likely
// a page of text than a list someone meant to download.
const MAX_SELECTION_LINKS = 20;

function title(key, fallback) {
  try {
    return chrome.i18n?.getMessage(key) || fallback;
  } catch {
    return fallback;
  }
}

// Three entries, like NeoDLP's extension: a link or media element, the page
// itself (the video you are watching), and the links in selected text.
// Chrome shows the page entry only where no link, media or selection is under
// the pointer, so a right-click never offers two OmniGet entries at once.
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

/**
 * Every http(s) link written in `text`, in order and without repeats.
 * A link that starts with `www.` gets `https://`. Punctuation that ends a
 * sentence or closes a bracket around the link is not part of it.
 */
export function linksInText(text, max = MAX_SELECTION_LINKS) {
  if (typeof text !== "string" || !text) return [];
  const out = [];
  for (const match of text.matchAll(/\b(?:https?:\/\/|www\.)[^\s<>"'`]+/gi)) {
    let raw = match[0].replace(/[.,;:!?]+$/, "");
    // Drop a closing bracket only when the link did not open one.
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

/** The URLs a click on one of the entries should send to the app. */
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
