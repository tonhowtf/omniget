import {
  DEFAULT_BLOCKED_HOSTS,
  USER_BLOCKED_HOSTS_KEY,
  mergeBlocklists,
  isUrlHostBlocked,
} from "./blocked-hosts.js";
import { hasWildcardHostPermission, requestWildcardHostPermission } from "./sniffer-toggle.js";

export const DEEP_SEARCH_STORAGE_KEY = "omniget_deep_search_enabled";

const MAIN_SCRIPT_ID = "omniget-deep-search-main";
const BRIDGE_SCRIPT_ID = "omniget-deep-search-bridge";
const PROBE_SCRIPT_ID = "omniget-deep-search-probe";
const MAIN_SCRIPT_FILE = "src/deep-search.js";
const BRIDGE_SCRIPT_FILE = "src/deep-search-bridge.js";
const MATCHES = ["*://*/*"];

export const DEEP_SEARCH_PROBE_MATCHES = Object.freeze([
  "http://127.0.0.1/__omniget_deep_search_probe__/*",
]);

const MAX_EXCLUDE_MATCHES = 100;

export const DEEP_SEARCH_EXTRA_BLOCKED_HOSTS = Object.freeze([
  "accounts.google.com",
  "mail.google.com",
  "docs.google.com",
  "drive.google.com",
  "meet.google.com",
  "calendar.google.com",
  "web.whatsapp.com",
  "teams.microsoft.com",
  "outlook.office.com",
  "outlook.live.com",
  "login.microsoftonline.com",
  "figma.com",
  "notion.so",
  "slack.com",
  "discord.com",
  "zoom.us",
  "atlassian.net",
  "github.dev",
]);

let enabled = false;
let blocklist = mergeBlocklists(DEFAULT_BLOCKED_HOSTS, DEEP_SEARCH_EXTRA_BLOCKED_HOSTS);
let mainWorldSupported = null;

export function getBlockedDeepSearchHosts() {
  return blocklist.slice();
}

export function shouldSkipDeepSearch(url) {
  return isUrlHostBlocked(url, blocklist);
}

export function getDeepSearchExcludeMatches() {
  const out = [];
  for (const host of blocklist) {
    if (out.length >= MAX_EXCLUDE_MATCHES) break;
    if (!/^[a-z0-9.-]+\.[a-z]{2,}$/.test(host)) continue;
    out.push(`*://*.${host}/*`);
  }
  return out;
}

async function loadUserBlocklist() {
  let user = [];
  try {
    const data = await chrome.storage.local.get(USER_BLOCKED_HOSTS_KEY);
    const stored = data?.[USER_BLOCKED_HOSTS_KEY];
    if (Array.isArray(stored)) user = stored;
  } catch {
    user = [];
  }
  blocklist = mergeBlocklists(DEFAULT_BLOCKED_HOSTS, [...DEEP_SEARCH_EXTRA_BLOCKED_HOSTS, ...user]);
  return blocklist;
}

export async function refreshDeepSearchBlocklist() {
  await loadUserBlocklist();
  if (enabled) await registerDeepSearchScripts();
  return getBlockedDeepSearchHosts();
}

export async function isDeepSearchSupported() {
  if (mainWorldSupported !== null) return mainWorldSupported;
  if (
    typeof chrome === "undefined" ||
    !chrome.scripting ||
    typeof chrome.scripting.registerContentScripts !== "function" ||
    typeof chrome.scripting.unregisterContentScripts !== "function"
  ) {
    mainWorldSupported = false;
    return false;
  }
  try {
    await chrome.scripting.unregisterContentScripts({ ids: [PROBE_SCRIPT_ID] });
  } catch {
  }
  try {
    await chrome.scripting.registerContentScripts([
      {
        id: PROBE_SCRIPT_ID,
        js: [MAIN_SCRIPT_FILE],
        matches: DEEP_SEARCH_PROBE_MATCHES.slice(),
        runAt: "document_start",
        world: "MAIN",
        persistAcrossSessions: false,
      },
    ]);
    mainWorldSupported = true;
  } catch {
    mainWorldSupported = false;
  }
  try {
    await chrome.scripting.unregisterContentScripts({ ids: [PROBE_SCRIPT_ID] });
  } catch {
  }
  return mainWorldSupported;
}

export async function registerDeepSearchScripts() {
  if (typeof chrome === "undefined" || !chrome.scripting?.registerContentScripts) return false;
  if (!(await isDeepSearchSupported())) return false;
  const granted = await hasWildcardHostPermission();
  if (!granted) return false;
  await unregisterDeepSearchScripts();
  const excludeMatches = getDeepSearchExcludeMatches();
  try {
    await chrome.scripting.registerContentScripts([
      {
        id: BRIDGE_SCRIPT_ID,
        js: [BRIDGE_SCRIPT_FILE],
        matches: MATCHES,
        excludeMatches,
        runAt: "document_start",
        allFrames: true,
        persistAcrossSessions: true,
      },
      {
        id: MAIN_SCRIPT_ID,
        js: [MAIN_SCRIPT_FILE],
        matches: MATCHES,
        excludeMatches,
        runAt: "document_start",
        allFrames: true,
        world: "MAIN",
        persistAcrossSessions: true,
      },
    ]);
    return true;
  } catch {
    return false;
  }
}

export async function unregisterDeepSearchScripts() {
  if (typeof chrome === "undefined" || !chrome.scripting?.unregisterContentScripts) return false;
  try {
    await chrome.scripting.unregisterContentScripts({ ids: [BRIDGE_SCRIPT_ID, MAIN_SCRIPT_ID] });
    return true;
  } catch {
    return true;
  }
}

async function store(value) {
  try {
    await chrome.storage.local.set({ [DEEP_SEARCH_STORAGE_KEY]: value });
  } catch {
  }
}

export async function loadDeepSearchState() {
  try {
    const result = await chrome.storage.local.get(DEEP_SEARCH_STORAGE_KEY);
    enabled = result?.[DEEP_SEARCH_STORAGE_KEY] === true;
  } catch {
    enabled = false;
  }
  await loadUserBlocklist();
  if (enabled) {
    const ok = await registerDeepSearchScripts();
    if (!ok) {
      enabled = false;
      await store(false);
    }
  } else {
    await unregisterDeepSearchScripts();
  }
  return enabled;
}

export function isDeepSearchEnabled() {
  return enabled;
}

export async function setDeepSearchEnabled(value) {
  if (value) {
    if (!(await isDeepSearchSupported())) {
      enabled = false;
      await store(false);
      return { ok: false, reason: "unsupported" };
    }
    const granted = await requestWildcardHostPermission();
    if (!granted) {
      enabled = false;
      await store(false);
      return { ok: false, reason: "permission_denied" };
    }
    await loadUserBlocklist();
    const registered = await registerDeepSearchScripts();
    if (!registered) {
      enabled = false;
      await store(false);
      return { ok: false, reason: "register_failed" };
    }
    enabled = true;
    await store(true);
    return { ok: true };
  }
  enabled = false;
  await store(false);
  await unregisterDeepSearchScripts();
  return { ok: true };
}
