#!/usr/bin/env python3
"""BN2 — parallel jobs and folder isolation benchmark (roadmap 1.5 and 1.6).

Scenario A (one writer per folder, 1.5): 4 jobs that write into one folder,
spelled 4 ways (the folder, a symlink to it, another letter case on
case-insensitive disks, its parent). Each job does a read-modify-write of a
shared ledger (two at once lose a line) and writes a file of its own.
Goals: 0 lines lost (0 overwrites), 0 pairs of jobs running together on the
same canonical folder, and undo of one job changes 0 bytes of the others —
while another job runs (refused, ERR_UNDO_SHARED), after all ended (only its
own paths go back), and when a later job changed its file (refused,
ERR_UNDO_CONFLICT). A control run without the gate shows the check does see
overwrites.

Scenario C (configurable cap, 1.6): 8 read jobs (no folder, `sleep` mode)
with the cap at 2, 4 and 8. Goals: wall time with cap 4 about 2x the median
of one job (it was ~4x with 2 slots, 81#P2), never more jobs at once than the
cap, and every queued job, sampled every 50 ms, with a position and a reason
that holds at that instant. The job length defaults to 10.4 s, the median
task of B0, so the 500 ms spawn stagger weighs what it weighs in real use.

How (`--fake`, the only mode): the measurements are Rust tests
(`core::llm::job_gate::bn2::*` in omniget-core) that run the real gate, the
real `FolderKey`, `scripts/llm/fake_claude.py` through the real
`spawn_turn_with` and `ClaudeParser`, and the real shadow-git snapshot/undo
bracketed like the Coordinator brackets a CLI turn. Each test appends a JSON
line per metric to $OMNIGET_BENCH_OUT; this script runs them, checks the
goals and writes one JSON report.

Level: e2e from the gate down (admission, stagger, process, parser,
snapshot, undo). NOT through `Jobs::run_job`/`LlmManager`/the Tauri app,
which cannot run headless here; `run_job` calls the same `Gate::acquire`
first thing (unit-tested separately in `jobs::queue_tests` and
`job_gate::tests`). Cost per task is unchanged by construction: the gate
never touches the turn's argv (checked below by the diff of the argv builder
and of the B0 harness); `missions_eval.py --real` is not run.

Usage: python3 scripts/llm/jobs_parallel_eval.py [--fake] [--reps 20]
       [--read-s 10.4] [--out FILE]
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


def run_tests(filter_, env_extra, out_file):
    env = dict(os.environ, OMNIGET_BENCH_OUT=str(out_file), **env_extra)
    cmd = ["cargo", "test", "-p", "omniget-core", "--lib", filter_, "--", "--test-threads=1"]
    p = subprocess.run(cmd, cwd=TAURI, env=env, capture_output=True, text=True)
    summary = [l for l in p.stdout.splitlines() if l.startswith("test result") or l.startswith("test ")]
    return {"cmd": " ".join(f"{k}={v}" for k, v in env_extra.items()) + " " + " ".join(cmd),
            "exit": p.returncode, "summary": summary,
            "stderr_tail": p.stderr[-2000:] if p.returncode else ""}


def read_metrics(path):
    per = {}
    if not path.exists():
        return per
    for line in path.read_text().splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        per[m["metric"]] = m["value"]
    return per


def goal(name, value, ok, target):
    return {"goal": name, "target": target, "value": value, "pass": bool(ok)}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fake", action="store_true", default=True)
    ap.add_argument("--real", action="store_true")
    ap.add_argument("--reps", type=int, default=20, help="scenario A repetitions")
    ap.add_argument("--read-s", type=float, default=10.4, help="length of one read job in scenario C")
    ap.add_argument("--out", default=str(ROOT / "estudos" / "87-onda1" / "bn2-jobs-parallel.json"))
    args = ap.parse_args()
    if args.real:
        sys.exit("--real is not offered here: the gate does not change the turn, so B0 "
                 "(missions_eval.py --reps 3) stays the cost check. Use --fake.")

    with tempfile.TemporaryDirectory() as d:
        out = pathlib.Path(d) / "bn2.jsonl"
        run = run_tests("core::llm::job_gate::bn2::", {
            "BN2_REPS": str(args.reps),
            "BN2_READ_S": str(args.read_s),
            "BN2_WRITE_S": "0.3",
        }, out)
        m = read_metrics(out)
    diff = subprocess.run(
        ["git", "diff", "--stat", "HEAD", "--",
         "src-tauri/omniget-core/src/core/llm/cli_runtime/claude.rs",
         "src-tauri/omniget-cli/src/commands/claude.rs",
         "scripts/llm/missions_eval.py"],
        cwd=ROOT, capture_output=True, text=True).stdout.strip()

    reps = m.get("a_reps")
    live = m.get("a_undo_while_live_codes") or []
    first = m.get("a_undo_first_codes") or []
    ratio4 = m.get("c_ratio_cap4")
    goals = [
        goal("A: control without the gate loses lines (the check sees overwrites)",
             m.get("a_control_lost_lines"), (m.get("a_control_lost_lines") or 0) >= 1, ">= 1"),
        goal("A: 0 overwrites (ledger lines lost)", m.get("a_lost_lines"),
             m.get("a_lost_lines") == 0, f"0 in {reps} reps x 4 jobs"),
        goal("A: 0 jobs together on one canonical folder", m.get("a_overlapping_pairs_same_folder"),
             m.get("a_overlapping_pairs_same_folder") == 0, "0 overlapping pairs"),
        goal("A: the queue said 'waiting for the folder'", m.get("a_max_waiting_workspace"),
             (m.get("a_max_waiting_workspace") or 0) >= 1, ">= 1 job in waiting_workspace"),
        goal("A: undo while another job runs is refused", live,
             reps and len(live) == reps and all(c == "ERR_UNDO_SHARED" for c in live),
             f"ERR_UNDO_SHARED x {reps}"),
        goal("A: undo after all ended restores only its own paths", m.get("a_undo_last_restored"),
             m.get("a_undo_last_restored") == reps, f"{reps} restored"),
        goal("A: undo of A changes 0 bytes of B", m.get("a_undo_last_bytes_of_others_changed"),
             m.get("a_undo_last_bytes_of_others_changed") == 0, "0 bytes"),
        goal("A: undo of a file changed later is refused, 0 bytes", {"codes": first, "bytes": m.get("a_undo_first_bytes_changed")},
             reps and len(first) == reps and all(c == "ERR_UNDO_CONFLICT" for c in first)
             and m.get("a_undo_first_bytes_changed") == 0, f"ERR_UNDO_CONFLICT x {reps}, 0 bytes"),
        goal("C: cap 4 wall time ~2x one job (was ~4x with 2 slots)", ratio4,
             ratio4 is not None and ratio4 <= 2.5, "<= 2.5x the median job"),
        goal("C: cap 2 wall time ~4x one job", m.get("c_ratio_cap2"),
             m.get("c_ratio_cap2") is not None and 3.5 <= m["c_ratio_cap2"] <= 4.6, "3.5x - 4.6x"),
        goal("C: never more jobs at once than the cap",
             {c: m.get(f"c_max_concurrent_cap{c}") for c in (2, 4, 8)},
             all((m.get(f"c_max_concurrent_cap{c}") or 99) <= c for c in (2, 4, 8)), "<= cap"),
        goal("C: no queued job without a position and a reason that holds", m.get("c_queued_without_reason"),
             m.get("c_queued_without_reason") == 0 and (m.get("c_queued_rows_sampled") or 0) > 0,
             f"0 of {m.get('c_queued_rows_sampled')} queued rows sampled"),
        goal("argv of the turn unchanged (cost per task = B0)", diff or "(empty)", not diff,
             "empty diff of cli_runtime/claude.rs, omniget-cli claude.rs, missions_eval.py"),
    ]
    report = {
        "bench": "BN2 jobs_parallel_eval",
        "when": dt.datetime.now().isoformat(timespec="seconds"),
        "mode": "fake",
        "level": "e2e from the gate down (gate, FolderKey, fake CLI process, ClaudeParser, shadow-git "
                 "snapshot/undo); not through Jobs::run_job / the Tauri app (headless not feasible)",
        "platform": sys.platform,
        "params": {"reps_a": args.reps, "write_hold_s": 0.3, "read_job_s": args.read_s,
                   "stagger_ms": 500, "spellings": ["folder", "symlink", "other case (macOS/Windows) or ./",
                                                     "parent folder"]},
        "not_measured": {"cost_per_task": "not run: the gate never changes the argv (see the argv goal); "
                                          "B0 missions_eval.py --real was not rerun on purpose",
                         "scenario_B": "2.3 (worktree), out of scope for wave 1"},
        "run": run,
        "metrics": m,
        "goals": goals,
        "all_pass": all(g["pass"] for g in goals) and run["exit"] == 0,
    }
    pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.out).write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    for g in goals:
        print(("PASS " if g["pass"] else "FAIL ") + g["goal"] + f"  ({g['value']})")
    print("all_pass:", report["all_pass"], "->", args.out)
    return 0 if report["all_pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
