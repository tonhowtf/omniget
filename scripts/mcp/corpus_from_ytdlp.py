#!/usr/bin/env python3
"""Build a download benchmark corpus from yt-dlp's own extractor test cases.

Every yt-dlp extractor ships `_TESTS`: real URLs with the metadata upstream
expects (id, ext, title, duration). This samples downloadable ones per site,
deterministically, into the cases format scripts/mcp/desktop_benchmark.py reads.

  python3 scripts/mcp/corpus_from_ytdlp.py --ytdlp /path/to/yt-dlp \
      --sites youtube,instagram,tiktok,twitter,reddit,vimeo --per-site 5 --out cases.json

--ytdlp is the zipapp/binary (Python zipapp) or a source checkout; it should be
the same pinned version the app uses, so results are comparable run to run.
Upstream test cases age: a URL that is gone is an external failure, not an
OmniGet defect. Local testing only; no redistribution of the media.
"""
import argparse
import hashlib
import json
import os
import random
import shutil
import sys
import urllib.request
import zipfile
from collections import defaultdict

PLAYLIST_KEYS = ("playlist", "playlist_count", "playlist_mincount", "playlist_maxcount")


def zipapp_for(path, tag):
    """A PyInstaller binary cannot be imported: fetch the official zipapp of
    the same release tag (from provenance.json next to it, or --tag) and check
    it against the release's SHA2-256SUMS."""
    if os.path.isdir(path) or zipfile.is_zipfile(path):
        return path
    if not tag:
        prov = os.path.join(os.path.dirname(path), "provenance.json")
        if os.path.exists(prov):
            tag = json.load(open(prov)).get("tag")
    if not tag:
        sys.exit(f"{path} is not a zipapp/source dir; pass --tag <release> to fetch the matching zipapp")
    cache = os.path.join(os.path.expanduser("~"), ".cache", "omniget")
    os.makedirs(cache, exist_ok=True)
    target = os.path.join(cache, f"yt-dlp-{tag}.zip")
    base = f"https://github.com/yt-dlp/yt-dlp/releases/download/{tag}/"
    sums = urllib.request.urlopen(base + "SHA2-256SUMS", timeout=60).read().decode()
    want = next((l.split()[0] for l in sums.splitlines() if l.split()[-1:] == ["yt-dlp"]), None)
    if not want:
        sys.exit(f"no yt-dlp entry in SHA2-256SUMS of {tag}")
    if not (os.path.exists(target) and hashlib.sha256(open(target, "rb").read()).hexdigest() == want):
        data = urllib.request.urlopen(base + "yt-dlp", timeout=120).read()
        if hashlib.sha256(data).hexdigest() != want:
            sys.exit(f"sha256 mismatch for yt-dlp {tag} zipapp")
        with open(target, "wb") as f:
            f.write(data)
    return target


def load_ytdlp(path):
    sys.path.insert(0, path)
    import yt_dlp  # noqa: E402
    from yt_dlp.extractor import gen_extractor_classes  # noqa: E402

    return yt_dlp.version.__version__, gen_extractor_classes()


def site_of(ie, sites):
    # IE_NAME is "youtube", "youtube:tab", "twitter:broadcast"...: the part
    # before ":" must be the site (the class name would also match unrelated
    # extractors such as YoutubeWebArchiveIE).
    base = (getattr(ie, "IE_NAME", "") or "").lower().split(":")[0]
    return base if base in sites else None


def usable(t, allow_playlists, max_duration=0):
    if not isinstance(t, dict) or not str(t.get("url", "")).startswith(("http://", "https://")) or t.get("only_matching") or t.get("skip"):
        return False
    if not allow_playlists and any(k in t for k in PLAYLIST_KEYS):
        return False
    info = t.get("info_dict") or {}
    # Live streams never finish; they measure nothing a download benchmark can check.
    if info.get("is_live") or info.get("live_status") in ("is_live", "is_upcoming"):
        return False
    # Hours-long VODs hit the worker's traffic budget and the per-case timeout:
    # they measure the budget, not the download path.
    if max_duration and isinstance(info.get("duration"), (int, float)) and info["duration"] > max_duration:
        return False
    return bool(info.get("id")) and info.get("_type") not in ("playlist", "multi_video")


def ensure_python():
    """yt-dlp needs Python 3.10+; macOS's /usr/bin/python3 is 3.9."""
    if sys.version_info >= (3, 10):
        return
    for name in ("python3.14", "python3.13", "python3.12", "python3.11", "python3.10"):
        exe = shutil.which(name)
        if exe:
            os.execv(exe, [exe, os.path.abspath(__file__), *sys.argv[1:]])
    sys.exit("yt-dlp needs Python 3.10 or newer")


def main():
    ensure_python()
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--ytdlp", default=shutil.which("yt-dlp"), help="yt-dlp zipapp or source dir (default: yt-dlp on PATH)")
    p.add_argument("--tag", help="yt-dlp release tag when --ytdlp is a PyInstaller binary")
    p.add_argument("--sites", default="youtube,instagram,tiktok,twitter,reddit,vimeo,twitch,bilibili,pinterest,soundcloud,dailymotion,bluesky")
    p.add_argument("--per-site", type=int, default=5)
    p.add_argument("--seed", type=int, default=20260925, help="same seed, same corpus")
    p.add_argument("--max-height", type=int, default=720)
    p.add_argument("--allow-playlists", action="store_true")
    p.add_argument("--max-duration", type=int, default=900, help="skip cases whose expected duration exceeds this many seconds (0 = no limit)")
    p.add_argument("--out", required=True)
    a = p.parse_args()
    if not a.ytdlp:
        sys.exit("yt-dlp not found: pass --ytdlp")
    version, classes = load_ytdlp(zipapp_for(a.ytdlp, a.tag))
    sites = [s.strip().lower() for s in a.sites.split(",") if s.strip()]
    pool = defaultdict(list)
    for ie in classes:
        site = site_of(ie, sites)
        if not site:
            continue
        try:
            tests = list(ie.get_testcases(include_onlymatching=False))
        except Exception:
            continue
        for t in tests:
            if "live" not in ie.IE_NAME.lower() and usable(t, a.allow_playlists, a.max_duration):
                pool[site].append((ie.IE_NAME, t))
    rng = random.Random(a.seed)
    cases, summary = [], {}
    for site in sites:
        found = sorted(pool.get(site, []), key=lambda x: (x[0], x[1]["url"]))
        picked = rng.sample(found, min(a.per_site, len(found)))
        summary[site] = f"{len(picked)}/{len(found)}"
        for n, (ie_name, t) in enumerate(picked, 1):
            info = t.get("info_dict") or {}
            cases.append({
                "caseId": f"{site}-{n}",
                "platform": site,
                "url": t["url"],
                "source": f"yt-dlp {version} {ie_name}._TESTS",
                "authorization": "Public upstream regression fixture; local test only; no redistribution; media license unverified",
                "expected": {k: info[k] for k in ("id", "ext", "duration") if k in info},
                "maxHeight": a.max_height,
            })
    with open(a.out, "w") as f:
        json.dump(cases, f, indent=2)
    print(f"yt-dlp {version}: {len(cases)} cases -> {a.out}")
    for site, s in summary.items():
        print(f"  {site:12} {s}")


if __name__ == "__main__":
    main()
