#!/usr/bin/env python3
"""Copy BleachBit CleanerML files into the washmybelly data folder.

Usage: python3 scripts/washmybelly/sync_bleachbit.py <path to a bleachbit clone>

- Keeps only `cleaners/*.xml` with at least one action for macOS, Linux or
  Windows (the three systems ship in every binary and are filtered at run time).
- The XML is copied byte for byte (GPL-3.0-or-later, Andrew Ziem): the Rust
  parser in `clean/cleanerml.rs` reads the originals.
- Writes `COPYING` and `SOURCE.md` next to them, and regenerates the
  `include_str!` table between the GENERATED markers in `cleanerml.rs`.

Nothing here runs at app run time; rerun it when BleachBit is updated and let
the golden test (`cleanerml::tests::golden_*`) show what changed.
"""

import datetime
import os
import re
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
DEST = os.path.join(ROOT, "src-tauri", "omniget-core", "data", "washmybelly", "bleachbit")
RUST = os.path.join(ROOT, "src-tauri", "omniget-core", "src", "core", "washmybelly", "clean", "cleanerml.rs")

PLATFORMS = {
    "macos": ("macos", "bsd", "unix"),
    "linux": ("linux", "unix"),
    "windows": ("windows",),
}

def os_match(os_str, platform):
    """Same rule as bleachbit/General.py os_match."""
    if not os_str:
        return True
    tokens = [t.strip() for t in os_str.split(",") if t.strip()]
    tokens = ["macos" if t == "darwin" else ("!macos" if t == "!darwin" else t) for t in tokens]
    neg = [t[1:] for t in tokens if t.startswith("!")]
    pos = [t for t in tokens if not t.startswith("!")]
    cur = PLATFORMS[platform]
    if any(t in cur for t in neg):
        return False
    if pos:
        return any(t in cur for t in pos)
    return True

def useful(path):
    tree = ET.parse(path)
    root = tree.getroot()
    if root.tag != "cleaner":
        return False
    for platform in PLATFORMS:
        if not os_match(root.get("os"), platform):
            continue
        for opt in root.findall("option"):
            if not os_match(opt.get("os"), platform):
                continue
            for act in opt.findall("action"):
                if os_match(act.get("os"), platform):
                    return True
    return False

def main():
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    src = os.path.abspath(sys.argv[1])
    cleaners = os.path.join(src, "cleaners")
    commit = subprocess.check_output(["git", "-C", src, "rev-parse", "HEAD"], text=True).strip()
    date = subprocess.check_output(["git", "-C", src, "log", "-1", "--format=%cs"], text=True).strip()

    out = os.path.join(DEST, "cleaners")
    if os.path.isdir(out):
        shutil.rmtree(out)
    os.makedirs(out)
    kept, dropped = [], []
    for name in sorted(os.listdir(cleaners)):
        if not name.endswith(".xml"):
            continue
        path = os.path.join(cleaners, name)
        with open(path, "rb") as f:
            if b"<!DOCTYPE" in f.read():
                dropped.append((name, "has a DTD"))
                continue
        if useful(path):
            shutil.copyfile(path, os.path.join(out, name))
            kept.append(name)
        else:
            dropped.append((name, "no action for macOS, Linux or Windows"))
    shutil.copyfile(os.path.join(src, "COPYING"), os.path.join(DEST, "COPYING"))

    with open(os.path.join(DEST, "SOURCE.md"), "w") as f:
        f.write("# BleachBit CleanerML\n\n")
        f.write("- Repository: https://github.com/bleachbit/bleachbit\n")
        f.write(f"- Commit: {commit} ({date})\n")
        f.write(f"- Copied: {datetime.date.today().isoformat()} by `scripts/washmybelly/sync_bleachbit.py`\n")
        f.write("- License: GPL-3.0-or-later, Copyright (c) 2008-2026 Andrew Ziem (see `COPYING`).\n")
        f.write(f"- Files: {len(kept)} of `cleaners/*.xml`, unmodified.\n")
        if dropped:
            f.write("- Left out:\n")
            for n, why in dropped:
                f.write(f"  - `{n}`: {why}\n")

    with open(RUST) as f:
        text = f.read()
    lines = ["// BEGIN GENERATED CLEANERS (scripts/washmybelly/sync_bleachbit.py)", "pub const EMBEDDED: &[(&str, &str)] = &["]
    for n in kept:
        stem = n[:-4]
        lines.append(f'    ("{stem}", include_str!("../../../../data/washmybelly/bleachbit/cleaners/{n}")),')
    lines.append("];")
    lines.append("// END GENERATED CLEANERS")
    block = "\n".join(lines)
    new, n = re.subn(r"// BEGIN GENERATED CLEANERS.*?// END GENERATED CLEANERS", lambda _m: block, text, flags=re.S)
    if n != 1:
        sys.exit("markers not found in cleanerml.rs")
    with open(RUST, "w") as f:
        f.write(new)
    print(f"kept {len(kept)}, dropped {len(dropped)}")

if __name__ == "__main__":
    main()
