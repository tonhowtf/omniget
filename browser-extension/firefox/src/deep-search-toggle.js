// Deep search is opt-in: it injects hooks into the MAIN world of every page,
// which is a much bigger ask than the passive network sniffer, so it defaults
// to off and only turns on from an explicit user gesture.
//
// Two separate content scripts are registered on purpose. The MAIN-world one
// can see the page's fetch/XHR/Worker but has no access to `chrome.runtime`;
// the isolated-world bridge has `chrome.runtime` but cannot see the page's
// globals. Putting both files in a single MAIN-world registration would leave
// the bridge unable to talk to the background, and nothing would ever arrive.
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

// The probe registration exists only to find out whether `world: "MAIN"` is
// accepted at all, so its pattern has two jobs: never match a page the user
// could be looking at, and stay inside a host permission the extension already
// holds unconditionally.
//
// That second part is what makes this the loopback address and not a reserved
// `.invalid` name. `registerContentScripts` refuses a pattern the extension has
// no host permission for, and the wildcard is optional — only granted once the
// user turns something on. Probing an origin we do not hold yet would fail on a
// fresh install for lack of permission, be read as "MAIN world unsupported",
// and hide the toggle that asks for the permission in the first place. The
// bridge's `http://127.0.0.1/*` is a fixed permission in both manifests, and no
// real page lives under this path.
export const DEEP_SEARCH_PROBE_MATCHES = Object.freeze([
  "http://127.0.0.1/__omniget_deep_search_probe__/*",
]);

const MAX_EXCLUDE_MATCHES = 100;

// Pages where hooking JSON.parse and fetch buys nothing and costs a lot: heavy
// single-page apps, and places where reading response bodies is simply not our
// business. These sit on top of the shared analytics/tracker blocklist.
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

// Only entries that look like real hostnames can become match patterns; the
// shared blocklist also carries bare substrings such as "analytics".
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

// Capability detection instead of a browser sniff. `world: "MAIN"` needs Chrome
// 111+ and Firefox 128+, and this project still supports Firefox 109, so the
// feature has to hide itself where it cannot work rather than raise the
// minimum. Registering a throwaway MAIN-world script against a pattern that
// matches nothing is the cheapest honest probe: browsers that do not know the
// `world` key reject the call, no page is ever touched, and the answer is
// cached for the life of the service worker.
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
    // No leftover probe from a previous run; that is the normal case.
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
    // ignore
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
      // The bridge is listed first so its listener is in place before the
      // MAIN-world hooks have anything to report.
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
    // Unregistering ids that were never registered throws; that is still the
    // state we wanted.
    return true;
  }
}

async function store(value) {
  try {
    await chrome.storage.local.set({ [DEEP_SEARCH_STORAGE_KEY]: value });
  } catch {
    // ignore
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

// Must be reachable from a user gesture: `chrome.permissions.request` only
// resolves to true when it can show its prompt, which rules out calling this
// from an alarm or from service-worker startup.
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
