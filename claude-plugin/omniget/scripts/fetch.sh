#!/usr/bin/env bash
# Download media from a URL. Prints one JSON line: {file,title,duration,size,platform,engine,id}.
# Usage: fetch.sh <url> [--audio] [--quality 720] [--out DIR]
set -eu
set -o pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$here/resolve-tools.sh"

url=""; audio=false; quality=""; out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --audio) audio=true ;;
    --quality) quality="$2"; shift ;;
    --out) out="$2"; shift ;;
    -*) echo "fetch: unknown flag $1" >&2; exit 2 ;;
    *) url="$1" ;;
  esac
  shift
done
[ -n "$url" ] || { echo "usage: fetch.sh <url> [--audio] [--quality N] [--out DIR]" >&2; exit 2; }
[ -n "$out" ] || out="$(og_output_dir)"
mkdir -p "$out"

file_size() {
  if stat -f%z "$1" >/dev/null 2>&1; then stat -f%z "$1"; else stat -c%s "$1"; fi
}

# omniget-cli path: native extractors and OmniGet cookie accounts for the hosts that need them.
if og_prefers_cli "$url" && cli="$(og_tool_path omniget-cli)"; then
  info="$("$cli" --json info "$url")"
  args=(--json download "$url" -o "$out")
  $audio && args+=(--audio-only)
  [ -n "$quality" ] && args+=(-q "$quality")
  result="$("$cli" "${args[@]}" | tail -n 1)"
  printf '%s\n%s\n' "$info" "$result" | python3 -c '
import json, sys
info, result = [json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()][:2]
if not result.get("success"):
    sys.exit("fetch: omniget-cli failed: %s" % result.get("error"))
print(json.dumps({
    "file": result["file_path"], "title": info.get("title"), "duration": info.get("duration"),
    "size": result.get("size"), "platform": result.get("platform"), "engine": "omniget-cli",
}, ensure_ascii=False))'
  exit 0
fi

ytdlp="$(og_tool_path yt-dlp)" || { echo "fetch: yt-dlp not found; run doctor.sh" >&2; exit 1; }
ffmpeg_dir=""
if ffmpeg="$(og_tool_path ffmpeg)"; then ffmpeg_dir="$(dirname "$ffmpeg")"; fi

args=(--no-warnings --no-playlist --no-simulate --quiet
      --print "%(id)s ||| %(extractor_key)s ||| %(duration)s ||| %(title)s"
      --print "after_move:filepath"
      -o "$out/%(title).120B [%(id)s].%(ext)s")
[ -n "$ffmpeg_dir" ] && args+=(--ffmpeg-location "$ffmpeg_dir")
if $audio; then
  args+=(-f "bestaudio/best" -x --audio-format m4a)
elif [ -n "$quality" ]; then
  args+=(-f "bv*[height<=$quality]+ba/b[height<=$quality]/b" --merge-output-format mp4)
else
  args+=(-f "bv*+ba/b" --merge-output-format mp4)
fi
while IFS= read -r line; do args+=("$line"); done < <(og_cookie_args "$url")

err="$(mktemp "${TMPDIR:-/tmp}/omniget-fetch.XXXXXX")"
if ! output="$("$ytdlp" "${args[@]}" "$url" 2>"$err")"; then
  cat "$err" >&2
  hint="$(og_explain_error "$(cat "$err" 2>/dev/null)")"
  [ -n "$hint" ] && echo "-> $hint" >&2
  rm -f "$err"
  exit 1
fi
rm -f "$err"
meta="$(printf '%s\n' "$output" | head -n 1)"
path="$(printf '%s\n' "$output" | tail -n 1)"
[ -f "$path" ] || { echo "fetch: download finished but file not found: $path" >&2; exit 1; }

id="$(printf '%s' "$meta" | awk -F' \\|\\|\\| ' '{print $1}')"
platform="$(printf '%s' "$meta" | awk -F' \\|\\|\\| ' '{print $2}')"
duration="$(printf '%s' "$meta" | awk -F' \\|\\|\\| ' '{print $3}')"
title="$(printf '%s' "$meta" | awk -F' \\|\\|\\| ' '{print $4}')"
[ "$duration" = "NA" ] && duration=""
og_json file="$path" title="$title" duration:n="$duration" size:n="$(file_size "$path")" platform="$platform" engine="yt-dlp" id="$id"
