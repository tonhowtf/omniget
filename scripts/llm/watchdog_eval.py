#!/usr/bin/env python3
"""BN1 — turn watchdog benchmark (roadmap item 1.2).

What it proves, with `scripts/llm/fake_claude.py` standing in for Claude:
  * stall-after-init  killed within idle + 2 s (79#P1; 82#P4 allows + 5 s)
  * silent-start      fails with ERR_CLI_NO_START
  * grandchild        0 descendants left, within 1 s of the kill (Foco 1)
  * grandchild-setsid the same for a grandchild that left the CLI's process
                      group with setsid() (killpg alone misses it)
  * ask-pending       a question open for 2.5x the idle window is NOT killed
  * tool-long         a tool silent for 2x the idle window is NOT killed
  * ok                a healthy turn is untouched
  * argv              the watchdog changes no argv (B0 stays valid)

How (`--fake`, the only mode that costs nothing): the measurements are Rust
tests (`cli_runtime::session::tests::bn1_*`) that spawn the fake through the
real `spawn_turn_with` with the real `ClaudeParser`: real processes, pipes and
tree kill, windows scaled 600 s -> 2 s (tool 1800 s -> 12 s, start 90 s ->
2 s). Each test appends a JSON line per metric to $OMNIGET_BENCH_OUT; this
script runs them, checks the goals and writes one JSON report.

Level: end to end at the turn (session) level. The job layer on top — job
`failed` with `failure_reason = "stalled"`, slot handed back, 0 `running` in
`omniget agent jobs` — is covered by unit tests of `jobs.rs`
(`watchdog_tests`) and follows structurally: the runner returns (dropping its
slot) as soon as the turn stream ends, which the report measures as
`stream_closed_after_error_ms`. Driving the Tauri app headless is not done.

`--real` is refused on purpose: a real stalled `claude -p` costs quota.

Usage: python3 scripts/llm/watchdog_eval.py [--fake] [--reps N] [--out FILE]
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
IDLE_S = 2.0
START_S = 2.0


def run_tests(filter_, crate, out_file):
    env = dict(os.environ, OMNIGET_BENCH_OUT=str(out_file))
    cmd = ["cargo", "test", "-p", crate, "--lib", filter_, "--", "--test-threads=1"]
    p = subprocess.run(cmd, cwd=TAURI, env=env, capture_output=True, text=True)
    summary = [l for l in p.stdout.splitlines() if l.startswith("test result")]
    return {"cmd": " ".join(cmd), "exit": p.returncode, "summary": summary,
            "stderr_tail": p.stderr[-1500:] if p.returncode else ""}


def read_metrics(path):
    per = {}
    if not path.exists():
        return per
    for line in path.read_text().splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        per.setdefault(m["metric"], []).append(m["value"])
    return per


def goal(name, value, ok, target):
    return {"goal": name, "target": target, "value": value, "pass": bool(ok)}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fake", action="store_true", default=True)
    ap.add_argument("--real", action="store_true")
    ap.add_argument("--reps", type=int, default=1)
    ap.add_argument("--out", default=str(ROOT / "estudos" / "87-onda1" / "bn1-watchdog.json"))
    args = ap.parse_args()
    if args.real:
        sys.exit("--real is not offered: a stalled real `claude -p` spends quota for nothing. Use --fake.")

    runs = []
    metrics = {}
    for _ in range(max(1, args.reps)):
        with tempfile.TemporaryDirectory() as d:
            out = pathlib.Path(d) / "bn1.jsonl"
            runs.append(run_tests("cli_runtime::session::tests::bn1_", "omniget-core", out))
            for k, v in read_metrics(out).items():
                metrics.setdefault(k, []).extend(v)
        # The argv proof: the watchdog only fills timing fields.
    with tempfile.TemporaryDirectory() as d:
        argv_run = run_tests("the_watchdog_leaves_the_argv_alone", "omniget-core", pathlib.Path(d) / "x")
    diff = subprocess.run(
        ["git", "diff", "--stat", "HEAD", "--",
         "src-tauri/omniget-core/src/core/llm/cli_runtime/claude.rs", "scripts/llm/missions_eval.py"],
        cwd=ROOT, capture_output=True, text=True).stdout.strip()

    mx = lambda k: max(metrics[k]) if metrics.get(k) else None  # noqa: E731
    allv = lambda k: metrics.get(k, [])  # noqa: E731
    kill = mx("stall_after_init_kill_s")
    goals = [
        goal("stall-after-init killed within idle + 2 s", kill,
             kill is not None and kill <= IDLE_S + 2, f"<= {IDLE_S + 2} s (idle {IDLE_S} s)"),
        goal("stall-after-init killed within idle + 5 s (82#P4)", kill,
             kill is not None and kill <= IDLE_S + 5, f"<= {IDLE_S + 5} s"),
        goal("stall is ERR_CLI_STALLED", allv("stall_after_init_code"),
             allv("stall_after_init_code") and all(c == "ERR_CLI_STALLED" for c in allv("stall_after_init_code")),
             "ERR_CLI_STALLED"),
        goal("silent-start is ERR_CLI_NO_START", allv("silent_start_code"),
             allv("silent_start_code") and all(c == "ERR_CLI_NO_START" for c in allv("silent_start_code")),
             f"ERR_CLI_NO_START within {START_S} s + 2 s"),
        goal("grandchild: 0 survivors", mx("grandchild_survivors"),
             allv("grandchild_survivors") and mx("grandchild_survivors") == 0, "0 (Unix: kill(pid, 0))"),
        goal("grandchild gone within 1 s", mx("grandchild_gone_after_ms"),
             mx("grandchild_gone_after_ms") is not None and mx("grandchild_gone_after_ms") <= 1000, "<= 1000 ms"),
        goal("grandchild-setsid (left the group): 0 survivors", mx("grandchild_setsid_survivors"),
             allv("grandchild_setsid_survivors") and mx("grandchild_setsid_survivors") == 0,
             "0 (Unix: kill(pid, 0))"),
        goal("grandchild-setsid gone within 1 s", mx("grandchild_setsid_gone_after_ms"),
             mx("grandchild_setsid_gone_after_ms") is not None
             and mx("grandchild_setsid_gone_after_ms") <= 1000, "<= 1000 ms"),
        goal("ask-pending (2.5x idle) not killed", allv("ask_pending_survived"),
             allv("ask_pending_survived") and all(allv("ask_pending_survived")), "survives"),
        goal("tool-long (2x idle, inside the tool window) not killed", allv("tool_long_survived"),
             allv("tool_long_survived") and all(allv("tool_long_survived")), "survives"),
        goal("healthy turn untouched", allv("ok_survived"),
             allv("ok_survived") and all(allv("ok_survived")), "no error"),
        goal("turn stream closes right after the stall (slot freed)", mx("stream_closed_after_error_ms"),
             mx("stream_closed_after_error_ms") is not None and mx("stream_closed_after_error_ms") <= 1000,
             "<= 1000 ms"),
        goal("argv unchanged by the watchdog", {"test": argv_run["summary"], "git_diff_claude_rs_and_harness": diff or "(empty)"},
             argv_run["exit"] == 0 and not diff, "test passes and the argv builder/harness diff is empty"),
    ]
    report = {
        "bench": "BN1 watchdog_eval",
        "when": dt.datetime.now().isoformat(timespec="seconds"),
        "mode": "fake",
        "level": "e2e at the turn (fake CLI process, real parser, real tree kill); job layer by unit tests",
        "scale": {"idle_s": IDLE_S, "tool_idle_s": 12.0, "start_s": START_S,
                  "stands_for": {"idle_s": 600, "tool_idle_s": 1800, "start_s": 90}},
        "reps": args.reps,
        "platform": sys.platform,
        "windows_grandchild": "not run here: #[cfg(windows)] test core::process::tree_tests::"
                              "terminate_kills_the_grandchild_on_windows runs on the Windows CI runner",
        "runs": runs,
        "metrics": metrics,
        "goals": goals,
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
