#!/usr/bin/env bash
# Locate the tools the omniget skill relies on, preferring OmniGet's own copies.
#
# Lookup order mirrors OmniGet's find_tool_with_source:
#   1. explicit override   OMNIGET_TOOL_<NAME>=/path   (e.g. OMNIGET_TOOL_YT_DLP)
#   2. OmniGet managed dir <data dir>/bin              (source "omniget")
#   3. PATH                                            (source "system")
#
# Usage:  resolve-tools.sh [tool ...]      prints "name<TAB>path<TAB>source" per tool
#         . resolve-tools.sh               then call og_find_tool / og_whisper_model / og_load_keys
#
# Bash 3.2 compatible (macOS default shell).

og_os() {
  case "$(uname -s)" in
    Darwin) echo mac ;;
    Linux) echo linux ;;
    MINGW*|MSYS*|CYGWIN*) echo windows ;;
    *) echo other ;;
  esac
}

# OmniGet's app data directory (see omniget-core/src/core/paths.rs).
og_data_dir() {
  if [ -n "${OMNIGET_DATA_DIR:-}" ]; then
    echo "$OMNIGET_DATA_DIR"
    return
  fi
  case "$(og_os)" in
    mac) echo "$HOME/Library/Application Support/wtf.tonho.omniget" ;;
    linux) echo "${XDG_DATA_HOME:-$HOME/.local/share}/wtf.tonho.omniget" ;;
    windows) echo "${APPDATA:-$HOME/AppData/Roaming}/wtf.tonho.omniget" ;;
    *) echo "$HOME/.omniget" ;;
  esac
}

# Where the skill keeps its own downloads (models) when OmniGet has none.
og_skill_cache_dir() {
  echo "${OMNIGET_SKILL_CACHE:-$HOME/.cache/omniget-skill}"
}

# Output directory for media and transcripts.
og_output_dir() {
  echo "${OMNIGET_DIR:-$HOME/Downloads/omniget}"
}

og_exe() {
  if [ "$(og_os)" = windows ]; then echo "$1.exe"; else echo "$1"; fi
}

# og_find_tool NAME -> prints "path<TAB>source", exit 1 when missing.
og_find_tool() {
  local name="$1" var path alt
  var="OMNIGET_TOOL_$(printf '%s' "$name" | tr 'a-z-' 'A-Z_')"
  path="$(eval "printf '%s' \"\${$var:-}\"")"
  if [ -n "$path" ] && [ -x "$path" ]; then
    printf '%s\tcustom\n' "$path"
    return 0
  fi
  path="$(og_data_dir)/bin/$(og_exe "$name")"
  if [ -x "$path" ]; then
    printf '%s\tomniget\n' "$path"
    return 0
  fi
  if path="$(command -v "$name" 2>/dev/null)"; then
    printf '%s\tsystem\n' "$path"
    return 0
  fi
  # Homebrew's older formula installed the whisper.cpp CLI as whisper-cpp.
  if [ "$name" = whisper-cli ] && alt="$(command -v whisper-cpp 2>/dev/null)"; then
    printf '%s\tsystem\n' "$alt"
    return 0
  fi
  return 1
}

og_tool_path() {
  og_find_tool "$1" | cut -f1
}

# Best available ggml model for whisper-cli. Preference: large-v3-turbo (quantised first),
# then medium, small, base, tiny. Looks in OmniGet's models dir, then the skill cache.
og_whisper_model() {
  local dir candidate
  if [ -n "${OMNIGET_WHISPER_MODEL:-}" ] && [ -f "$OMNIGET_WHISPER_MODEL" ]; then
    echo "$OMNIGET_WHISPER_MODEL"
    return 0
  fi
  for dir in "$(og_data_dir)/models/whisper" "$(og_skill_cache_dir)/models"; do
    [ -d "$dir" ] || continue
    for candidate in \
      ggml-large-v3-turbo-q5_0.bin ggml-large-v3-turbo-q8_0.bin ggml-large-v3-turbo.bin \
      ggml-large-v3-q5_0.bin ggml-large-v3.bin \
      ggml-medium-q5_0.bin ggml-medium-q8_0.bin ggml-medium.bin \
      ggml-small-q5_1.bin ggml-small-q8_0.bin ggml-small.bin \
      ggml-base-q5_1.bin ggml-base-q8_0.bin ggml-base.bin \
      ggml-tiny-q5_1.bin ggml-tiny-q8_0.bin ggml-tiny.bin; do
      if [ -f "$dir/$candidate" ]; then
        echo "$dir/$candidate"
        return 0
      fi
    done
  done
  return 1
}

# Newest cookies.txt OmniGet holds for a domain (e.g. instagram.com), if any.
og_cookie_file() {
  local domain="$1" root dir
  root="$(og_data_dir)/cookies"
  [ -d "$root" ] || return 1
  for dir in "$root"/*"$domain"*; do
    [ -d "$dir" ] || continue
    ls -t "$dir"/*.txt 2>/dev/null | head -n 1
    return 0
  done
  return 1
}

# Load API keys from the environment or ~/.config/ai-keys.env. Never prints values.
og_load_keys() {
  local file="${OMNIGET_KEYS_FILE:-$HOME/.config/ai-keys.env}"
  if [ -f "$file" ]; then
    set -a
    # shellcheck disable=SC1090
    . "$file"
    set +a
  fi
}

og_has_key() {
  og_load_keys
  [ -n "$(eval "printf '%s' \"\${$1:-}\"")" ]
}

# og_json key=value key:n=number key:b=bool ... -> one JSON object on stdout. Empty values are omitted.
og_json() {
  python3 - "$@" <<'PY'
import json, sys
out = {}
for kv in sys.argv[1:]:
    key, _, value = kv.partition("=")
    if value == "":
        continue
    name, _, kind = key.partition(":")
    if kind == "n":
        try:
            out[name] = int(value)
        except ValueError:
            out[name] = float(value)
    elif kind == "b":
        out[name] = value.lower() in ("1", "true", "yes")
    else:
        out[name] = value
print(json.dumps(out, ensure_ascii=False))
PY
}

# og_slug TEXT -> filesystem-safe ASCII slug, max 80 chars.
og_slug() {
  python3 - "$1" <<'PY'
import re, sys, unicodedata
text = unicodedata.normalize("NFKD", sys.argv[1]).encode("ascii", "ignore").decode()
text = re.sub(r"[^A-Za-z0-9]+", "_", text).strip("_")
print((text or "untitled")[:80].rstrip("_"))
PY
}

# og_url_host URL -> bare host without www./m. prefixes.
og_url_host() {
  printf '%s\n' "$1" | sed -E 's#^[A-Za-z]+://##; s#[/?].*##; s#^(www|m|mobile)\.##'
}

# og_cookie_args URL -> yt-dlp cookie flags for that host, if OmniGet or the user provides any.
og_cookie_args() {
  local host file
  host="$(og_url_host "$1")"
  case "$host" in
    x.com) host=twitter.com ;;
  esac
  if file="$(og_cookie_file "$host")" && [ -n "$file" ]; then
    printf -- '--cookies\n%s\n' "$file"
  elif [ -n "${OMNIGET_COOKIES_FROM_BROWSER:-}" ]; then
    printf -- '--cookies-from-browser\n%s\n' "$OMNIGET_COOKIES_FROM_BROWSER"
  fi
}

og_is_url() {
  case "$1" in
    http://*|https://*) return 0 ;;
    *) return 1 ;;
  esac
}

# Hosts where omniget-cli's native extractors beat plain yt-dlp.
og_prefers_cli() {
  case "$(og_url_host "$1")" in
    instagram.com|threads.net|threads.com|twitter.com|x.com|bilibili.com|b23.tv) return 0 ;;
    *) return 1 ;;
  esac
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  tools="$*"
  [ -n "$tools" ] || tools="omniget-cli yt-dlp ffmpeg ffprobe whisper-cli mlx_whisper"
  for t in $tools; do
    if found="$(og_find_tool "$t")"; then
      printf '%s\t%s\n' "$t" "$found"
    else
      printf '%s\t-\tmissing\n' "$t"
    fi
  done
fi
