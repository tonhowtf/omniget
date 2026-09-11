// What the popup is willing to put in its list. A plain file has to be big
// enough to be worth offering (the sniffer already lets small responses
// through so nothing is lost upstream), but a manifest is judged differently:
// an m3u8 or an mpd is a few KB of text pointing at hundreds of MB, and one
// found by deep search arrives with no size at all. Filtering those by byte
// count is how a playlist gets captured and then silently never shown.
export const DIRECT_MEDIA_MIN_BYTES = 500 * 1024;

export function isManifestEntry(entry) {
  return entry?.mediaType === "hls" || entry?.mediaType === "dash";
}

export function isListableMedia(entry, minBytes = DIRECT_MEDIA_MIN_BYTES) {
  if (!entry) return false;
  if (isManifestEntry(entry)) return true;
  return Number(entry.contentLength) > minBytes;
}
