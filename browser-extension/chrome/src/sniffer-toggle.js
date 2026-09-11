const STORAGE_KEY = "omniget_sniffer_enabled";
const WILDCARD_ORIGINS = ["*://*/*"];

let enabled = true;

// Resolves once the stored flag has actually been read. The webRequest
// listeners are registered synchronously — that is what lets an incoming
// request wake a terminated service worker at all — so they can fire before
// this settles. They capture optimistically against the default, and the
// decision to *keep* what they captured is deferred until this promise
// answers. Without it, a user who turned detection off still gets whatever
// arrived during the boot window recorded and persisted.
let statePromise = null;

export function loadSnifferState() {
  if (!statePromise) {
    statePromise = (async () => {
      try {
        const result = await chrome.storage.local.get(STORAGE_KEY);
        enabled = result[STORAGE_KEY] !== false;
      } catch {
        // Storage unavailable: keep the default rather than fail the boot.
      }
      return enabled;
    })();
  }
  return statePromise;
}

export function isSnifferEnabled() {
  return enabled;
}

export async function hasWildcardHostPermission() {
  if (!chrome.permissions || typeof chrome.permissions.contains !== "function") {
    return true;
  }
  try {
    return await chrome.permissions.contains({ origins: WILDCARD_ORIGINS });
  } catch {
    return false;
  }
}

export async function requestWildcardHostPermission() {
  if (!chrome.permissions || typeof chrome.permissions.request !== "function") {
    return true;
  }
  try {
    return await chrome.permissions.request({ origins: WILDCARD_ORIGINS });
  } catch {
    return false;
  }
}

export async function setSnifferEnabled(value) {
  if (value) {
    const granted = await requestWildcardHostPermission();
    if (!granted) {
      enabled = false;
      statePromise = Promise.resolve(false);
      await chrome.storage.local.set({ [STORAGE_KEY]: false });
      return { ok: false, reason: "permission_denied" };
    }
  }
  enabled = value;
  statePromise = Promise.resolve(value);
  await chrome.storage.local.set({ [STORAGE_KEY]: value });
  return { ok: true };
}
