#!/usr/bin/env bash
# Manage and test the transcription API keys the skill uses (OpenAI, Gemini).
#
#   keys.sh set     Interactively add/replace keys in ~/.config/ai-keys.env (hidden
#                   input, file chmod 600). Run this in YOUR OWN terminal — it needs
#                   a prompt and Claude must never type your keys.
#   keys.sh check   Test each configured key against its provider and report
#                   working / bad / not-set. Safe for anyone (Claude included) to run;
#                   never prints key values.
#
# Keys live in ~/.config/ai-keys.env (override with OMNIGET_KEYS_FILE). OpenRouter is
# not supported: it has no speech-to-text endpoint, so it can't transcribe.
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=resolve-tools.sh
. "$here/resolve-tools.sh"

KEYS_FILE="${OMNIGET_KEYS_FILE:-$HOME/.config/ai-keys.env}"

# _upsert_env NAME  (value read from stdin) — set NAME in KEYS_FILE, preserving every
# other line. Kept value-on-stdin (never argv) so it stays out of ps and shell history.
_upsert_env() {
  local name="$1" value file tmp
  value="$(cat)"
  file="$KEYS_FILE"
  mkdir -p "$(dirname "$file")"
  [ -f "$file" ] || { : > "$file"; chmod 600 "$file"; }
  tmp="$(mktemp "${TMPDIR:-/tmp}/omniget-keys.XXXXXX")"
  awk -v n="$name" 'BEGIN{FS="="} $1!=n && $1!="export "n {print}' "$file" > "$tmp"
  printf '%s=%s\n' "$name" "$value" >> "$tmp"
  chmod 600 "$tmp"
  mv "$tmp" "$file"
  chmod 600 "$file"
}

cmd_set() {
  if [ ! -t 0 ]; then
    echo "keys.sh set needs an interactive terminal (it hides what you type)." >&2
    echo "Open your own terminal and run:  bash \"$here/keys.sh\" set" >&2
    return 2
  fi
  echo "Add transcription API keys. Leave a field blank to keep the current value."
  echo "Keys are written to $KEYS_FILE (permissions 600) and never printed."
  echo
  og_load_keys
  local existing
  for spec in "OpenAI:OPENAI_API_KEY:https://platform.openai.com/api-keys" \
              "Gemini:GEMINI_API_KEY:https://aistudio.google.com/apikey"; do
    local label var url
    label="${spec%%:*}"; var="$(printf '%s' "$spec" | cut -d: -f2)"; url="${spec#*:*:}"
    existing="$(eval "printf '%s' \"\${$var:-}\"")"
    if [ -n "$existing" ]; then
      printf '%s key [already set, press Enter to keep] (%s): ' "$label" "$url"
    else
      printf '%s key [%s]: ' "$label" "$url"
    fi
    local value=""
    read -r -s value
    echo
    if [ -n "$value" ]; then
      printf '%s' "$value" | _upsert_env "$var"
      echo "  saved $var"
    fi
  done
  echo
  cmd_check
}

# http_status URL [header] — GET, print the numeric HTTP status (000 on network failure).
_http_status() {
  local url="$1" header="${2:-}"
  if [ -n "$header" ]; then
    curl -s -o /dev/null -w '%{http_code}' -m 20 -H "$header" "$url" 2>/dev/null || echo 000
  else
    curl -s -o /dev/null -w '%{http_code}' -m 20 "$url" 2>/dev/null || echo 000
  fi
}

_report() { printf '  %-14s %s\n' "$1" "$2"; }

cmd_check() {
  og_load_keys
  echo "Transcription API keys ($KEYS_FILE):"
  local any=false

  if [ -n "${OPENAI_API_KEY:-}" ]; then
    any=true
    case "$(_http_status https://api.openai.com/v1/models "Authorization: Bearer $OPENAI_API_KEY")" in
      200) _report OpenAI "working" ;;
      401|403) _report OpenAI "BAD KEY (rejected)" ;;
      000) _report OpenAI "no network / timeout" ;;
      *) _report OpenAI "unexpected response" ;;
    esac
  else
    _report OpenAI "not set"
  fi

  if [ -n "${GEMINI_API_KEY:-}" ]; then
    any=true
    case "$(_http_status "https://generativelanguage.googleapis.com/v1beta/models?key=$GEMINI_API_KEY")" in
      200) _report Gemini "working" ;;
      400|401|403) _report Gemini "BAD KEY (rejected)" ;;
      000) _report Gemini "no network / timeout" ;;
      *) _report Gemini "unexpected response" ;;
    esac
  else
    _report Gemini "not set"
  fi

  echo
  if $any; then
    echo "A 'working' key lets you use --backend openai / --backend gemini for accurate transcription."
  else
    echo "No cloud keys set. You can still use platform captions or a local Whisper model,"
    echo "or add a key with:  bash \"$here/keys.sh\" set"
  fi
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  case "${1:-}" in
    set) cmd_set ;;
    check|"") cmd_check ;;
    *) echo "usage: keys.sh set|check" >&2; exit 2 ;;
  esac
fi
