const STORAGE_KEY = "omniget_sniffer_enabled";

let enabled = true;

export async function loadSnifferState() {
  const result = await chrome.storage.local.get(STORAGE_KEY);
  enabled = result[STORAGE_KEY] !== false;
  return enabled;
}

export function isSnifferEnabled() {
  return enabled;
}

export async function setSnifferEnabled(value) {
  enabled = value;
  await chrome.storage.local.set({ [STORAGE_KEY]: value });
}
