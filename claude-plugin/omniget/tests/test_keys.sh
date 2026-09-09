#!/usr/bin/env bash
# Tests for keys.sh _upsert_env: it must set/replace one var while preserving the rest.
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fail=0
check() { if [ "$2" = "$3" ]; then echo "ok: $1"; else echo "FAIL: $1 — expected [$3], got [$2]"; fail=1; fi; }

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
export OMNIGET_KEYS_FILE="$tmp/ai-keys.env"
# seed with unrelated keys that must survive
printf 'REPLICATE_API_KEY=keep-me\nANTHROPIC_API_KEY=also-keep\n' > "$OMNIGET_KEYS_FILE"

# shellcheck source=../scripts/keys.sh
. "$here/../scripts/keys.sh"

printf 'sk-openai-1' | _upsert_env OPENAI_API_KEY
printf 'gem-1'       | _upsert_env GEMINI_API_KEY
printf 'sk-openai-2' | _upsert_env OPENAI_API_KEY   # replace, not append

get() { grep -E "^$1=" "$OMNIGET_KEYS_FILE" | tail -1 | cut -d= -f2-; }
count() { grep -cE "^$1=" "$OMNIGET_KEYS_FILE"; }

check "unrelated key preserved (replicate)" "$(get REPLICATE_API_KEY)" "keep-me"
check "unrelated key preserved (anthropic)" "$(get ANTHROPIC_API_KEY)" "also-keep"
check "openai replaced not duplicated"      "$(count OPENAI_API_KEY)" "1"
check "openai has latest value"             "$(get OPENAI_API_KEY)" "sk-openai-2"
check "gemini set"                          "$(get GEMINI_API_KEY)" "gem-1"
perm="$(stat -f '%Lp' "$OMNIGET_KEYS_FILE" 2>/dev/null || stat -c '%a' "$OMNIGET_KEYS_FILE")"
check "keys file is chmod 600"              "$perm" "600"

exit $fail
