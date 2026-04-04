const MENU_ID = "omniget-download";

export function registerContextMenu() {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: MENU_ID,
      title: "Download with OmniGet",
      contexts: ["link", "video", "audio", "image"],
    });
  });
}

export function getContextMenuId() {
  return MENU_ID;
}
