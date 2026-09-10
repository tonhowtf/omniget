#!/usr/bin/env python3
"""Transcribe an audio file with Gemini and write SubRip (SRT) output.

Usage: gemini_transcribe.py AUDIO --out OUT.srt [--model gemini-2.5-flash] [--lang xx]

Reads GEMINI_API_KEY from the environment. Standard library only: files up to
19 MB are sent inline, larger ones go through the Gemini Files API. If the
model does not return parseable SRT, the raw text is written to OUT with a
.txt extension and that path is printed instead.
"""
import argparse
import base64
import json
import mimetypes
import os
import re
import sys
import time
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import srt_tools  # noqa: E402

API = "https://generativelanguage.googleapis.com"
INLINE_LIMIT = 19 * 1024 * 1024

PROMPT = (
    "Transcribe this audio verbatim into SubRip (SRT) subtitle format. "
    "Number every cue, use `HH:MM:SS,mmm --> HH:MM:SS,mmm` timestamps that match "
    "when the words are spoken, keep each cue under about 10 seconds, and write "
    "exactly what is said with no commentary, no speaker guesses and no markdown fences. {lang}"
)


def normalize_srt_timestamps(text):
    """Rewrite the timestamp lines Gemini emits into strict SRT `HH:MM:SS,mmm`.

    Gemini is inconsistent: it produces `MM:SS:mmm`, `HH:MM:SS.mmm`, bare
    `HH:MM:SS`, and the correct comma form interchangeably. Normalize every
    `-->` line so srt_tools.parse_cues can read it.
    """
    def one(token):
        parts = [p for p in re.split(r"[:.,]", token.strip()) if p != ""]
        if not parts:
            return None
        ms = 0
        if len(parts) > 1 and len(parts[-1]) == 3 and parts[-1].isdigit():
            ms = int(parts[-1])
            parts = parts[:-1]
        try:
            nums = [int(p) for p in parts]
        except ValueError:
            return None
        s = nums[-1] if len(nums) >= 1 else 0
        m = nums[-2] if len(nums) >= 2 else 0
        h = nums[-3] if len(nums) >= 3 else 0
        return "%02d:%02d:%02d,%03d" % (h, m, s, ms)

    out = []
    for line in text.splitlines():
        if "-->" in line:
            m = re.match(r"\s*([0-9:.,]+)\s*-->\s*([0-9:.,]+)(.*)$", line)
            if m:
                a, b = one(m.group(1)), one(m.group(2))
                if a and b:
                    line = "%s --> %s" % (a, b)
        out.append(line)
    return "\n".join(out)



def request(method, url, body=None, headers=None, raw=False, timeout=900):
    data = body if raw else (json.dumps(body).encode() if body is not None else None)
    hdrs = {"Content-Type": "application/json"}
    hdrs.update(headers or {})
    req = urllib.request.Request(url, data=data, method=method, headers=hdrs)
    for attempt in range(4):
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return resp.headers, resp.read()
        except urllib.error.HTTPError as err:
            detail = err.read().decode(errors="replace")[:500]
            if err.code in (429, 500, 502, 503, 504) and attempt < 3:
                time.sleep(5 * (attempt + 1))
                continue
            sys.exit("gemini: HTTP %s: %s" % (err.code, detail))
        except urllib.error.URLError as err:
            if attempt < 3:
                time.sleep(5 * (attempt + 1))
                continue
            sys.exit("gemini: %s" % err)


def upload_file(path, mime, key):
    size = os.path.getsize(path)
    headers, _ = request(
        "POST",
        "%s/upload/v1beta/files?key=%s" % (API, key),
        {"file": {"display_name": os.path.basename(path)}},
        {
            "X-Goog-Upload-Protocol": "resumable",
            "X-Goog-Upload-Command": "start",
            "X-Goog-Upload-Header-Content-Length": str(size),
            "X-Goog-Upload-Header-Content-Type": mime,
        },
    )
    upload_url = headers.get("X-Goog-Upload-URL")
    if not upload_url:
        sys.exit("gemini: upload session did not return an upload URL")
    with open(path, "rb") as fh:
        payload = fh.read()
    _, body = request(
        "POST",
        upload_url,
        payload,
        {
            "Content-Type": mime,
            "X-Goog-Upload-Offset": "0",
            "X-Goog-Upload-Command": "upload, finalize",
        },
        raw=True,
    )
    info = json.loads(body)["file"]
    name, uri = info["name"], info["uri"]
    while info.get("state") == "PROCESSING":
        time.sleep(3)
        _, body = request("GET", "%s/v1beta/%s?key=%s" % (API, name, key))
        info = json.loads(body)
    if info.get("state") == "FAILED":
        sys.exit("gemini: file processing failed")
    return uri


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("audio")
    ap.add_argument("--out", required=True)
    ap.add_argument("--model", default="gemini-2.5-flash")
    ap.add_argument("--lang", default="")
    args = ap.parse_args()

    key = os.environ.get("GEMINI_API_KEY") or os.environ.get("GOOGLE_API_KEY")
    if not key:
        sys.exit("gemini: GEMINI_API_KEY is not set")

    mime = mimetypes.guess_type(args.audio)[0] or "audio/mpeg"
    if os.path.getsize(args.audio) <= INLINE_LIMIT:
        with open(args.audio, "rb") as fh:
            media = {"inline_data": {"mime_type": mime, "data": base64.b64encode(fh.read()).decode()}}
    else:
        media = {"file_data": {"mime_type": mime, "file_uri": upload_file(args.audio, mime, key)}}

    lang_hint = ("The audio is in %s." % args.lang) if args.lang else "Keep the spoken language."
    body = {
        "contents": [{"parts": [{"text": PROMPT.format(lang=lang_hint)}, media]}],
        "generationConfig": {"temperature": 0, "maxOutputTokens": 65536},
    }
    _, raw = request("POST", "%s/v1beta/models/%s:generateContent?key=%s" % (API, args.model, key), body)
    reply = json.loads(raw)
    candidates = reply.get("candidates") or []
    if not candidates:
        sys.exit("gemini: empty response: %s" % json.dumps(reply)[:300])
    text = "".join(p.get("text", "") for p in candidates[0].get("content", {}).get("parts", []))
    text = text.strip()
    if text.startswith("```"):
        text = text.strip("`")
        text = text.split("\n", 1)[1] if "\n" in text else text

    cues = srt_tools.parse_cues(normalize_srt_timestamps(text))
    if cues:
        out = args.out
        with open(out, "w", encoding="utf-8") as fh:
            fh.write(srt_tools.render_srt(cues))
    else:
        out = os.path.splitext(args.out)[0] + ".txt"
        with open(out, "w", encoding="utf-8") as fh:
            fh.write(text + "\n")
    print(out)


if __name__ == "__main__":
    main()
