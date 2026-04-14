export const PAGE_KEY_STORAGE_PREFIX = "media_page:";

export const DEFAULT_PAGE_TTL_MS = 24 * 60 * 60 * 1000;

export function normalizePageKey(url) {
  if (!url) return null;
  try {
    const u = new URL(url);
    if (u.protocol !== "http:" && u.protocol !== "https:") return null;
    const host = u.hostname.toLowerCase();
    const path = u.pathname || "/";
    return `${u.protocol}//${host}${path}`;
  } catch {
    return null;
  }
}

export function storageKeyForPage(pageKey) {
  return `${PAGE_KEY_STORAGE_PREFIX}${pageKey}`;
}

export function pageKeyFromStorageKey(storageKey) {
  if (typeof storageKey !== "string") return null;
  if (!storageKey.startsWith(PAGE_KEY_STORAGE_PREFIX)) return null;
  return storageKey.slice(PAGE_KEY_STORAGE_PREFIX.length);
}

export function isExpired(savedAt, now, ttlMs = DEFAULT_PAGE_TTL_MS) {
  if (typeof savedAt !== "number" || !Number.isFinite(savedAt)) return true;
  if (typeof now !== "number" || !Number.isFinite(now)) return true;
  if (savedAt > now) return false;
  return now - savedAt > ttlMs;
}

export function classifyStoredPages(allStorage, now, ttlMs = DEFAULT_PAGE_TTL_MS) {
  const valid = [];
  const stale = [];
  if (!allStorage || typeof allStorage !== "object") {
    return { valid, stale };
  }
  for (const [key, value] of Object.entries(allStorage)) {
    const pageKey = pageKeyFromStorageKey(key);
    if (!pageKey) continue;
    if (!value || typeof value !== "object" || !Array.isArray(value.media)) {
      stale.push(key);
      continue;
    }
    if (isExpired(value.savedAt, now, ttlMs)) {
      stale.push(key);
      continue;
    }
    valid.push({ storageKey: key, pageKey, media: value.media, savedAt: value.savedAt });
  }
  return { valid, stale };
}
