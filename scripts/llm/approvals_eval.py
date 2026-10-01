#!/usr/bin/env python3
"""BN3 — approvals benchmark (roadmap items 1.3 and 2.2).

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
Nothing here opens the app or a window.

Goals of wave 2 (item 2.2, durable approvals answerable from anywhere):
  * `omniget agent approve` -> the tool runs within 1 s (p50): the production
    guard and approvals handlers on 127.0.0.1 (`approvals::tests::bn3_*`,
    omniget), a real broker and run record, a CLI-scoped token; 20 asks
    listed with `GET /v1/approvals?job_id=` and answered with
    `POST /v1/approvals/{id}` exactly as the CLI sends them;
  * scenario `restart`: the process holding the waiter dies, the next boot
    reconciles, the ask is `stale` with "restart the turn", an answer is
    refused as stale and `cancel` closes it (`broker::a11_permission_tests::
    bn3_restart_*`, omniget-core);
  * 0 orphan asks in 20 ask/answer cycles through every ending (accept,
    decline, accept for the session, cancel, timeout); every second answer is
    refused (`ERR_NO_ASK` over HTTP); 0 ask ids in what the agent sees (tool
    result, errors, the bus of its turn).
Level: HTTP + guard + broker + run record, not the Tauri app (the real
handlers only add `from_app`, which picks the same broker and registry).

Usage: python3 scripts/llm/approvals_eval.py [--fake] [--out FILE]
"""
from prepared import run_tests as run_prepared_tests
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
    cmd, p = run_prepared_tests(crate, filter_, env)
    summary = [l for l in p.stdout.splitlines() if l.startswith("test result")]
    return {"cmd": " ".join(cmd), "exit": p.returncode, "summary": summary,
            "stderr_tail": p.stderr[-1500:] if p.returncode else ""}

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fake", action="store_true", default=True)
    ap.add_argument("--real", action="store_true")
    ap.add_argument("--out", default=str(ROOT / "estudos" / "88-onda2" / "bn3-approvals-2.2.json"))
    args = ap.parse_args()
    if args.real:
        sys.exit("--real needs the app running with a hidden window and real asks; not offered. Use --fake.")

    metrics = {}
    runs = []
    with tempfile.TemporaryDirectory() as d:
        out = pathlib.Path(d) / "bn3.jsonl"
        runs.append(run_tests("omniget-core", "broker::tests::bn3_", out))
        runs.append(run_tests("omniget", "llm_attention::tests", out))
        runs.append(run_tests("omniget-core", "broker::a11_permission_tests::bn3_", out))
        runs.append(run_tests("omniget", "approvals::tests::bn3_", out))
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
    p50 = metrics.get("approve_http_to_tool_ms_p50")
    goals += [
        {"goal": "omniget agent approve (HTTP) -> tool runs within 1 s p50", "target": "<= 1000 ms",
         "value": {"p50_ms": p50, "max_ms": metrics.get("approve_http_to_tool_ms_max"),
                   "asks": metrics.get("approve_http_asks"),
                   "in_process_p50_ms": metrics.get("respond_to_tool_ms_p50_inprocess")},
         "pass": p50 is not None and p50 <= 1000 and metrics.get("approve_http_asks") == 20},
        {"goal": "restart -> the pending ask is stale, says restart the turn, and can be closed", "target": "stale",
         "value": {"state": metrics.get("restart_state"), "note": metrics.get("restart_note_says_restart"),
                   "resolvable": metrics.get("restart_resolvable")},
         "pass": metrics.get("restart_state") == "stale" and metrics.get("restart_note_says_restart") is True
                 and metrics.get("restart_resolvable") is True},
        {"goal": "0 orphan asks in 20 cycles", "target": 0,
         "value": metrics.get("orphan_asks_after_20_cycles"),
         "pass": metrics.get("orphan_asks_after_20_cycles") == 0},
        {"goal": "a second answer is refused (ERR_NO_ASK)", "target": "20/20",
         "value": {"refused": metrics.get("second_answer_refused"), "http_code": metrics.get("http_second_answer_code")},
         "pass": metrics.get("second_answer_refused") == 20 and metrics.get("http_second_answer_code") == "ERR_NO_ASK"},
        {"goal": "the ask id never reaches the agent", "target": 0,
         "value": metrics.get("ask_ids_in_agent_stream"),
         "pass": metrics.get("ask_ids_in_agent_stream") == 0},
    ]
    report = {
        "bench": "BN3 approvals_eval (waves 1 and 2)",
        "when": dt.datetime.now().isoformat(timespec="seconds"),
        "mode": "fake",
        "level": "wave 1: unit (real broker and notification gate, paused tokio clock); CLI visibility = measured event latency + 800 ms poll (analytic). wave 2: HTTP on 127.0.0.1 through the production guard and approvals handlers, real broker and run record (no Tauri window)",
        "runs": runs,
        "metrics": metrics,
        "goals": goals,
        "not_measured": ["the omniget binary itself (its request body is unit-tested; the HTTP call is the one measured)",
                         "a real claude -p waiting on the prompt tool (SP1 measured >= 30 min; 2.9 not needed)"],
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
