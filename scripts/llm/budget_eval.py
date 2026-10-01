#!/usr/bin/env python3
"""BN7 — budget policies with a scope, an incident and subscription metrics
(roadmap item 3.3, S18, decision D5; 80#P3, 80#P4, 85#P6).

Goals:
  * a loop of 20 rounds under a US$ 0.05 hard ceiling spends at most the
    ceiling plus one turn;
  * 0 jobs started after the hard stop (the next round is never created, and
    a turn someone starts in the paused scope is refused before the CLI runs);
  * an incident is open;
  * "raise and resume" starts the next round within 2 s;
  * `QuotaPct(5h) = 5%` stops within one turn of the window moving 5 points;
  * a subscription account (the CLI reports no price) with a ceiling blocks;
  * a checkpoint (soft ceiling) asks through the approvals broker (2.2).

How (`--fake`, the default): Rust tests in omniget-core,
`core::llm::budget::policy::bn7_tests::bn7_*`. The real Coordinator (gate
before the turn, charge when it settles), the real CLI runtime and
`ClaudeParser`, and `scripts/llm/fake_claude.py` as the CLI: its cost, usage
and `rate_limit_event` come from its env, per conversation. The loop driver
does what `Jobs::run_loop` does about budgets (ask `Engine::blocked` before a
round, bind the round to `loop:<id>`/`job:<id>`) and nothing else.

`--real` (spends quota): the same loop on the installed `claude` with Haiku
(`bn7_real_haiku_loop_stops_at_its_ceiling`, ignored by default).

Level: turn path + engine, not the Tauri app (the row pausing, the scope's
job cancel on `BudgetCrossed` and the resume after a raise are app code,
unit-tested in `jobs::tests::a_budget_scope_*` and `llm_budget::tests`).

Usage: python3 scripts/llm/budget_eval.py [--fake | --real] [--out FILE]
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

def run_real(out_file):
    env = dict(os.environ, OMNIGET_BENCH_OUT=str(out_file))
    cmd = ["cargo", "test", "--lib", "-p", "omniget-core", "budget::policy::bn7_tests::bn7_real_",
           "--", "--ignored", "--test-threads=1", "--nocapture"]
    p = subprocess.run(cmd, cwd=TAURI, env=env, capture_output=True, text=True)
    summary = [l for l in p.stdout.splitlines() if l.startswith("test result")]
    return {"cmd": " ".join(cmd), "exit": p.returncode, "summary": summary,
            "stderr_tail": p.stderr[-2500:] if p.returncode else ""}

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fake", action="store_true", default=True)
    ap.add_argument("--real", action="store_true", help="also the loop on the installed claude with Haiku (spends quota)")
    ap.add_argument("--out", default=str(ROOT / "estudos" / "91-onda3" / "bn7-budget-3.3.json"))
    args = ap.parse_args()

    metrics = {}
    runs = []
    with tempfile.TemporaryDirectory() as d:
        out = pathlib.Path(d) / "bn7.jsonl"
        runs.append(run_tests("omniget-core", "budget::policy::bn7_tests::bn7_", out))
        if args.real:
            runs.append(run_real(out))
        for line in out.read_text().splitlines() if out.exists() else []:
            m = json.loads(line)
            metrics[m["metric"]] = m["value"]

    m = metrics.get
    ceiling, per_turn = m("loop_ceiling_usd"), m("loop_cost_per_turn_usd")
    goals = [
        {"goal": "20-round loop, US$ 0.05 hard ceiling: spend <= ceiling + 1 turn",
         "target": None if ceiling is None else f"<= {ceiling} + {per_turn}",
         "value": {"spent_usd": m("loop_spent_usd"), "rounds_started": m("loop_rounds_started"),
                   "rounds_planned": m("loop_rounds_planned")},
         "pass": None not in (m("loop_spent_usd"), ceiling, per_turn)
                 and m("loop_spent_usd") <= ceiling + per_turn + 1e-9
                 and m("loop_rounds_started", 99) < m("loop_rounds_planned", 0)},
        {"goal": "0 jobs started after the hard stop", "target": 0,
         "value": {"started_after_hard": m("loop_jobs_started_after_hard"),
                   "a_new_turn_refused_before_the_cli": m("loop_refused_after_hard")},
         "pass": m("loop_jobs_started_after_hard") == 0 and m("loop_refused_after_hard") is True},
        {"goal": "an incident is open", "target": True, "value": m("loop_incident_open"),
         "pass": m("loop_incident_open") is True},
        {"goal": "raise and resume <= 2 s", "target": "<= 2000 ms", "value": m("raise_resume_ms"),
         "pass": m("raise_resume_ms") is not None and m("raise_resume_ms") <= 2000},
        {"goal": "QuotaPct(5h) = 5% stops within 1 turn", "target": "<= 1 turn past",
         "value": {"turns_past_ceiling": m("quota_turns_past_ceiling"), "rounds_started": m("quota_rounds_started"),
                   "window_used_pct": m("quota_window_used_pct"), "step_points": m("quota_step_points")},
         "pass": m("quota_turns_past_ceiling") is not None and m("quota_turns_past_ceiling") <= 1
                 and m("quota_incident_open") is True},
        {"goal": "a subscription account with a ceiling blocks", "target": True,
         "value": {"blocked": m("sub_blocked"), "rounds_started": m("sub_rounds_started"),
                   "spent_usd_estimated": m("sub_spent_usd_estimated")},
         "pass": m("sub_blocked") is True},
        {"goal": "B0 without a ceiling: no price lookup, no question, per-turn overhead < 5 ms", "target": "< 5 ms",
         "value": {"overhead_ms_per_turn": m("b0_no_policy_overhead_ms_per_turn"),
                   "price_lookups": m("b0_no_policy_price_lookups")},
         "pass": m("b0_no_policy_overhead_ms_per_turn") is not None and m("b0_no_policy_overhead_ms_per_turn") < 5
                 and m("b0_no_policy_price_lookups") == 0},
        {"goal": "a checkpoint asks through the approvals (2.2)", "target": True,
         "value": m("checkpoint_asked_via_approvals"), "pass": m("checkpoint_asked_via_approvals") is True},
    ]
    if args.real:
        rc = m("real_cost_per_turn_usd") or 0
        goals.append({
            "goal": "real Haiku loop: spend <= US$ 0.05 + 1 turn, 0 started after hard",
            "target": f"<= 0.05 + {rc}",
            "value": {k[5:]: v for k, v in metrics.items() if k.startswith("real_")},
            "pass": m("real_spent_usd") is not None and m("real_spent_usd") <= 0.05 + rc + 1e-9
                    and m("real_jobs_started_after_hard") == 0 and m("real_incident_open") is True})
    report = {
        "bench": "BN7 budget_eval (item 3.3)",
        "when": dt.datetime.now().isoformat(timespec="seconds"),
        "mode": "real" if args.real else "fake",
        "level": "turn path: real Coordinator + CliRuntime + ClaudeParser + policy engine (SQLite), fake_claude.py as the CLI; the loop driver mirrors run_loop's budget checks; not the Tauri app",
        "runs": runs,
        "metrics": metrics,
        "goals": goals,
        "not_measured": ["the Tauri side end to end (row paused, scope jobs cancelled on BudgetCrossed, loop resumed after a raise): unit-tested pieces, no window opened",
                         "QuotaPct from the status line (only rate_limit_event is exercised)"],
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
