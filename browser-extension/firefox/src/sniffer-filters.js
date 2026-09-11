export const MEDIA_CONTENT_TYPES = Object.freeze([
  "video/mp4",
  "video/webm",
  "video/x-flv",
  "video/ogg",
  "video/x-matroska",
  "video/3gpp",
  "video/mpeg",
  "video/x-msvideo",
  "video/x-ms-wmv",
  "video/quicktime",
  "audio/mpeg",
  "audio/ogg",
  "audio/mp4",
  "audio/webm",
  "audio/aac",
  "audio/wav",
  "audio/x-wav",
  "audio/flac",
  "audio/x-flac",
  "audio/x-m4a",
  "audio/x-ms-wma",
  "audio/opus",
  "application/vnd.apple.mpegurl",
  "application/x-mpegurl",
  "application/dash+xml",
  "application/f4m+xml",
]);

export const MEDIA_EXTENSIONS = Object.freeze([
  ".mp4", ".webm", ".m3u8", ".mpd",
  ".flv", ".ogg", ".mp3", ".m4a", ".m4v",
  ".mkv", ".avi", ".mov", ".wmv",
  ".wav", ".flac", ".aac", ".opus",
  ".3gp", ".mpg", ".mpeg", ".divx",
  ".f4m", ".f4v", ".ts",
]);

// An extension only counts when it sits at a path boundary: either the path
// ends with it, or it is a whole segment (`/file.mp4/range/0-100`, which some
// CDNs use). A bare `includes` would call `/api/v2/list.mp4.json` a video.
// The path is expected to be a lowercased `URL.pathname`, so no query string.
export function extensionMatchesPath(path, ext) {
  if (typeof path !== "string" || typeof ext !== "string" || ext === "") return false;
  return path.endsWith(ext) || path.includes(`${ext}/`);
}

export function pathnameOf(url) {
  try {
    return new URL(url).pathname.toLowerCase();
  } catch {
    return null;
  }
}

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
