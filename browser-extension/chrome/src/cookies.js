export const DEFAULT_PLATFORM_COOKIE_DOMAINS = Object.freeze({
  youtube: [".youtube.com", ".google.com"],
  instagram: [".instagram.com", ".cdninstagram.com", ".fbcdn.net"],
  tiktok: [".tiktok.com", ".tiktokcdn.com"],
  twitter: [".twitter.com", ".x.com"],
  reddit: [".reddit.com"],
  twitch: [".twitch.tv", ".jtvnw.net"],
  vimeo: [".vimeo.com", ".vimeocdn.com"],
  bilibili: [".bilibili.com", ".bilivideo.com"],
  pinterest: [".pinterest.com"],
  hotmart: [".hotmart.com"],
  udemy: [".udemy.com"],
  bluesky: [".bsky.app", ".bsky.social"],
  telegram: [".telegram.org", ".t.me"],
});

export const COOKIE_DOMAINS_RESOURCE_PATH = "src/cookies-domains.json";

let cachedDomains = null;
let cacheSource = null;

export async function loadPlatformCookieDomains() {
  if (cachedDomains) return cachedDomains;
  const loaded = await fetchCookieDomainsFromResource();
  if (loaded) {
    cachedDomains = Object.freeze(loaded);
    cacheSource = "resource";
    return cachedDomains;
  }
  cachedDomains = DEFAULT_PLATFORM_COOKIE_DOMAINS;
  cacheSource = "default";
  return cachedDomains;
}

export function getPlatformCookieDomainsCacheSource() {
  return cacheSource;
}

export function resetPlatformCookieDomainsCacheForTesting() {
  cachedDomains = null;
  cacheSource = null;
}

async function fetchCookieDomainsFromResource() {
  if (
    typeof chrome === "undefined" ||
    !chrome.runtime ||
    typeof chrome.runtime.getURL !== "function" ||
    typeof fetch !== "function"
  ) {
    return null;
  }
  try {
    const url = chrome.runtime.getURL(COOKIE_DOMAINS_RESOURCE_PATH);
    const response = await fetch(url);
    if (!response.ok) return null;
    const parsed = await response.json();
    return isValidDomainsMap(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function isValidDomainsMap(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  for (const key of Object.keys(value)) {
    const entry = value[key];
    if (!Array.isArray(entry)) return false;
    if (!entry.every((d) => typeof d === "string" && d.length > 0)) return false;
  }
  return true;
}

export async function extractCookiesForPlatform(
  platform,
  getAllCookies = defaultGetAllCookies,
  domainsOverride = null,
) {
  const byPlatform = domainsOverride || (await loadPlatformCookieDomains());
  const domains = byPlatform[platform];
  if (!domains) return null;

  const allCookies = [];
  for (const domain of domains) {
    const cookies = await getAllCookies({ domain });
    allCookies.push(...cookies);
  }

  if (allCookies.length === 0) return null;

  return allCookies.map((c) => ({
    domain: c.domain,
    httpOnly: c.httpOnly,
    path: c.path,
    secure: c.secure,
    expires: c.expirationDate ? Math.floor(c.expirationDate) : 0,
    name: c.name,
    value: c.value,
    hostOnly: c.hostOnly,
    sameSite: c.sameSite,
  }));
}

function defaultGetAllCookies(details) {
  if (typeof chrome === "undefined" || !chrome.cookies?.getAll) {
    return Promise.resolve([]);
  }
  return chrome.cookies.getAll(details);
}
