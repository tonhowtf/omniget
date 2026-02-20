#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MANIFEST="$ROOT/flatpak/wtf.tonho.omniget.yml"
METAINFO="$ROOT/flatpak/wtf.tonho.omniget.metainfo.xml"
DESKTOP="$ROOT/flatpak/wtf.tonho.omniget.desktop"
TAURI_CONF="$ROOT/src-tauri/tauri.conf.json"
ICON_128="$ROOT/flatpak/wtf.tonho.omniget.png"

PASS=0
FAIL=0
WARN=0

pass() { PASS=$((PASS + 1)); printf "  \033[32mPASS\033[0m  %s\n" "$1"; }
fail() { FAIL=$((FAIL + 1)); printf "  \033[31mFAIL\033[0m  %s\n" "$1"; }
warn() { WARN=$((WARN + 1)); printf "  \033[33mWARN\033[0m  %s\n" "$1"; }
section() { printf "\n\033[1m[%s]\033[0m\n" "$1"; }

section "Tools"

if command -v flatpak-builder &>/dev/null; then
    pass "flatpak-builder installed ($(flatpak-builder --version 2>/dev/null || echo 'unknown'))"
else
    warn "flatpak-builder not installed (needed for local builds only)"
fi

if command -v appstreamcli &>/dev/null; then
    pass "appstreamcli installed"
else
    warn "appstreamcli not installed (metainfo validation skipped)"
fi

section "Manifest"

if [ -f "$MANIFEST" ]; then
    pass "Manifest exists: flatpak/wtf.tonho.omniget.yml"
else
    fail "Manifest not found: flatpak/wtf.tonho.omniget.yml"
fi

if [ -f "$MANIFEST" ]; then
    YAML_VALIDATED=false
    if command -v python3 &>/dev/null && python3 -c "import yaml" 2>/dev/null; then
        if python3 -c "import yaml; yaml.safe_load(open('$MANIFEST'))" 2>/dev/null; then
            pass "Manifest is valid YAML"
            YAML_VALIDATED=true
        else
            fail "Manifest is not valid YAML"
            YAML_VALIDATED=true
        fi
    fi
    if [ "$YAML_VALIDATED" = false ] && command -v yq &>/dev/null; then
        if yq '.' "$MANIFEST" >/dev/null 2>&1; then
            pass "Manifest is valid YAML"
            YAML_VALIDATED=true
        else
            fail "Manifest is not valid YAML"
            YAML_VALIDATED=true
        fi
    fi
    if [ "$YAML_VALIDATED" = false ]; then
        warn "No YAML validator available (install python3-yaml or yq)"
    fi
fi

section "Runtime"

if [ -f "$MANIFEST" ]; then
    RUNTIME_VERSION=$(grep 'runtime-version:' "$MANIFEST" | head -1 | sed "s/.*runtime-version:[[:space:]]*['\"]*//" | sed "s/['\"].*//")
    if [ -n "$RUNTIME_VERSION" ]; then
        pass "Runtime version specified: GNOME $RUNTIME_VERSION"
        if [ "$RUNTIME_VERSION" -lt 47 ] 2>/dev/null; then
            warn "Runtime version $RUNTIME_VERSION may be outdated (current stable: 48)"
        fi
    else
        fail "Could not parse runtime-version from manifest"
    fi

    if grep -q 'org.freedesktop.Sdk.Extension.rust-stable' "$MANIFEST"; then
        pass "Rust SDK extension declared"
    else
        fail "Missing rust-stable SDK extension"
    fi

    if grep -q 'org.freedesktop.Sdk.Extension.node' "$MANIFEST"; then
        pass "Node SDK extension declared"
    else
        fail "Missing Node SDK extension"
    fi
fi

section "Sandbox Permissions"

if [ -f "$MANIFEST" ]; then
    if grep -q '\-\-share=network' "$MANIFEST"; then
        pass "Network access granted"
    else
        fail "Missing --share=network (required for downloads)"
    fi

    if grep -q '\-\-socket=wayland' "$MANIFEST"; then
        pass "Wayland socket granted"
    else
        warn "Missing --socket=wayland"
    fi

    if grep -q '\-\-socket=fallback-x11' "$MANIFEST"; then
        pass "X11 fallback socket granted"
    else
        warn "Missing --socket=fallback-x11"
    fi

    if grep -q '\-\-device=dri' "$MANIFEST"; then
        pass "DRI device access granted (GPU)"
    else
        warn "Missing --device=dri (GPU acceleration unavailable)"
    fi

    if grep -q 'xdg-download' "$MANIFEST"; then
        pass "Filesystem access: xdg-download"
    else
        fail "Missing xdg-download filesystem access"
    fi

    if grep -q 'org.freedesktop.Notifications' "$MANIFEST"; then
        pass "D-Bus: Notifications"
    else
        warn "Missing Notifications D-Bus access"
    fi

    if grep -q 'org.kde.StatusNotifierWatcher' "$MANIFEST"; then
        pass "D-Bus: StatusNotifierWatcher (tray icon)"
    else
        warn "Missing StatusNotifierWatcher D-Bus access (tray may not work)"
    fi
fi

section "Identifier Consistency"

if [ -f "$TAURI_CONF" ] && [ -f "$MANIFEST" ]; then
    TAURI_ID=$(grep '"identifier"' "$TAURI_CONF" | head -1 | sed 's/.*"identifier"[[:space:]]*:[[:space:]]*"//' | sed 's/".*//')
    FLATPAK_ID=$(grep 'app-id:' "$MANIFEST" | head -1 | sed 's/.*app-id:[[:space:]]*//')

    if [ -n "$TAURI_ID" ] && [ -n "$FLATPAK_ID" ]; then
        if [ "$TAURI_ID" = "$FLATPAK_ID" ]; then
            pass "Identifier match: $TAURI_ID"
        else
            fail "Identifier mismatch: tauri.conf.json=$TAURI_ID, manifest=$FLATPAK_ID"
        fi
    else
        fail "Could not extract identifiers"
    fi
fi

section "Version Consistency"

if [ -f "$TAURI_CONF" ] && [ -f "$METAINFO" ]; then
    TAURI_VER=$(grep '"version"' "$TAURI_CONF" | head -1 | sed 's/.*"version"[[:space:]]*:[[:space:]]*"//' | sed 's/".*//')
    METAINFO_VER=$(grep '<release version=' "$METAINFO" | head -1 | sed 's/.*version="//' | sed 's/".*//')
    SCREENSHOT_VER=$(grep 'raw.githubusercontent.com' "$METAINFO" | head -1 | sed 's/.*\/v//' | sed 's/\/.*//')

    if [ -n "$TAURI_VER" ] && [ -n "$METAINFO_VER" ]; then
        if [ "$TAURI_VER" = "$METAINFO_VER" ]; then
            pass "Version match: $TAURI_VER (tauri.conf.json == metainfo.xml)"
        else
            fail "Version mismatch: tauri.conf.json=$TAURI_VER, metainfo.xml=$METAINFO_VER"
        fi
    fi

    if [ -n "$TAURI_VER" ] && [ -n "$SCREENSHOT_VER" ]; then
        if [ "$TAURI_VER" = "$SCREENSHOT_VER" ]; then
            pass "Screenshot URL version match: v$SCREENSHOT_VER"
        else
            fail "Screenshot URL version mismatch: expected v$TAURI_VER, found v$SCREENSHOT_VER"
        fi
    fi
fi

section "Icons"

if [ -f "$ICON_128" ]; then
    pass "Flatpak icon exists: wtf.tonho.omniget.png"
    if command -v file &>/dev/null; then
        ICON_INFO=$(file "$ICON_128")
        if echo "$ICON_INFO" | grep -q 'PNG'; then
            pass "Icon is valid PNG"
        else
            fail "Icon is not a PNG file"
        fi
    fi
else
    fail "Flatpak icon missing: flatpak/wtf.tonho.omniget.png"
fi

if [ -f "$ROOT/src-tauri/icons/128x128.png" ]; then
    pass "Tauri icon 128x128 exists"
else
    fail "Tauri icon 128x128.png missing"
fi

section "Desktop File"

if [ -f "$DESKTOP" ]; then
    pass "Desktop file exists"

    if grep -q "^Exec=omniget" "$DESKTOP"; then
        pass "Desktop Exec matches binary name"
    else
        fail "Desktop Exec does not match expected binary 'omniget'"
    fi

    if grep -q "^Icon=wtf.tonho.omniget" "$DESKTOP"; then
        pass "Desktop Icon uses reverse-DNS ID"
    else
        fail "Desktop Icon should be wtf.tonho.omniget"
    fi

    if grep -q "^Categories=" "$DESKTOP"; then
        pass "Desktop Categories defined"
    else
        warn "Desktop Categories missing"
    fi
else
    fail "Desktop file not found: flatpak/wtf.tonho.omniget.desktop"
fi

section "AppStream Metainfo"

if [ -f "$METAINFO" ]; then
    pass "Metainfo file exists"

    if grep -q '<id>wtf.tonho.omniget</id>' "$METAINFO"; then
        pass "Metainfo ID matches"
    else
        fail "Metainfo ID does not match wtf.tonho.omniget"
    fi

    if grep -q '<releases>' "$METAINFO"; then
        pass "Metainfo has <releases> section"
    else
        fail "Metainfo missing <releases> section"
    fi

    if grep -q '<screenshots>' "$METAINFO"; then
        pass "Metainfo has <screenshots> section"
    else
        warn "Metainfo missing <screenshots> (recommended by Flathub)"
    fi

    if grep -q 'content_rating' "$METAINFO"; then
        pass "Metainfo has content rating"
    else
        warn "Metainfo missing content_rating (required by Flathub)"
    fi

    if command -v appstreamcli &>/dev/null; then
        if appstreamcli validate "$METAINFO" 2>&1 | grep -qi 'passed'; then
            pass "appstreamcli validate passed"
        else
            ISSUES=$(appstreamcli validate "$METAINFO" 2>&1 | tail -3)
            fail "appstreamcli validate issues: $ISSUES"
        fi
    fi
else
    fail "Metainfo file not found: flatpak/wtf.tonho.omniget.metainfo.xml"
fi

section "Bundled Dependencies"

if [ -f "$MANIFEST" ]; then
    if grep -q 'yt-dlp' "$MANIFEST"; then
        pass "yt-dlp module declared in manifest"
        if grep -q '/app/bin/yt-dlp' "$MANIFEST"; then
            pass "yt-dlp installs to /app/bin/ (sandbox-accessible)"
        else
            warn "yt-dlp install path may not be in /app/bin/"
        fi
    else
        warn "yt-dlp not bundled in manifest"
    fi

    if grep -q 'ffmpeg' "$MANIFEST"; then
        pass "ffmpeg module declared in manifest"
    else
        warn "ffmpeg not bundled in manifest"
    fi

    if [ -f "$ROOT/flatpak/cargo-sources.json" ]; then
        pass "cargo-sources.json exists (offline Rust deps)"
    else
        warn "cargo-sources.json missing (generated at CI build time)"
    fi

    if [ -f "$ROOT/flatpak/node-sources.json" ]; then
        pass "node-sources.json exists (offline Node deps)"
    else
        warn "node-sources.json missing (generated at CI build time)"
    fi
fi

printf "\n\033[1m=== Summary ===\033[0m\n"
printf "  \033[32mPASS: %d\033[0m\n" "$PASS"
printf "  \033[31mFAIL: %d\033[0m\n" "$FAIL"
printf "  \033[33mWARN: %d\033[0m\n" "$WARN"

if [ "$FAIL" -gt 0 ]; then
    printf "\n\033[31mPre-flight FAILED with %d error(s).\033[0m\n" "$FAIL"
    exit 1
else
    printf "\n\033[32mPre-flight PASSED.\033[0m\n"
    exit 0
fi
