#!/usr/bin/env python3
"""BN13 — companies of agents, OmniGet against the Paperclip baseline.

Runs the Rust benches that drive the real code:
  - core (`company::bench`): model, checkout race, API latency in process,
    search at 10k issues, heartbeat coalescing and throughput, the budget
    ledger and hard stop, the audit chain, routines, and the Paperclip
    scenario of a `process` agent running /usr/bin/true;
  - HTTP (`local_bridge_company::bench`): the same API calls Paperclip was
    measured with, through the real bridge router on loopback;
then compares each number with the Paperclip baseline measured in study 92
(estudos/92-paperclip-completo/bench/summary.json, median of its 3 runs) and
checks the correctness gates.

  python3 scripts/llm/company_eval.py            # core in release, HTTP in debug
  python3 scripts/llm/company_eval.py --debug    # everything in debug (faster build)

Writes estudos/bench/<timestamp>-bn13/{metrics.jsonl,summary.json} and prints
one line per comparison. Exit 1 if a gate fails.
"""
import argparse
import json
import os
import pathlib
import statistics
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
TAURI = ROOT / "src-tauri"
BASELINE = ROOT / "estudos" / "92-paperclip-completo" / "bench" / "summary.json"

def cargo(args, out, release):
    env = dict(os.environ, OMNIGET_BENCH_OUT=str(out))
    cmd = ["cargo", "test"] + (["--release"] if release else []) + args
    print("$", " ".join(cmd), flush=True)
    r = subprocess.run(cmd, cwd=TAURI, env=env, capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stdout[-4000:], r.stderr[-4000:])
        return False
    return True

def load_metrics(path):
    m = {}
    for line in path.read_text().splitlines():
        try:
            v = json.loads(line)
        except ValueError:
            continue
        m[f"{v['bench']}.{v['metric']}"] = v["value"]
    return m

def baseline():
    if not BASELINE.exists():
        return {}
    runs = json.loads(BASELINE.read_text())["runs"].values()

    def med(path):
        vals = []
        for r in runs:
            cur = r
            for k in path:
                cur = cur.get(k) if isinstance(cur, dict) else None
            if isinstance(cur, (int, float)):
                vals.append(cur)
        return statistics.median(vals) if vals else None

    def checkout(stat):
        vals = []
        for r in runs:
            w = (r.get("checkout50") or {}).get("wallMsPerRound") or []
            if w:
                vals.append(statistics.median(w) if stat == "p50" else max(w))
        return statistics.median(vals) if vals else None

    return {
        "startup_s": med(["startupS"]),
        "idle_rss_mb": med(["idleRss", "totalMB"]),
        "rss_1000_issues_mb": med(["rssAfter1000Issues", "totalMB"]),
        "create_company_ms": med(["createCompanyMs"]),
        "create_agent_p50_ms": med(["createAgent", "p50"]),
        "create_p50_ms": med(["createIssue", "p50"]),
        "create_p99_ms": med(["createIssue", "p99"]),
        "create_1000_total_s": med(["createIssue", "total1000S"]),
        "list500_p50_ms": med(["listIssues500", "p50"]),
        "list500_p99_ms": med(["listIssues500", "p99"]),
        "get_p50_ms": med(["getIssue", "p50"]),
        "get_p99_ms": med(["getIssue", "p99"]),
        "comment_p50_ms": med(["comment", "p50"]),
        "comment_p99_ms": med(["comment", "p99"]),
        "status_p50_ms": med(["changeStatus", "p50"]),
        "status_p99_ms": med(["changeStatus", "p99"]),
        "checkout50_p50_ms": checkout("p50"),
        "checkout50_max_ms": checkout("max"),
        "wake_to_start_p50_ms": med(["wakeToStartMs", "p50"]),
        "wake_to_start_max_ms": med(["wakeToStartMs", "max"]),
        "wake_to_finish_p50_ms": med(["wakeToFinishMs", "p50"]),
        "wake_to_finish_max_ms": med(["wakeToFinishMs", "max"]),
        "burst50_runs_per_s": med(["wakeBurst50", "runsCompletedPerSec"]),
    }

COMPARE = [
    ("create company (ms)", "bn13-http.create_company_ms", "create_company_ms", True),
    ("hire agent p50 (ms)", "bn13-http.create_agent_p50_ms", "create_agent_p50_ms", True),
    ("create issue p50 (ms)", "bn13-http.create_p50_ms", "create_p50_ms", True),
    ("create issue p99 (ms)", "bn13-http.create_p99_ms", "create_p99_ms", True),
    ("1000 issues total (s)", "bn13-http.create_1000_total_s", "create_1000_total_s", True),
    ("list 500 p50 (ms)", "bn13-http.list500_p50_ms", "list500_p50_ms", True),
    ("list 500 p99 (ms)", "bn13-http.list500_p99_ms", "list500_p99_ms", True),
    ("get issue p50 (ms)", "bn13-http.get_p50_ms", "get_p50_ms", True),
    ("get issue p99 (ms)", "bn13-http.get_p99_ms", "get_p99_ms", True),
    ("comment p50 (ms)", "bn13-http.comment_p50_ms", "comment_p50_ms", True),
    ("comment p99 (ms)", "bn13-http.comment_p99_ms", "comment_p99_ms", True),
    ("change status p50 (ms)", "bn13-http.status_p50_ms", "status_p50_ms", True),
    ("change status p99 (ms)", "bn13-http.status_p99_ms", "status_p99_ms", True),
    ("50-way checkout, resolve p50 (ms)", "bn13-http.checkout50_resolve_p50_ms", "checkout50_p50_ms", True),
    ("50-way checkout, resolve max (ms)", "bn13-http.checkout50_resolve_max_ms", "checkout50_max_ms", True),
    ("wake → process start p50 (ms)", "bn13.proc_wake_to_start_p50_ms", "wake_to_start_p50_ms", True),
    ("wake → process start max (ms)", "bn13.proc_wake_to_start_max_ms", "wake_to_start_max_ms", True),
    ("wake → run finished p50 (ms)", "bn13.proc_wake_to_end_p50_ms", "wake_to_finish_p50_ms", True),
    ("wake → run finished max (ms)", "bn13.proc_wake_to_end_max_ms", "wake_to_finish_max_ms", True),
    ("burst of 50 wakes, runs/s", "bn13.proc_burst50_runs_per_s", "burst50_runs_per_s", False),
    ("memory after 1000 issues (MB, whole process)", "bn13-http.process_rss_after_1000_issues_mb", "rss_1000_issues_mb", True),
]

GATES = [
    ("exactly one checkout winner (in process)", "bn13.checkout_50_winners_per_round", lambda v: v == 1),
    ("exactly one checkout winner (HTTP)", "bn13-http.checkout50_exactly_one", lambda v: v == 1),
    ("0 lost wakes", "bn13.lost_wakes", lambda v: v == 0),
    ("a burst of 50 comments = 1 run", "bn13.burst50_runs", lambda v: v == 1),
    ("0 invariant violations", "bn13.invariant_violations", lambda v: v == 0),
    ("ledger exact at US$ 0.004 × 12", "bn13.ledger_error_usd_12x0.004", lambda v: v < 1e-9),
    ("0 runs after a hard budget stop", "bn13.runs_started_after_hard_stop", lambda v: v == 0),
    ("token cap bites on a subscription", "bn13.subscription_token_cap_enforced", lambda v: v == 1),
    ("audit tampering detected", "bn13.audit_tamper_detected", lambda v: v == 1),
    ("routine catch-up policies", "bn13.routine_catch_up_ok", lambda v: v == 1),
    ("export → import keeps the org chart", "bn13.roundtrip_org_chart_ok", lambda v: v == 1),
    ("search 10k issues p95 ≤ 25 ms", "bn13.search_10k_p95_ms", lambda v: v <= 25),
]

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--debug", action="store_true", help="build the core benches in debug too")
    ap.add_argument("--http-release", action="store_true", help="build the HTTP bench (app crate) in release")
    ap.add_argument("--skip-run", help="reuse a metrics.jsonl instead of running")
    ap.add_argument("--out", help="summary.json path (bench.py passes it)")
    a = ap.parse_args()
    stamp = time.strftime("%Y%m%d-%H%M%S")
    out_dir = ROOT / "estudos" / "bench" / f"{stamp}-bn13"
    out_dir.mkdir(parents=True, exist_ok=True)
    metrics_path = out_dir / "metrics.jsonl"
    ok = True
    if a.skip_run:
        metrics_path.write_text(pathlib.Path(a.skip_run).read_text())
    else:
        ok &= cargo(["-p", "omniget-core", "--lib", "company::bench", "--", "--test-threads=1", "--nocapture"], metrics_path, not a.debug)
        ok &= cargo(["-p", "omniget", "--lib", "local_bridge_company::bench", "--", "--test-threads=1", "--nocapture"], metrics_path, a.http_release)
    m = load_metrics(metrics_path)
    base = baseline()
    rows = []
    print()
    for label, key, bkey, lower in COMPARE:
        o, p = m.get(key), base.get(bkey)
        if o is None or p is None:
            print(f"- {label}: OmniGet {o} · Paperclip {p} (missing)")
            continue
        if lower:
            ratio = p / o if o > 0 else float("inf")
            better = o <= p
        else:
            ratio = o / p if p > 0 else float("inf")
            better = o >= p
        rows.append({"label": label, "omniget": o, "paperclip": p, "times_better": ratio, "better": better})
        print(f"- {label}: OmniGet {o:.3f} · Paperclip {p:.3f} · {ratio:.1f}× {'better' if better else 'WORSE'}")
    print()
    gates = []
    for label, key, test in GATES:
        v = m.get(key)
        passed = v is not None and test(v)
        gates.append({"gate": label, "value": v, "pass": passed})
        ok &= passed
        print(f"- [{'PASS' if passed else 'FAIL'}] {label} ({v})")
    summary = {
        "bench": "bn13",
        "when": stamp,
        "core_profile": "debug" if a.debug else "release",
        "http_profile": "release" if a.http_release else "debug",
        "comparisons": rows,
        "gates": gates,
        "all_pass": ok and all(r["better"] for r in rows),
        "metrics": m,
        "paperclip_baseline": base,
        "paperclip_extra": {"startup_s": base.get("startup_s"), "idle_rss_mb": base.get("idle_rss_mb")},
    }
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False) + "\n")
    if a.out:
        pathlib.Path(a.out).write_text(json.dumps(summary, indent=2, ensure_ascii=False) + "\n")
    print(f"\n{out_dir / 'summary.json'} — all_pass={summary['all_pass']}")
    sys.exit(0 if summary["all_pass"] else 1)

if __name__ == "__main__":
    main()
