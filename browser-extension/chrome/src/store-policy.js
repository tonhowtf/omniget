// WHY: the Chrome Web Store rejects extensions that download from YouTube; scripts/package.mjs --store sets this to true in the packaged copy only.
export const STORE_BUILD = false;

const EXCLUDED_HOSTS = ["youtube.com", "youtube-nocookie.com", "youtu.be", "googlevideo.com", "ytimg.com"];
const EXCLUDED_PLATFORMS = new Set(["youtube", "youtube_music"]);

export function isStoreExcludedHost(host, store = STORE_BUILD) {
  if (!store || typeof host !== "string" || !host) return false;
  const h = host.toLowerCase().replace(/^\./, "");
  return EXCLUDED_HOSTS.some((entry) => h === entry || h.endsWith(`.${entry}`));
}

export function isStoreExcludedUrl(rawUrl, store = STORE_BUILD) {
  if (!store || typeof rawUrl !== "string") return false;
  try {
    return isStoreExcludedHost(new URL(rawUrl).hostname, store);
  } catch {
    return false;
  }
}

export function isStoreExcludedPlatform(platform, store = STORE_BUILD) {
  return Boolean(store) && EXCLUDED_PLATFORMS.has(platform);
}
