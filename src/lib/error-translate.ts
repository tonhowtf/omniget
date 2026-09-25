/**
 * Maps backend yt-dlp error strings (from translate_ytdlp_error in Rust) to i18n keys.
 * The backend returns fixed English strings — we detect them and return the translated version.
 */

const BACKEND_ERROR_MAP: Record<string, string> = {
  "rune pages are full": "league.runes_full",
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
  "Downloaded streams are DRM-protected and cannot be merged. This content is not supported.":
    "errors.drm_protected",
  "The owner of this site asked OmniGet not to support it.":
    "errors.platform_opted_out",
  "OmniDisc: invalid instance URL. Use http:// or https:// without a username or password.":
    "omnidisc.error.invalid_url",
  "OmniDisc: the server did not respond. Check the address or ask the owner for a new link.":
    "omnidisc.error.unreachable",
  "OmniDisc: this address is not an OmniDisc server.": "omnidisc.error.not_an_instance",
  ERR_UNAUTHORIZED: "omnidisc.error.unauthorized",
  ERR_NO_SESSION: "omnidisc.error.no_session",
  ERR_FORBIDDEN: "omnidisc.error.forbidden",
  "ERR_FORBIDDEN:registration_closed": "omnidisc.error.registration_closed",
  "ERR_FORBIDDEN:missing_permissions": "omnidisc.error.missing_permissions",
  "ERR_FORBIDDEN:too_many_guilds": "omnidisc.error.too_many_guilds",
  ERR_NOT_FOUND: "omnidisc.error.not_found",
  ERR_RATE_LIMITED: "omnidisc.error.rate_limited",
  ERR_UNREACHABLE: "omnidisc.error.unreachable",
  ERR_SERVER: "omnidisc.error.server",
  ERR_NOT_CONNECTED: "omnidisc.error.not_connected",
  ERR_PROTOCOL: "omnidisc.error.protocol",
  ERR_BAD_REQUEST: "omnidisc.error.bad_request",
  "ERR_BAD_REQUEST:invalid_credentials": "omnidisc.error.invalid_credentials",
  "ERR_BAD_REQUEST:username_taken": "omnidisc.error.username_taken",
  "ERR_BAD_REQUEST:invalid_username": "omnidisc.error.invalid_username",
  "ERR_BAD_REQUEST:invalid_password": "omnidisc.error.invalid_password",
  "ERR_BAD_REQUEST:invalid_invite": "omnidisc.error.invalid_invite",
  "ERR_BAD_REQUEST:message_too_long": "omnidisc.error.message_too_long",
  "ERR_BAD_REQUEST:empty_message": "omnidisc.error.empty_message",
  ERR_SCREEN_PERMISSION: "omnidisc.error.screen_permission",
  ERR_STREAM_UNSUPPORTED: "omnidisc.error.stream_unsupported",
  ERR_STREAM_SOURCE_GONE: "omnidisc.error.stream_source_gone",
  ERR_STREAM_CAPTURE_FAILED: "omnidisc.error.stream_capture_failed",
  ERR_STREAM_ENCODER_FAILED: "omnidisc.error.stream_encoder_failed",
  ERR_STREAM_NOT_STREAMING: "omnidisc.error.stream_not_streaming",
  ERR_STREAM_NOT_FOUND: "omnidisc.error.stream_not_found",
  ERR_STREAM_VIEWER_FAILED: "omnidisc.error.stream_viewer_failed",
  ERR_VOICE_MIC_PERMISSION: "omnidisc.error.voice_mic_permission",
  ERR_VOICE_NO_INPUT_DEVICE: "omnidisc.error.voice_no_input_device",
  ERR_VOICE_NO_OUTPUT_DEVICE: "omnidisc.error.voice_no_output_device",
  ERR_VOICE_NO_AUDIO_DEVICE: "omnidisc.error.voice_no_input_device",
  ERR_VOICE_DEVICE_BUSY: "omnidisc.error.voice_device_busy",
  ERR_VOICE_MIC_FAILED: "omnidisc.error.voice_mic_failed",
  ERR_VOICE_OUTPUT_FAILED: "omnidisc.error.voice_output_failed",
  ERR_VOICE_MIC_LOST: "omnidisc.error.voice_mic_lost",
  ERR_VOICE_OUTPUT_LOST: "omnidisc.error.voice_output_lost",
  ERR_VOICE_OUTPUT_PERMISSION: "omnidisc.error.voice_output_permission",
  ERR_VOICE_UNREACHABLE: "omnidisc.error.voice_unreachable",
  ERR_VOICE_TIMEOUT: "omnidisc.error.voice_timeout",
  ERR_VOICE_DENIED: "omnidisc.error.voice_denied",
  ERR_VOICE_DM_UNSUPPORTED: "omnidisc.error.voice_dm_unsupported",
  ERR_VOICE_NOT_CONNECTED: "omnidisc.error.voice_not_connected",
  ERR_VOICE_DISCONNECTED: "omnidisc.error.voice_disconnected",
  ERR_VOICE_UNAVAILABLE: "omnidisc.error.voice_unavailable",
  ERR_VOICE_ENGINE_CRASHED: "omnidisc.error.voice_engine_crashed",
  ERR_E2EE: "omnidisc.error.e2ee",
  ERR_E2EE_NOT_READY: "omnidisc.error.e2ee_not_ready",
  ERR_UPLOAD_TOO_LARGE: "omnidisc.error.upload_too_large",
  ERR_UPLOAD: "omnidisc.error.upload_failed",
  ERR_UPLOAD_CANCELLED: "omnidisc.error.upload_cancelled",
  ERR_UNKNOWN_UPLOAD: "omnidisc.error.unknown_upload",
  ERR_ATTACHMENT_CORRUPT: "omnidisc.error.attachment_corrupt",
  ERR_ATTACHMENT_ORIGIN: "omnidisc.error.attachment_origin",
  ERR_ATTACHMENT_TOO_LARGE: "omnidisc.error.attachment_too_large",
  ERR_E2EE_UNTRUSTED: "omnidisc.error.e2ee_untrusted",
  ERR_TOO_MANY_ATTACHMENTS: "omnidisc.error.too_many_attachments",
  "ERR_BAD_REQUEST:file_missing": "omnidisc.error.file_missing",
  "ERR_BAD_REQUEST:epoch_conflict": "omnidisc.error.epoch_conflict",
  "ERR_FORBIDDEN:device_required": "omnidisc.error.device_required",
};

const OMNIDISC_PREFIX_MAP: Record<string, string> = {
  ERR_FORBIDDEN: "omnidisc.error.forbidden",
  ERR_BAD_REQUEST: "omnidisc.error.bad_request",
  ERR_UPLOAD: "omnidisc.error.upload_failed",
  ERR_UNKNOWN_UPLOAD: "omnidisc.error.unknown_upload",
  ERR_UPLOAD_TOO_LARGE: "omnidisc.error.upload_too_large",
  ERR_TOO_MANY_ATTACHMENTS: "omnidisc.error.too_many_attachments",
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

  if (stripped.startsWith("ERR_")) {
    const prefix = stripped.split(":")[0];
    const prefixKey = OMNIDISC_PREFIX_MAP[prefix];
    if (prefixKey) return t(prefixKey);
  }

  const lower = stripped.toLowerCase();
  if (lower.includes("could not copy") && lower.includes("cookie")) return t("errors.cookie_database");
  if (lower.includes("size mismatch")) return t("errors.size_mismatch");
  if (lower.includes("disk full") || lower.includes("write error")) return t("errors.disk_full");
  if (lower.includes("tiktok") && lower.includes("blocking")) return t("errors.tiktok_blocked");
  if (lower.includes("download reported success but no matching file"))
    return t("errors.console_encoding");
  if (lower.includes("invalid data found when processing input") || lower.includes("drm-protected"))
    return t("errors.drm_protected");

  return stripped || msg;
}
