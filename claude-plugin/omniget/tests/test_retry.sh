#!/usr/bin/env bash
# Tests og_run_ytdlp's one-shot cookie retry using a stub yt-dlp, so no network is needed.
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fail=0
check() { if [ "$2" = "$3" ]; then echo "ok: $1"; else echo "FAIL: $1 — expected [$3] got [$2]"; fail=1; fi; }

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

# stub yt-dlp: fails with a rate-limit error unless --cookies-from-browser is passed.
cat > "$tmp/yt-dlp" <<'STUB'
#!/usr/bin/env bash
for a in "$@"; do [ "$a" = "--cookies-from-browser" ] && { echo "OK title ||| 12 ||| Some Title"; exit 0; }; done
echo "ERROR: Instagram sent an empty media response" >&2
exit 1
STUB
chmod +x "$tmp/yt-dlp"

# stub that always fails (even with cookies) -> block persists.
cat > "$tmp/yt-dlp-hardfail" <<'STUB'
#!/usr/bin/env bash
echo "ERROR: Instagram sent an empty media response" >&2
exit 1
STUB
chmod +x "$tmp/yt-dlp-hardfail"

# stub that fails with a NON-retryable error (DRM) -> must NOT retry.
cat > "$tmp/yt-dlp-drm" <<'STUB'
#!/usr/bin/env bash
echo "ERROR: HLS uses SAMPLE-AES (FairPlay DRM), cannot decrypt" >&2
exit 1
STUB
chmod +x "$tmp/yt-dlp-drm"

export OMNIGET_KEYS_FILE="$tmp/none.env"   # keep key loading inert
# shellcheck source=../scripts/resolve-tools.sh
. "$here/../scripts/resolve-tools.sh"

url="https://www.instagram.com/reel/TESTTEST/"

# 1) retry succeeds when cookies help
export OMNIGET_TOOL_YT_DLP="$tmp/yt-dlp"
err="$tmp/e1"; out="$(og_run_ytdlp "$url" "$err" --print x 2>"$tmp/notice1")"; rc=$?
check "retry: exit 0 when cookies unblock" "$rc" "0"
check "retry: returns stub stdout"         "$out" "OK title ||| 12 ||| Some Title"
grep -q "retrying with" "$tmp/notice1" && echo "ok: retry: printed the retry notice" || { echo "FAIL: retry notice missing"; fail=1; }

# 2) hard-fail: one retry, then give up non-zero (no infinite loop)
export OMNIGET_TOOL_YT_DLP="$tmp/yt-dlp-hardfail"
err="$tmp/e2"; out="$(og_run_ytdlp "$url" "$err" --print x 2>/dev/null)"; rc=$?
check "hardfail: exit non-zero"            "$rc" "1"
check "hardfail: empty stdout"             "$out" ""
grep -qi "empty media" "$err" && echo "ok: hardfail: error captured to errfile" || { echo "FAIL: errfile empty"; fail=1; }

# 3) non-retryable error (DRM) must NOT trigger a cookie retry
export OMNIGET_TOOL_YT_DLP="$tmp/yt-dlp-drm"
err="$tmp/e3"; og_run_ytdlp "$url" "$err" --print x >/dev/null 2>"$tmp/notice3"; rc=$?
check "drm: exit non-zero"                 "$rc" "1"
if grep -q "retrying with" "$tmp/notice3"; then echo "FAIL: drm should not retry"; fail=1; else echo "ok: drm: no retry attempted"; fi

exit $fail
