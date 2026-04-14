export const MIN_CONTENT_LENGTH = 50 * 1024;

export const DASH_MIN_CONTENT_LENGTH = 1024;

export function isHlsManifest(url, contentType) {
  const u = String(url || "").toLowerCase();
  const ct = String(contentType || "").toLowerCase();
  return u.includes(".m3u8") || ct.includes("mpegurl");
}

export function isDashManifest(url, contentType) {
  const u = String(url || "").toLowerCase();
  const ct = String(contentType || "").toLowerCase();
  return u.includes(".mpd") || ct.includes("dash");
}

export function shouldDropBySize(url, contentType, contentLength) {
  if (!contentLength || contentLength <= 0) return false;

  if (isHlsManifest(url, contentType)) {
    return false;
  }

  if (isDashManifest(url, contentType)) {
    return contentLength < DASH_MIN_CONTENT_LENGTH;
  }

  return contentLength < MIN_CONTENT_LENGTH;
}
