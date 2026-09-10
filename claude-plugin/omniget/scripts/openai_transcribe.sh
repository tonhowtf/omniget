#!/usr/bin/env bash
# Transcribe an audio file with OpenAI's /v1/audio/transcriptions and write SRT (whisper-1)
# or plain text (gpt-4o-transcribe, gpt-4o-mini-transcribe). Files over 24 MB are split into
# 20-minute parts and stitched back with time offsets. Prints the output path.
# Usage: openai_transcribe.sh AUDIO --out OUT.srt [--model whisper-1] [--lang xx]
set -eu
set -o pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$here/resolve-tools.sh"

audio=""; out=""; model="whisper-1"; lang=""
while [ $# -gt 0 ]; do
  case "$1" in
    --out) out="$2"; shift ;;
    --model) model="$2"; shift ;;
    --lang) lang="$2"; shift ;;
    -*) echo "openai_transcribe: unknown flag $1" >&2; exit 2 ;;
    *) audio="$1" ;;
  esac
  shift
done
[ -n "$audio" ] && [ -n "$out" ] || { echo "usage: openai_transcribe.sh AUDIO --out OUT.srt [--model M] [--lang xx]" >&2; exit 2; }
og_load_keys
[ -n "${OPENAI_API_KEY:-}" ] || { echo "openai_transcribe: OPENAI_API_KEY is not set" >&2; exit 1; }

case "$model" in
  whisper-1) fmt=srt; ext=srt ;;
  *) fmt=text; ext=txt ;;
esac
[ "$ext" = txt ] && out="${out%.*}.txt"

file_size() { if stat -f%z "$1" >/dev/null 2>&1; then stat -f%z "$1"; else stat -c%s "$1"; fi; }
tmp="$(mktemp -d "${TMPDIR:-/tmp}/omniget-openai.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

transcribe_one() {
  local part="$1" dest="$2" attempt
  for attempt in 1 2 3; do
    if curl -sS --fail --max-time 900 https://api.openai.com/v1/audio/transcriptions \
        -H "Authorization: Bearer $OPENAI_API_KEY" \
        -F "file=@$part" -F "model=$model" -F "response_format=$fmt" \
        ${lang:+-F "language=$lang"} -o "$dest"; then
      return 0
    fi
    echo "openai_transcribe: attempt $attempt failed, retrying" >&2
    sleep $((attempt * 5))
  done
  return 1
}

if [ "$(file_size "$audio")" -le $((24 * 1024 * 1024)) ]; then
  transcribe_one "$audio" "$out"
  echo "$out"
  exit 0
fi

ffmpeg="$(og_tool_path ffmpeg)" || { echo "openai_transcribe: ffmpeg needed to split large audio" >&2; exit 1; }
ffprobe="$(og_tool_path ffprobe)" || ffprobe="$(dirname "$ffmpeg")/ffprobe"
"$ffmpeg" -v error -y -i "$audio" -f segment -segment_time 1200 -c copy "$tmp/part%03d.${audio##*.}"

offset=0
parts=()
for part in "$tmp"/part*; do
  dest="$tmp/$(basename "${part%.*}").$ext"
  transcribe_one "$part" "$dest"
  parts+=("$dest:$offset")
  dur="$("$ffprobe" -v error -show_entries format=duration -of csv=p=0 "$part")"
  offset="$(python3 -c "print($offset + $dur)")"
done

if [ "$ext" = srt ]; then
  python3 "$here/srt_tools.py" concat --out "$out" "${parts[@]}"
else
  : > "$out"
  for spec in "${parts[@]}"; do cat "${spec%:*}" >> "$out"; printf '\n' >> "$out"; done
fi
echo "$out"
