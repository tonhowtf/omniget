#!/usr/bin/env python3
"""Build `data/washmybelly/mole/protection.txt` from a Mole clone.

Usage: python3 scripts/washmybelly/sync_mole_protection.py <path to a Mole clone>

Mole (GPL-3.0, https://github.com/tw93/Mole) keeps its "looks like cache but
is not" list in `lib/core/app_protection_data.sh` (data only by contract).
This script copies every pattern of the arrays below, one per line, and
appends the path patterns that `lib/core/app_protection.sh`
(`should_protect_path`, `should_protect_data`) and `lib/core/base.sh` (default
and safety whitelists) hard-code, ported by hand.

Line format read by `clean/protection.rs`:

    name <glob>    the last path component (bundle id or folder name)
    path <glob>    the whole path; `*` crosses `/`; a leading `~` is the home
    ipath <glob>   same, ignoring case

The name "Mole" never reaches the UI; attribution lives in SOURCE.md.
"""

import datetime
import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
DEST = os.path.join(ROOT, "src-tauri", "omniget-core", "data", "washmybelly", "mole")

ARRAYS = ["SYSTEM_CRITICAL_BUNDLES", "DATA_PROTECTED_BUNDLES"]

# should_protect_data(): patterns matched against the last component.
NAME_EXTRA = """
com.apple.*
loginwindow
dock
systempreferences
finder
safari
org.cups.*
backgroundtaskmanagement*
keychain*
security*
bluetooth*
wifi*
network*
tcc
notification*
accessibility*
universalaccess*
HIToolbox*
*inputmethod*
*InputMethod*
*IME
textinput*
TextInput*
keyboard*
Keyboard*
inputsource*
InputSource*
keylayout*
KeyLayout*
GlobalPreferences
.GlobalPreferences
org.pqrs.Karabiner*
com.microsoft.*
Cursor
Claude
ChatGPT
com.openai.codex
Codex
codex-runtimes
Ollama
com.nssurge.*
com.v2ray.*
ClashX*
Surge*
Shadowrocket*
Quantumult*
com.docker.*
com.getpostman.*
com.insomnia.*
"""

# should_protect_path() steps 1, 2, 4, 5 and the base.sh whitelists.
PATH_EXTRA = """
ipath *systemsettings*
ipath *systempreferences*
ipath *controlcenter*
ipath *com.apple.settings*
ipath *com.apple.notes*
path *com.apple.finder*
path *com.apple.dock*
path */Library/Group Containers/*dev.orbstack
path */Library/Group Containers/*dev.orbstack/*
path */.orbstack
path */.orbstack/*
path */com.apple.sharedfilelist/*
path */Library/Preferences/com.apple.dock.plist
path */Library/Preferences/com.apple.finder.plist
path */Library/Application Support/Codex
path */Library/Application Support/Codex/*
path */Library/Logs/com.openai.codex
path */Library/Logs/com.openai.codex/*
path */.codex/sessions
path */.codex/sessions/*
path */.codex/auth.json
path */.codex/history.jsonl
path */.codex/state_*.sqlite
path */.codex/logs_*.sqlite
path */.codex/session_index.jsonl
path */.codex/cache/session_index.jsonl
path */.codex/cache/codex_app_directory
path */.codex/cache/codex_app_directory/*
path */ByHost/com.apple.bluetooth.*
path */ByHost/com.apple.wifi.*
path */Library/Preferences/com.apple.networkextension*.plist
path */Mobile Documents*
path */Library/Accounts
path */Library/Accounts/*
path */Library/Keychains
path */Library/Keychains/*
path */Library/Mail
path */Library/Mail/*
path */Library/Calendars
path */Library/Calendars/*
path */Library/Contacts
path */Library/Contacts/*
path /Library/Audio/Plug-Ins/*
path */Library/Application Support/iZotope
path */Library/Application Support/iZotope/*
path /Library/Application Support/LaserSoft Imaging*
path */Library/Preferences/com.native-instruments*
path */Library/Preferences/com.avid.mediacomposer*.plist
path */Library/Preferences/com.paceap.*.plist
path /private/var/folders/*/C/com.native-instruments*
path /private/var/folders/*/C/com.avid.mediacomposer*
path /private/var/folders/*/C/com.paceap.eden.iLokLicenseManager*
path */Library/Caches/ms-playwright*
path */Library/Caches/app.cotypist.Cotypist*
path */Library/Caches/com.displaylink.DisplayLinkUserAgent*
path */Library/Caches/com.lasersoft-imaging.SilverFast*
path */Library/Caches/Adobe *
path */Library/Caches/* Adobe*
path */Library/Caches/com.apple.containermanagerd*
path */Library/Caches/com.apple.homed*
path */Library/Caches/com.apple.ap.adprivacyd*
path */Library/Caches/FamilyCircle*
path */Library/Caches/com.apple.HomeKit*
path */Library/Caches/com.apple.WorkflowKit.BackgroundShortcutRunner.ShortcutsSandboxCache*
path */Library/Caches/com.apple.siriactionsd.ShortcutsSandboxCache*
path */Library/Caches/GIPPseudonymousID*
path */Library/Caches/CCTClearcutLogger*
path */Library/Application Support/com.apple.idleassetsd*
path */Library/Application Support/com.apple.wallpaper*
path *com.apple.coreaudio*
path *com.apple.audio.*
path *coreaudiod*
path */com.apple.e5rt.e5bundlecache
path */com.apple.e5rt.e5bundlecache/*
path ~/.local/bin
path ~/.local/lib
path ~/.local/state
path */.gradle/daemon/*
path */.ollama/models*
path */Library/Caches/com.nssurge.surge-mac*
path */Library/Application Support/com.nssurge.surge-mac*
path */Library/Caches/org.R-project.R/R/renv*
path */Library/Caches/JetBrains*
path */Library/Caches/com.jetbrains.toolbox*
path */Library/Caches/tealdeer/tldr-pages*
path */Library/Application Support/JetBrains*
path */Library/Caches/com.apple.FontRegistry*
path */Library/Caches/com.apple.spotlight*
path */Library/Caches/com.apple.Spotlight*
path */Library/Caches/CloudKit*
path */Library/Caches/pypoetry/virtualenvs*
path */Library/Autosave Information*
path */Library/Caches/Calendar Cache*
"""


def array(text, name):
    m = re.search(r"readonly " + name + r"=\((.*?)\n\)", text, re.S)
    if not m:
        sys.exit(f"array {name} not found")
    out = []
    for line in m.group(1).splitlines():
        line = line.split("#", 1)[0].strip()
        for q in re.findall(r'"([^"]+)"', line):
            out.append(q)
    return out


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    src = os.path.abspath(sys.argv[1])
    commit = subprocess.check_output(["git", "-C", src, "rev-parse", "HEAD"], text=True).strip()
    date = subprocess.check_output(["git", "-C", src, "log", "-1", "--format=%cs"], text=True).strip()
    with open(os.path.join(src, "lib", "core", "app_protection_data.sh")) as f:
        data = f.read()

    lines = [
        "# Paths that look like cache but hold data, licences or live state.",
        "# Generated by scripts/washmybelly/sync_mole_protection.py; do not edit.",
        "# Source: Mole (GPL-3.0) lib/core/app_protection_data.sh, app_protection.sh, base.sh.",
        f"# Commit {commit} ({date}).",
    ]
    seen = set()

    def add(line):
        if line not in seen:
            seen.add(line)
            lines.append(line)

    for name in ARRAYS:
        lines.append(f"# {name}")
        for p in array(data, name):
            add(f"name {p}")
    lines.append("# ENDPOINT_SECURITY_BUNDLE_PREFIXES (per-user Darwin folders)")
    for p in array(data, "ENDPOINT_SECURITY_BUNDLE_PREFIXES"):
        add(f"ipath /private/var/folders/*{p}*")
        add(f"ipath /var/folders/*{p}*")
    lines.append("# should_protect_data")
    for p in NAME_EXTRA.split():
        add(f"name {p}")
    lines.append("# should_protect_path and whitelists")
    for p in PATH_EXTRA.strip().splitlines():
        add(p.strip())

    with open(os.path.join(DEST, "protection.txt"), "w") as f:
        f.write("\n".join(lines) + "\n")
    shutil.copyfile(os.path.join(src, "LICENSE"), os.path.join(DEST, "LICENSE"))
    with open(os.path.join(DEST, "SOURCE.md"), "w") as f:
        f.write("# Mole\n\n")
        f.write("- Repository: https://github.com/tw93/Mole\n")
        f.write(f"- Commit: {commit} ({date})\n")
        f.write(f"- Copied: {datetime.date.today().isoformat()} by `scripts/washmybelly/sync_mole_protection.py`\n")
        f.write("- License: GPL-3.0 (see `LICENSE`). The name and logo are trademarks of the Mole project\n")
        f.write("  (`TRADEMARK.md`): they never appear in OmniGet's interface.\n")
        f.write("- `protection.txt`: every pattern of `SYSTEM_CRITICAL_BUNDLES`, `DATA_PROTECTED_BUNDLES` and\n")
        f.write("  `ENDPOINT_SECURITY_BUNDLE_PREFIXES`, plus the path patterns hard-coded in `should_protect_path`,\n")
        f.write("  `should_protect_data` and the default and safety whitelists of `lib/core/base.sh`.\n")
        f.write("- The macOS cleanup targets in `clean/rules_macos.rs` marked `source: \"Mole\"` were ported by hand\n")
        f.write("  from `lib/clean/{user,dev,app_caches,brew,system}.sh` of the same commit.\n")
    print(f"{len(seen)} patterns")


if __name__ == "__main__":
    main()
