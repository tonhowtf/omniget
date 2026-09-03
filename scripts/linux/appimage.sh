#! /usr/bin/env bash
#
# Builds the Linux AppImage with our patched linuxdeploy-plugin-gtk.
#
# The Tauri bundler downloads linuxdeploy-plugin-gtk.sh once and never
# re-checks it, so dropping our copy where it looks pins that copy for every
# later build. We therefore turn on `bundle.useLocalToolsDir`, which moves the
# bundler's tool cache from the shared ~/.cache/tauri into this project's cargo
# target directory. Nothing outside the repo is touched, and `cargo clean`
# undoes it.

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

# NO_STRIP: linuxdeploy's strip step fails on the .relr.dyn sections newer
# linkers emit. createUpdaterArtifacts: a local build has no signing key.
exec env NO_STRIP=1 node_modules/.bin/tauri build --bundles appimage \
    --config '{"bundle":{"useLocalToolsDir":true,"createUpdaterArtifacts":false}}' "$@"
