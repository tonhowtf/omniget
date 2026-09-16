const STORAGE_KEY = "omniget_open_app_on_download";

let openApp = false;

export async function loadOpenAppState() {
  const result = await chrome.storage.local.get(STORAGE_KEY);
  openApp = result[STORAGE_KEY] === true;
  return openApp;
}

export function isOpenAppEnabled() {
  return openApp;
}

export async function setOpenAppEnabled(value) {
  openApp = value;
  await chrome.storage.local.set({ [STORAGE_KEY]: value });
}
