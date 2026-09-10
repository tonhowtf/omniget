#!/usr/bin/env bash
# Download a whisper.cpp ggml model from Hugging Face (ggerganov/whisper.cpp) and verify its SHA-256.
# Usage: get-model.sh <name>      e.g. large-v3-turbo-q5_0 (547 MB), base (141 MB), small (465 MB)
# Destination: OmniGet's models/whisper dir when it exists, else ~/.cache/omniget-skill/models.
set -eu
set -o pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$here/resolve-tools.sh"

name="${1:?usage: get-model.sh <model-name>}"
name="${name#ggml-}"; name="${name%.bin}"
file="ggml-$name.bin"

dest_dir="$(og_data_dir)/models/whisper"
[ -d "$dest_dir" ] || dest_dir="$(og_skill_cache_dir)/models"
mkdir -p "$dest_dir"
dest="$dest_dir/$file"
if [ -f "$dest" ]; then
  echo "$dest"
  exit 0
fi

expected="$(curl -sSf -m 30 "https://huggingface.co/api/models/ggerganov/whisper.cpp/tree/main" \
  | python3 -c 'import json,sys; want=sys.argv[1]
for e in json.load(sys.stdin):
    if e.get("path")==want: print((e.get("lfs") or {}).get("oid","")); break' "$file")"
[ -n "$expected" ] || { echo "get-model: $file is not in ggerganov/whisper.cpp" >&2; exit 1; }

tmp="$dest.part"
echo "Downloading $file to $dest_dir ..." >&2
curl -L --fail --progress-bar -o "$tmp" "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/$file"

if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp" | cut -d' ' -f1)"
else
  actual="$(shasum -a 256 "$tmp" | cut -d' ' -f1)"
fi
if [ "$actual" != "$expected" ]; then
  rm -f "$tmp"
  echo "get-model: SHA-256 mismatch for $file (expected $expected, got $actual)" >&2
  exit 1
fi
mv "$tmp" "$dest"
echo "$dest"
