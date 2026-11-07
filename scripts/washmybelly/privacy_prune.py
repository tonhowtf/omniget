#!/usr/bin/env python3
"""Drop the privacy.sexy entries OmniGet never offers (they weaken antivirus and exploit
protection, or wipe backups and logs), and the TrustedInstaller and token-privilege helpers of scripts it only
shows and never runs, so those commands are not shipped inside the app. Usage: privacy_prune.py windows.yaml"""
import re
import sys

import yaml

LEFT_OUT = re.compile(
    r"defender|smartscreen|windows security|spectre|meltdown|virtualization-based|\bvbs\b|\bucpd\b"
    r"|smart app control|bitlocker|sehop|control flow guard|exploit guard|\bamsi\b|antimalware",
    re.I,
)
LEFT_OUT_FUNCTIONS = {"SetMpPreference"}
DESTRUCTIVE_CODE = re.compile(r"vssadmin\s+delete|wbadmin\s+delete|wevtutil(\.exe)?\s+cl\b|wevtutil(\.exe)?\s+el\b", re.I)


class Dumper(yaml.SafeDumper):
    def ignore_aliases(self, data):
        return True


def block_text(dumper, text):
    style = "|" if "\n" in text else None
    return dumper.represent_scalar("tag:yaml.org,2002:str", text, style=style)


Dumper.add_representer(str, block_text)


def calls_of(node):
    call = node.get("call")
    if isinstance(call, dict):
        call = [call]
    return [c["function"] for c in call or []]


def banned_functions(functions):
    banned = set(LEFT_OUT_FUNCTIONS)
    while True:
        more = {f["name"] for f in functions if f["name"] not in banned and banned & set(calls_of(f))}
        if not more:
            return banned
        banned |= more


def prune(nodes, banned):
    kept = []
    removed = 0
    for node in nodes:
        name = node.get("category") or node.get("name") or ""
        if LEFT_OUT.search(name) or DESTRUCTIVE_CODE.search(str(node.get("code", ""))):
            removed += count(node)
            continue
        if "children" in node:
            node["children"], n = prune(node["children"], banned)
            removed += n
            if node["children"]:
                kept.append(node)
            continue
        if banned & set(calls_of(node)):
            removed += 1
            continue
        kept.append(node)
    return kept, removed


ELEVATION_BLOCK = re.compile(r"\{\{ with \$(?:elevateToTrustedInstaller|grantPermissions) \}\}")
TEMPLATE_TOKEN = re.compile(r"\{\{ with \$\w+ \}\}|\{\{ end \}\}")
KEPT_BLOCK_BODIES = {"true", "{{ . }}"}


def strip_elevation(text):
    out = []
    pos = 0
    for start in ELEVATION_BLOCK.finditer(text):
        if start.start() < pos:
            continue
        depth = 1
        for token in TEMPLATE_TOKEN.finditer(text, start.end()):
            depth += -1 if token.group() == "{{ end }}" else 1
            if depth == 0:
                break
        body = text[start.end() : token.start()].strip()
        if body in KEPT_BLOCK_BODIES:
            continue
        out.append(text[pos : start.start()])
        pos = token.end()
    out.append(text[pos:])
    return "".join(out)


def strip_tree(node):
    if isinstance(node, dict):
        return {k: strip_tree(v) for k, v in node.items()}
    if isinstance(node, list):
        return [strip_tree(v) for v in node]
    return strip_elevation(node) if isinstance(node, str) else node


def count(node):
    return sum(count(c) for c in node["children"]) if "children" in node else 1


def main(path):
    with open(path, encoding="utf-8") as f:
        data = yaml.safe_load(f)
    banned = banned_functions(data["functions"])
    data["actions"], removed = prune(data["actions"], banned)
    used = set()
    pending = [c for a in data["actions"] for c in walk_calls(a)]
    by_name = {f["name"]: f for f in data["functions"]}
    while pending:
        name = pending.pop()
        if name in used:
            continue
        used.add(name)
        pending.extend(calls_of(by_name[name]))
    data["functions"] = [strip_tree(f) for f in data["functions"] if f["name"] in used]
    with open(path, "w", encoding="utf-8") as f:
        yaml.dump(data, f, Dumper=Dumper, allow_unicode=True, sort_keys=False, width=100000)
    print(f"removed {removed} scripts, kept {len(data['functions'])} of {len(by_name)} functions")


def walk_calls(node):
    if "children" in node:
        for child in node["children"]:
            yield from walk_calls(child)
    else:
        yield from calls_of(node)


if __name__ == "__main__":
    main(sys.argv[1])
