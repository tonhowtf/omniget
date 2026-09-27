#!/usr/bin/env bash
# Regenerate the golden files of the Rust privacy.sexy compiler with the
# original TypeScript compiler.
#
#   scripts/washmybelly/privacy_golden.sh <privacy.sexy checkout> [count]
#
# Writes src-tauri/omniget-core/tests/washmybelly_fixtures/privacy/golden_<os>.json
# (a sample of `count` scripts per system, default 30) and leaves the full
# outputs in the temporary folder it prints, for
#   PRIVACY_GOLDEN_FULL=<that folder> cargo test -p omniget-core --lib \
#     washmybelly::privacy -- --ignored full_golden
# Needs node and network access for `npm install tsx js-yaml` (once).
set -euo pipefail
src="${1:?path to a privacy.sexy checkout}"
count="${2:-30}"
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$root/src-tauri/omniget-core/tests/washmybelly_fixtures/privacy"
work="$(mktemp -d "${TMPDIR:-/tmp}/privacy-golden.XXXXXX")"
cp -R "$src/src" "$work/src"
cp "$src/tsconfig.json" "$work/"
cp "$here/privacy_golden.ts" "$work/"
echo '{"name":"privacy-golden","private":true,"type":"module"}' > "$work/package.json"
(cd "$work" && npm install --silent --no-audit --no-fund tsx js-yaml >/dev/null)
mkdir -p "$out"
for os in windows macos linux; do
  yaml="$root/src-tauri/omniget-core/data/washmybelly/privacy-sexy/$os.yaml"
  (cd "$work" && npx tsx privacy_golden.ts "$yaml" "$work/full_$os.json" "$out/golden_$os.json" "$count")
done
echo "full outputs: $work"
