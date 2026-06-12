/**
 * Maps backend yt-dlp error strings (from translate_ytdlp_error in Rust) to i18n keys.
 * The backend returns fixed English strings — we detect them and return the translated version.
 */

const BACKEND_ERROR_MAP: Record<string, string> = {
  "Video requires login. Use browser cookies or try another URL.":
    "errors.login_required",
  "This video requires login. Import cookies for this site in Settings → Cookies, then retry.":
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
  "TikTok is blocking requests. Try again in a few minutes.":
    "errors.tiktok_blocked",
  "Download timeout — no data received for 30 seconds":
    "errors.download_timeout",
  "Write error (disk full?)": "errors.disk_full",
  "Console encoding error (non-UTF-8 locale). Update yt-dlp in Settings → Dependencies, or run `chcp 65001` in a terminal and reopen the app.":
    "errors.console_encoding",
  "yt-dlp extractor is broken for this site. Update yt-dlp in Settings → Dependencies, then retry.":
    "errors.extractor_broken",
  "Download reported success but the file is missing or empty. Check disk space and antivirus exclusions, then retry.":
    "errors.output_missing",
};

/**
 * Translate a backend error string to the user's locale.
 * Strips the "Failed to get formats: " prefix added by the Tauri command layer.
 * Falls back to the original message if not recognized.
 */
export function translateBackendError(
  msg: string,
  t: (key: string) => string,
  tWithValues?: (key: string, opts: { values: Record<string, string | number> }) => string
): string {
  if (!msg) return t("common.unknown_error");

  if (msg.startsWith("PathTooLong|")) {
    const parts = msg.split("|");
    const limit = Number(parts[1] ?? 0);
    const current = Number(parts[2] ?? 0);
    if (tWithValues) {
      return tWithValues("errors.path_too_long", {
        values: { limit, current },
      });
    }
    return t("errors.path_too_long");
  }

  const stripped = msg.replace(/^Failed to get formats:\s*/, "").trim();

  const key = BACKEND_ERROR_MAP[stripped];
  if (key) return t(key);

  const lower = stripped.toLowerCase();
  if (lower.includes("could not copy") && lower.includes("cookie")) return t("errors.cookie_database");
  if (lower.includes("size mismatch")) return t("errors.size_mismatch");
  if (lower.includes("disk full") || lower.includes("write error")) return t("errors.disk_full");
  if (lower.includes("tiktok") && lower.includes("blocking")) return t("errors.tiktok_blocked");
  if (lower.includes("download reported success but no matching file"))
    return t("errors.console_encoding");

  return stripped || msg;
}
