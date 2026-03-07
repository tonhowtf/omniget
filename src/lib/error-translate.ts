/**
 * Maps backend yt-dlp error strings (from translate_ytdlp_error in Rust) to i18n keys.
 * The backend returns fixed English strings — we detect them and return the translated version.
 */

const BACKEND_ERROR_MAP: Record<string, string> = {
  "Video requires login. Use browser cookies or try another URL.":
    "errors.login_required",
  "Server returned error 429 (too many requests). Try again later.":
    "errors.rate_limited",
  "Access denied (403). The video may be private or region-restricted.":
    "errors.access_denied",
  "Video extraction failed. Update yt-dlp or try again.":
    "errors.extraction_failed",
  "Requested format is not available. The download will retry with a compatible format.":
    "errors.format_unavailable",
  "Video unavailable or removed.": "errors.video_unavailable",
  "This video is private.": "errors.video_private",
  "Video blocked due to copyright.": "errors.copyright_blocked",
  "Video restricted in your region.": "errors.geo_restricted",
  "Connection timed out. Check your internet and try again.":
    "errors.connection_timeout",
  "FFmpeg not found. Install FFmpeg to download this format.":
    "errors.ffmpeg_missing",
  "Unsupported URL. Check that the link is correct.": "errors.unsupported_url",
  "Failed to access the page. Check the link and your connection.":
    "errors.page_access_failed",
  "No video formats found for this link.": "errors.no_formats",
};

/**
 * Translate a backend error string to the user's locale.
 * Strips the "Failed to get formats: " prefix added by the Tauri command layer.
 * Falls back to the original message if not recognized.
 */
export function translateBackendError(
  msg: string,
  t: (key: string) => string
): string {
  if (!msg) return t("common.unknown_error");

  // Strip prefix added in downloads.rs ("Failed to get formats: ...")
  const stripped = msg.replace(/^Failed to get formats:\s*/, "").trim();

  const key = BACKEND_ERROR_MAP[stripped];
  if (key) return t(key);

  // Fallback: return the message as-is (may be an unrecognized yt-dlp error)
  return stripped || msg;
}
