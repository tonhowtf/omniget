#!/usr/bin/env bash
set -euo pipefail
src="${1:?path to a privacy.sexy checkout}"
here="$(cd "$(dirname "$0")" && pwd)"
dest="$here/../../src-tauri/omniget-core/data/washmybelly/privacy-sexy"
mkdir -p "$dest"
for os in windows macos linux; do
  cp "$src/src/application/collections/$os.yaml" "$dest/$os.yaml"
done
cp "$src/LICENSE" "$dest/LICENSE"
commit="$(git -C "$src" log -1 --format='%H %cs')"
echo "copied privacy.sexy at $commit; update SOURCE.md with this commit"
