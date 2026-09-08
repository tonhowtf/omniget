#!/usr/bin/env bash
# Produce a transcript for a media URL or local file. Prints one JSON line:
# {transcript, srt?, backend, model?, title, duration?, chars, source}
#
# Ladder (backend=auto): platform captions -> whisper-cli (local) -> mlx_whisper (local, Apple Silicon)
#                        -> Gemini (GEMINI_API_KEY) -> OpenAI (OPENAI_API_KEY)
# Usage: transcribe.sh <url|file> [--backend auto|captions|local|mlx|gemini|openai]
#                      [--model NAME] [--lang xx] [--out DIR] [--keep-audio] [--title T]
set -eu
set -o pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$here/resolve-tools.sh"

input=""; backend="auto"; model=""; lang=""; out=""; keep_audio=false; title=""
while [ $# -gt 0 ]; do
  case "$1" in
    --backend) backend="$2"; shift ;;
    --model) model="$2"; shift ;;
    --lang) lang="$2"; shift ;;
    --out) out="$2"; shift ;;
    --title) title="$2"; shift ;;
    --keep-audio) keep_audio=true ;;
    -*) echo "transcribe: unknown flag $1" >&2; exit 2 ;;
    *) input="$1" ;;
  esac
  shift
done
[ -n "$input" ] || { echo "usage: transcribe.sh <url|file> [--backend B] [--model M] [--lang xx] [--out DIR]" >&2; exit 2; }
case "$backend" in auto|captions|local|mlx|gemini|openai) ;; *) echo "transcribe: bad --backend $backend" >&2; exit 2 ;; esac
[ -n "$out" ] || out="$(og_output_dir)/transcripts"
mkdir -p "$out"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/omniget-transcribe.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
log() { echo "transcribe: $*" >&2; }

id=""; duration=""; source="$input"; audio=""
if og_is_url "$input"; then
  ytdlp="$(og_tool_path yt-dlp)" || { log "yt-dlp not found; run doctor.sh"; exit 1; }
  cookie_args=()
  while IFS= read -r line; do cookie_args+=("$line"); done < <(og_cookie_args "$input")

  # One yt-dlp call fetches metadata and any captions. --print implies --skip-download.
  sub_langs="en.*,en,.*-orig"
  [ -n "$lang" ] && sub_langs="$lang.*,$lang,$sub_langs"
  meta="$("$ytdlp" --no-warnings --no-playlist --write-subs --write-auto-subs \
      --sub-langs "$sub_langs" --convert-subs srt -o "$tmp/cap.%(ext)s" \
      --print "%(id)s ||| %(duration)s ||| %(title)s" ${cookie_args[@]+${cookie_args[@]+"${cookie_args[@]}"}} "$input" | head -n 1)"
  id="$(printf '%s' "$meta" | awk -F' \\|\\|\\| ' '{print $1}')"
  duration="$(printf '%s' "$meta" | awk -F' \\|\\|\\| ' '{print $2}')"
  [ -n "$title" ] || title="$(printf '%s' "$meta" | awk -F' \\|\\|\\| ' '{print $3}')"
  [ "$duration" = "NA" ] && duration=""

  caption=""
  if [ "$backend" = auto ] || [ "$backend" = captions ]; then
    caption="$(ls -S "$tmp"/cap.*.srt "$tmp"/cap.*.vtt 2>/dev/null | head -n 1 || true)"
    if [ -z "$caption" ]; then
      "$ytdlp" --no-warnings --no-playlist --skip-download --write-subs --sub-langs all \
        --convert-subs srt -o "$tmp/cap.%(ext)s" ${cookie_args[@]+${cookie_args[@]+"${cookie_args[@]}"}} "$input" >/dev/null 2>&1 || true
      caption="$(ls -S "$tmp"/cap.*.srt "$tmp"/cap.*.vtt 2>/dev/null | head -n 1 || true)"
    fi
  fi
  if [ -n "$caption" ] && [ "$(python3 "$here/srt_tools.py" stats "$caption" | python3 -c 'import json,sys; print(json.load(sys.stdin)["chars"])')" -gt 20 ]; then
    srt_src="$caption"; used="captions"; used_model="$(basename "$caption" | sed 's/^cap\.//; s/\.[a-z]*$//')"
  else
    [ "$backend" = captions ] && { log "no usable captions for $input"; exit 3; }
    log "no captions; downloading audio"
    audio="$("$ytdlp" --no-warnings --no-playlist --no-simulate --quiet -f "bestaudio/best" \
        -o "$tmp/audio.%(ext)s" --print "after_move:filepath" ${cookie_args[@]+${cookie_args[@]+"${cookie_args[@]}"}} "$input" | tail -n 1)"
  fi
else
  [ -f "$input" ] || { log "no such file: $input"; exit 1; }
  audio="$input"
  [ -n "$title" ] || title="$(basename "${input%.*}")"
  source="$(cd "$(dirname "$input")" && pwd)/$(basename "$input")"
  [ "$backend" = captions ] && { log "--backend captions needs a URL"; exit 2; }
fi

if [ -n "$audio" ]; then
  ffmpeg="$(og_tool_path ffmpeg)" || { log "ffmpeg not found; run doctor.sh"; exit 1; }
  if [ -z "$duration" ] && ffprobe="$(og_tool_path ffprobe)"; then
    duration="$("$ffprobe" -v error -show_entries format=duration -of csv=p=0 "$audio" 2>/dev/null || true)"
  fi

  if [ "$backend" = auto ]; then
    if og_find_tool whisper-cli >/dev/null && og_whisper_model >/dev/null; then backend=local
    elif og_find_tool mlx_whisper >/dev/null; then backend=mlx
    elif og_has_key GEMINI_API_KEY; then backend=gemini
    elif og_has_key OPENAI_API_KEY; then backend=openai
    else
      log "no transcription backend available: install whisper-cli + a model, mlx-whisper, or set GEMINI_API_KEY / OPENAI_API_KEY (see doctor.sh)"
      exit 4
    fi
  fi
  log "backend: $backend"

  case "$backend" in
    local)
      cli="$(og_tool_path whisper-cli)" || { log "whisper-cli not found (brew install whisper-cpp)"; exit 4; }
      if [ -n "$model" ]; then
        [ -f "$model" ] || model="$("$here/get-model.sh" "$model")"
      else
        model="$(og_whisper_model)" || { log "no whisper model; run get-model.sh large-v3-turbo-q5_0"; exit 4; }
      fi
      "$ffmpeg" -v error -y -i "$audio" -ac 1 -ar 16000 -c:a pcm_s16le "$tmp/a16.wav"
      threads="$( (sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4) )"
      [ "$threads" -gt 8 ] && threads=8
      "$cli" -m "$model" -f "$tmp/a16.wav" -l "${lang:-auto}" -t "$threads" -osrt -of "$tmp/out" -np -pp 1>&2
      srt_src="$tmp/out.srt"; used=local; used_model="$(basename "$model")"
      ;;
    mlx)
      mlx="$(og_tool_path mlx_whisper)" || { log "mlx_whisper not found (uv tool install mlx-whisper)"; exit 4; }
      [ -n "$model" ] || model="mlx-community/whisper-large-v3-turbo"
      "$ffmpeg" -v error -y -i "$audio" -ac 1 -ar 16000 -c:a pcm_s16le "$tmp/a16.wav"
      "$mlx" "$tmp/a16.wav" --model "$model" --output-dir "$tmp" --output-name out --output-format srt \
        ${lang:+--language "$lang"} 1>&2
      srt_src="$tmp/out.srt"; used=mlx; used_model="$model"
      ;;
    gemini)
      og_load_keys
      [ -n "$model" ] || model="gemini-2.5-flash"
      "$ffmpeg" -v error -y -i "$audio" -ac 1 -ar 16000 -b:a 32k "$tmp/a16.mp3"
      result="$(python3 "$here/gemini_transcribe.py" "$tmp/a16.mp3" --out "$tmp/out.srt" --model "$model" ${lang:+--lang "$lang"})"
      used=gemini; used_model="$model"
      case "$result" in *.srt) srt_src="$result" ;; *) text_src="$result" ;; esac
      ;;
    openai)
      og_load_keys
      [ -n "$model" ] || model="whisper-1"
      "$ffmpeg" -v error -y -i "$audio" -ac 1 -ar 16000 -b:a 32k "$tmp/a16.mp3"
      result="$("$here/openai_transcribe.sh" "$tmp/a16.mp3" --out "$tmp/out.srt" --model "$model" ${lang:+--lang "$lang"})"
      used=openai; used_model="$model"
      case "$result" in *.srt) srt_src="$result" ;; *) text_src="$result" ;; esac
      ;;
  esac
fi

slug="$(og_slug "$title")"
[ -n "$id" ] && slug="$slug-$id"
md="$out/$slug.md"
srt=""
if [ -n "${srt_src:-}" ]; then
  srt="$out/$slug.srt"
  cp "$srt_src" "$srt"
  python3 "$here/srt_tools.py" to-md "$srt" --title "$title" --source "$source" --backend "$used" --model "$used_model" > "$md"
else
  {
    printf '# %s\n\n- Source: %s\n- Transcribed with: %s (%s)\n\n' "$title" "$source" "$used" "$used_model"
    cat "$text_src"
  } > "$md"
fi

kept=""
if $keep_audio && [ -n "$audio" ] && og_is_url "$input"; then
  kept="$out/$slug.${audio##*.}"
  cp "$audio" "$kept"
fi

chars="$(python3 -c 'import sys; print(len(open(sys.argv[1], encoding="utf-8").read()))' "$md")"
og_json transcript="$md" srt="$srt" backend="$used" model="$used_model" title="$title" duration:n="$duration" chars:n="$chars" source="$source" audio="$kept"
