#!/usr/bin/env python3
"""SRT/VTT helpers for the omniget skill.

Subcommands:
  to-md   <file> [--title T] [--source URL] [--backend B] [--model M] [--interval S]
  to-text <file>
  concat  --out merged.srt part.srt:OFFSET_SECONDS ...
  stats   <file>            -> JSON {cues, chars, duration}

Standard library only, Python 3.8+.
"""
import argparse
import html
import json
import re
import sys
from dataclasses import dataclass
from typing import List, Tuple

TS_RE = re.compile(r"(?:(\d+):)?(\d{1,2}):(\d{2})[.,](\d{1,3})")
TAG_RE = re.compile(r"<[^>]+>")
WS_RE = re.compile(r"\s+")


@dataclass
class Cue:
    start: float
    end: float
    text: str


def parse_ts(raw: str) -> float:
    m = TS_RE.search(raw)
    if not m:
        raise ValueError("bad timestamp: %r" % raw)
    hours = int(m.group(1) or 0)
    ms = m.group(4).ljust(3, "0")
    return hours * 3600 + int(m.group(2)) * 60 + int(m.group(3)) + int(ms) / 1000.0


def clean_text(raw: str) -> str:
    text = html.unescape(TAG_RE.sub("", raw))
    return WS_RE.sub(" ", text).strip()


def parse_cues(content: str) -> List[Cue]:
    """Parse SRT or VTT into cues.

    Boundary-based on the `-->` lines rather than blank-line blocks, so output
    that omits blank separators between cues (as some models emit) still parses.
    The lone integer index that precedes the next cue is stripped from a cue's
    trailing text.
    """
    lines = content.replace("\r", "").split("\n")
    ts_lines = [i for i, l in enumerate(lines) if "-->" in l]
    cues: List[Cue] = []
    for k, i in enumerate(ts_lines):
        left, _, right = lines[i].partition("-->")
        try:
            start = parse_ts(left)
            end = parse_ts(right.split()[0] if right.split() else right)
        except ValueError:
            continue
        stop = ts_lines[k + 1] if k + 1 < len(ts_lines) else len(lines)
        text_lines = lines[i + 1:stop]
        while text_lines and not text_lines[-1].strip():
            text_lines.pop()
        if k + 1 < len(ts_lines) and text_lines and text_lines[-1].strip().isdigit():
            text_lines.pop()  # the next cue's index number, not caption text
        cleaned = [clean_text(l) for l in text_lines]
        cleaned = [c for c in cleaned if c]
        if cleaned:
            cues.append(Cue(start, end, "\n".join(cleaned)))
    return cues

def _merge_line(prev: str, line: str) -> Tuple[bool, str]:
    """Collapse rolling captions. Returns (replace_previous, merged_text)."""
    if line == prev:
        return True, prev
    if line.startswith(prev):
        return True, line
    pw = prev.split(" ")
    lw = line.split(" ")
    max_overlap = min(len(pw), len(lw)) - 1
    for n in range(max_overlap, 1, -1):
        if pw[-n:] == lw[:n]:
            return True, " ".join(pw + lw[n:])
    return False, line


def dedupe(cues: List[Cue]) -> List[Tuple[float, str]]:
    """Flatten cues into (start, line) pairs with rolling repeats removed."""
    out: List[Tuple[float, str]] = []
    for cue in cues:
        for line in cue.text.split("\n"):
            if out:
                replace, merged = _merge_line(out[-1][1], line)
                if replace:
                    out[-1] = (out[-1][0], merged)
                    continue
            out.append((cue.start, line))
    return out


def to_text(cues: List[Cue]) -> str:
    return " ".join(line for _, line in dedupe(cues))


def fmt_marker(seconds: float) -> str:
    total = int(seconds)
    h, rem = divmod(total, 3600)
    m, s = divmod(rem, 60)
    if h:
        return "%d:%02d:%02d" % (h, m, s)
    return "%02d:%02d" % (m, s)


def fmt_srt_ts(seconds: float) -> str:
    total_ms = int(round(seconds * 1000))
    h, rem = divmod(total_ms, 3600000)
    m, rem = divmod(rem, 60000)
    s, ms = divmod(rem, 1000)
    return "%02d:%02d:%02d,%03d" % (h, m, s, ms)


def to_markdown(cues: List[Cue], interval: float = 60.0) -> str:
    paragraphs: List[str] = []
    current: List[str] = []
    para_start = None
    for start, line in dedupe(cues):
        if para_start is None:
            para_start = start
        elif start - para_start >= interval:
            paragraphs.append("[%s] %s" % (fmt_marker(para_start), " ".join(current)))
            current = []
            para_start = start
        current.append(line)
    if current and para_start is not None:
        paragraphs.append("[%s] %s" % (fmt_marker(para_start), " ".join(current)))
    return "\n\n".join(paragraphs)


def offset_cues(cues: List[Cue], seconds: float) -> List[Cue]:
    return [Cue(c.start + seconds, c.end + seconds, c.text) for c in cues]


def render_srt(cues: List[Cue]) -> str:
    parts = []
    for i, c in enumerate(cues, 1):
        parts.append("%d\n%s --> %s\n%s\n" % (i, fmt_srt_ts(c.start), fmt_srt_ts(c.end), c.text))
    return "\n".join(parts)


def read(path: str) -> str:
    with open(path, encoding="utf-8", errors="replace") as fh:
        return fh.read()


def cmd_to_md(args) -> int:
    cues = parse_cues(read(args.file))
    body = to_markdown(cues, args.interval)
    header = []
    if args.title:
        header.append("# %s" % args.title)
        header.append("")
    meta = []
    if args.source:
        meta.append("- Source: %s" % args.source)
    if args.backend:
        engine = args.backend + (" (%s)" % args.model if args.model else "")
        meta.append("- Transcribed with: %s" % engine)
    if cues:
        meta.append("- Duration: %s" % fmt_marker(cues[-1].end))
    if meta:
        header.extend(meta)
        header.append("")
    sys.stdout.write("\n".join(header) + body + "\n")
    return 0


def cmd_to_text(args) -> int:
    sys.stdout.write(to_text(parse_cues(read(args.file))) + "\n")
    return 0


def cmd_concat(args) -> int:
    merged: List[Cue] = []
    for spec in args.parts:
        path, _, offset = spec.rpartition(":")
        if not path:
            path, offset = spec, "0"
        merged.extend(offset_cues(parse_cues(read(path)), float(offset)))
    merged.sort(key=lambda c: c.start)
    with open(args.out, "w", encoding="utf-8") as fh:
        fh.write(render_srt(merged))
    return 0


def cmd_stats(args) -> int:
    cues = parse_cues(read(args.file))
    text = to_text(cues)
    print(json.dumps({
        "cues": len(cues),
        "chars": len(text),
        "duration": cues[-1].end if cues else 0.0,
    }))
    return 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("to-md")
    p.add_argument("file")
    p.add_argument("--title")
    p.add_argument("--source")
    p.add_argument("--backend")
    p.add_argument("--model")
    p.add_argument("--interval", type=float, default=60.0)
    p.set_defaults(func=cmd_to_md)

    p = sub.add_parser("to-text")
    p.add_argument("file")
    p.set_defaults(func=cmd_to_text)

    p = sub.add_parser("concat")
    p.add_argument("--out", required=True)
    p.add_argument("parts", nargs="+", help="path.srt:offset_seconds")
    p.set_defaults(func=cmd_concat)

    p = sub.add_parser("stats")
    p.add_argument("file")
    p.set_defaults(func=cmd_stats)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
