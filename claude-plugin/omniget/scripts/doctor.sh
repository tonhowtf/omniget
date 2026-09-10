#!/usr/bin/env bash
# Report which tools, models and keys the omniget skill can use, and how to get the missing ones.
# Usage: doctor.sh [--json]
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=resolve-tools.sh
. "$here/resolve-tools.sh"

json=false
check_keys=false
for arg in "$@"; do
  case "$arg" in
    --json) json=true ;;
    --check-keys) check_keys=true ;;
  esac
done

rows=""
add_row() { rows="$rows$1	$2	$3
"; }

for t in omniget-cli yt-dlp ffmpeg ffprobe whisper-cli mlx_whisper; do
  if found="$(og_find_tool "$t")"; then
    add_row "$t" "$(printf '%s' "$found" | cut -f2)" "$(printf '%s' "$found" | cut -f1)"
  else
    add_row "$t" missing -
  fi
done

if model="$(og_whisper_model)"; then
  add_row whisper-model present "$model"
else
  add_row whisper-model missing -
fi

for k in GEMINI_API_KEY OPENAI_API_KEY; do
  if og_has_key "$k"; then add_row "$k" present "(set)"; else add_row "$k" missing -; fi
done

data="$(og_data_dir)"
if [ -d "$data" ]; then add_row omniget-app installed "$data"; else add_row omniget-app missing "$data"; fi
add_row output-dir configured "$(og_output_dir)"

if $json; then
  printf '{'
  first=true
  printf '%s' "$rows" | while IFS='	' read -r name status where; do
    [ -n "$name" ] || continue
    $first || printf ','
    first=false
    printf '"%s":{"status":"%s","path":"%s"}' "$name" "$status" "$(printf '%s' "$where" | sed 's/\\/\\\\/g; s/"/\\"/g')"
  done
  printf '}\n'
  exit 0
fi

printf '%-16s %-10s %s\n' TOOL STATUS WHERE
printf '%s' "$rows" | while IFS='	' read -r name status where; do
  [ -n "$name" ] || continue
  printf '%-16s %-10s %s\n' "$name" "$status" "$where"
done

echo
echo "How to fill gaps (ask the user before running any of these):"
case "$(og_os)" in
  mac)
    echo "  OmniGet app:     https://github.com/tonhowtf/omniget/releases/latest  (.dmg; then: xattr -cr /Applications/omniget.app)"
    echo "  yt-dlp + ffmpeg: brew install yt-dlp ffmpeg"
    echo "  local whisper:   brew install whisper-cpp        (whisper.cpp CLI, CPU/Metal)"
    echo "                   uv tool install mlx-whisper     (faster on Apple Silicon)"
    ;;
  linux)
    echo "  OmniGet app:     https://github.com/tonhowtf/omniget/releases/latest  (.deb/.rpm/.AppImage)"
    echo "  yt-dlp + ffmpeg: sudo apt install ffmpeg && pip install -U yt-dlp   (or your distro's packages)"
    echo "  local whisper:   whisper-bin-ubuntu-x64.tar.gz from https://github.com/ggml-org/whisper.cpp/releases"
    ;;
  windows)
    echo "  OmniGet app:     winget install -e --id tonhowtf.OmniGet"
    echo "  yt-dlp + ffmpeg: winget install yt-dlp.yt-dlp Gyan.FFmpeg"
    echo "  local whisper:   whisper-bin-x64.zip from https://github.com/ggml-org/whisper.cpp/releases"
    ;;
esac
echo "  whisper model:   $here/get-model.sh large-v3-turbo-q5_0    (547 MB, from ggerganov/whisper.cpp on Hugging Face)"
echo "  cloud keys:      export GEMINI_API_KEY=... or OPENAI_API_KEY=..., or put them in ~/.config/ai-keys.env"

if $check_keys && ! $json; then
  echo
  bash "$here/keys.sh" check
fi
