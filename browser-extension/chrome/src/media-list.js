export const DIRECT_MEDIA_MIN_BYTES = 500 * 1024;

export function isManifestEntry(entry) {
  return entry?.mediaType === "hls" || entry?.mediaType === "dash";
}

export function isListableMedia(entry, minBytes = DIRECT_MEDIA_MIN_BYTES) {
  if (!entry) return false;
  if (isManifestEntry(entry)) return true;
  return Number(entry.contentLength) > minBytes;
}
