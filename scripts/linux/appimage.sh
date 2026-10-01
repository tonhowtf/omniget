#! /usr/bin/env bash

set -e

cd "$(dirname "$0")/../.."

target_dir=$(cargo metadata --no-deps --format-version 1 \
    --manifest-path src-tauri/Cargo.toml |
    node -e 'let s="";process.stdin.on("data",c=>s+=c).on("end",()=>console.log(JSON.parse(s).target_directory))')

tools_dir="$target_dir/.tauri"
mkdir -p "$tools_dir"
cp scripts/linux/linuxdeploy-plugin-gtk.sh "$tools_dir/"
chmod +x "$tools_dir/linuxdeploy-plugin-gtk.sh"
echo "Using patched linuxdeploy-plugin-gtk.sh from $tools_dir"
echo "Tool cache is project-local (target/.tauri/); 'cargo clean' removes it."

exec env NO_STRIP=1 node_modules/.bin/tauri build --bundles appimage \
    --config '{"bundle":{"useLocalToolsDir":true,"createUpdaterArtifacts":false}}' "$@"
