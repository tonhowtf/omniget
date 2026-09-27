#!/usr/bin/env python3
"""BN3 (wave 1 part) — approvals benchmark (roadmap item 1.3).

Goals of wave 1:
  * 0 silent denials: 20 background jobs ask for `shell_exec`, nobody answers;
    every expiry must say so (ERR_TOOL_TIMEOUT + `outcome: expired` event);
  * ToolAsk -> OS notification within delay + 1 s (79#P3);
  * 0 notifications for asks answered in under 5 s;
  * `waiting_approval` visible through the CLI within 1 s.

How (`--fake`): Rust tests measure it.
  * `broker::tests::bn3_*` (omniget-core): the real ToolBroker with a
    per-request (job) timeout resolver; the 20 expiries and the ToolAsk event
    latency to a bus listener.
  * `llm_attention::tests` (omniget): the real notification gate (delay,
    re-check at delivery, focus, 5 s cooldown) under tokio's paused clock, so
    "5 s" is exact virtual time, with a recording sink instead of the OS.

Level: unit, with injected clocks. "waiting_approval via CLI <= 1 s" is the
measured in-process event latency plus the CLI's polling period (800 ms in
`omniget agent run|jobs`), an analytic bound: the job's state is written on
the event (no 1 s tick any more), and the CLI reads the row on its next poll.
Nothing here opens the app or a window. Wave-2 goals (approve via CLI, restart
-> stale) are not measured: they need 2.2.

Usage: python3 scripts/llm/approvals_eval.py [--fake] [--out FILE]
"""
import argparse
import datetime as dt
import json
import os
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
TAURI = ROOT / "src-tauri"


def run_tests(crate, filter_, out_file):
    env = dict(os.environ, OMNIGET_BENCH_OUT=str(out_file))
    cmd = ["cargo", "test", "-p", crate, "--lib", filter_, "--", "--test-threads=1"]
    p = subprocess.run(cmd, cwd=TAURI, env=env, capture_output=True, text=True)
    summary = [l for l in p.stdout.splitlines() if l.startswith("test result")]
    return {"cmd": " ".join(cmd), "exit": p.returncode, "summary": summary,
            "stderr_tail": p.stderr[-1500:] if p.returncode else ""}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fake", action="store_true", default=True)
    ap.add_argument("--real", action="store_true")
    ap.add_argument("--out", default=str(ROOT / "estudos" / "87-onda1" / "bn3-approvals.json"))
    args = ap.parse_args()
    if args.real:
        sys.exit("--real needs the app running with a hidden window and real asks; not offered in wave 1. Use --fake.")

    metrics = {}
    runs = []
    with tempfile.TemporaryDirectory() as d:
        out = pathlib.Path(d) / "bn3.jsonl"
        runs.append(run_tests("omniget-core", "broker::tests::bn3_", out))
        runs.append(run_tests("omniget", "llm_attention::tests", out))
        for line in out.read_text().splitlines() if out.exists() else []:
            m = json.loads(line)
            metrics[m["metric"]] = m["value"]

    delay = metrics.get("notify_delay_s")
    notif = metrics.get("ask_to_notification_s")
    lat_ms = metrics.get("ask_event_latency_max_ms")
    poll_ms = metrics.get("cli_poll_ms", 800)
    visible_s = None if lat_ms is None else (lat_ms + poll_ms) / 1000
    goals = [
        {"goal": "0 silent denials (20 job asks expire)", "target": 0,
         "value": {"silent": metrics.get("silent_denials"), "expired_with_reason": metrics.get("expired_with_reason"),
                   "asks": metrics.get("job_asks")},
         "pass": metrics.get("silent_denials") == 0 and metrics.get("expired_with_reason") == 20},
        {"goal": "ToolAsk -> notification within delay + 1 s", "target": f"<= {None if delay is None else delay + 1} s",
         "value": notif, "pass": notif is not None and delay is not None and notif <= delay + 1},
        {"goal": "0 notifications for asks answered in < 5 s", "target": 0,
         "value": {"notifications": metrics.get("notifications_for_asks_answered_under_5s"),
                   "asks": metrics.get("asks_answered_under_5s")},
         "pass": metrics.get("notifications_for_asks_answered_under_5s") == 0},
        {"goal": "waiting_approval visible via CLI within 1 s", "target": "<= 1 s",
         "value": {"event_latency_max_ms": lat_ms, "cli_poll_ms": poll_ms, "bound_s": visible_s},
         "pass": visible_s is not None and visible_s <= 1.0},
    ]
    report = {
        "bench": "BN3 approvals_eval (wave 1)",
        "when": dt.datetime.now().isoformat(timespec="seconds"),
        "mode": "fake",
        "level": "unit (real broker and notification gate, paused tokio clock); CLI visibility = measured event latency + 800 ms poll (analytic)",
        "runs": runs,
        "metrics": metrics,
        "goals": goals,
        "not_measured": ["approve via CLI -> tool in <= 1 s (wave 2, item 2.2)",
                         "restart -> stale/parked (wave 2, items 2.2/2.9)"],
        "all_pass": all(g["pass"] for g in goals) and all(r["exit"] == 0 for r in runs),
    }
    pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.out).write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    for g in goals:
        print(("PASS " if g["pass"] else "FAIL ") + g["goal"] + f"  ({g['value']})")
    print("all_pass:", report["all_pass"], "->", args.out)
    return 0 if report["all_pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
