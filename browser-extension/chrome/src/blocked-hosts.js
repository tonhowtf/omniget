export const DEFAULT_BLOCKED_HOSTS = Object.freeze([
  "google-analytics.com",
  "googletagmanager.com",
  "facebook.com",
  "doubleclick.net",
  "analytics",
  "ads.google.com",
  "ad.doubleclick.net",
  "adservice.google.com",
  "pagead2.googlesyndication.com",
  "cdn.mxpnl.com",
  "stats.wp.com",
  "pixel.facebook.com",
  "connect.facebook.net",
  "scorecardresearch.com",
  "hotjar.com",
  "sentry.io",
  "newrelic.com",
  "nr-data.net",
  "segment.io",
  "segment.com",
  "amplitude.com",
  "mixpanel.com",
  "cdn.heapanalytics.com",
]);

export const USER_BLOCKED_HOSTS_KEY = "userBlockedHosts";

export function normalizeBlocklist(entries) {
  if (!Array.isArray(entries)) return [];
  const out = [];
  const seen = new Set();
  for (const raw of entries) {
    if (typeof raw !== "string") continue;
    const trimmed = raw.trim().toLowerCase().replace(/^\*\.?/, "").replace(/^\./, "");
    if (!trimmed) continue;
    if (seen.has(trimmed)) continue;
    seen.add(trimmed);
    out.push(trimmed);
  }
  return out;
}

export function mergeBlocklists(defaults, user) {
  const merged = new Set(normalizeBlocklist(defaults));
  for (const host of normalizeBlocklist(user)) {
    merged.add(host);
  }
  return Array.from(merged);
}

export function isHostBlocked(host, blocklist) {
  if (!host || typeof host !== "string") return false;
  const lowerHost = host.toLowerCase();
  for (const entry of blocklist) {
    if (!entry) continue;
    if (lowerHost.includes(entry)) return true;
  }
  return false;
}

export function isUrlHostBlocked(url, blocklist) {
  try {
    const host = new URL(url).hostname;
    return isHostBlocked(host, blocklist);
  } catch {
    return false;
  }
}
