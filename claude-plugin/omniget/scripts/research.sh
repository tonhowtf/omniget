#!/usr/bin/env bash
# Describe a media URL without downloading it. Prints one JSON line:
# {title, platform, uploader, duration, url, caption?, has_captions, thumbnail?, upload_date?}
# `caption` is the post text / description and is omitted when the platform provides none.
# Usage: research.sh <url>
set -eu
set -o pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$here/resolve-tools.sh"

url="${1:?usage: research.sh <url>}"
ytdlp="$(og_tool_path yt-dlp)" || { echo "research: yt-dlp not found; run doctor.sh" >&2; exit 1; }
args=(-J --no-warnings --no-playlist)

err="$(mktemp "${TMPDIR:-/tmp}/omniget-research.XXXXXX")"
if ! json="$(og_run_ytdlp "$url" "$err" "${args[@]}")"; then
  cat "$err" >&2
  hint="$(og_explain_error "$(cat "$err" 2>/dev/null)")"; [ -n "$hint" ] && echo "-> $hint" >&2
  rm -f "$err"; exit 1
fi
rm -f "$err"

printf '%s' "$json" | python3 -c '
import json, sys
info = json.load(sys.stdin)
if info.get("_type") == "playlist":
    entries = info.get("entries") or []
    info = entries[0] if entries else info
caption = (info.get("description") or "").strip()
out = {
    "title": info.get("title"),
    "platform": info.get("extractor_key"),
    "uploader": info.get("uploader") or info.get("channel") or info.get("uploader_id"),
    "duration": info.get("duration"),
    "url": info.get("webpage_url") or sys.argv[1],
    "has_captions": bool(info.get("subtitles")) or bool(info.get("automatic_captions")),
    "thumbnail": info.get("thumbnail"),
    "upload_date": info.get("upload_date"),
}
if caption:
    out["caption"] = caption
print(json.dumps({k: v for k, v in out.items() if v not in (None, "")}, ensure_ascii=False))' "$url"
