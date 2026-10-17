#!/usr/bin/env bash
set -euo pipefail

target="$1"
find /work -mindepth 1 -maxdepth 1 -exec rm -rf {} +
tar -xf /in/source.tar -C /work
cd /work
COREPACK_ENABLE_DOWNLOAD_PROMPT=0 pnpm install --frozen-lockfile
node scripts/release-local.mjs --target "$target" --cli --out /out --allow-dirty
