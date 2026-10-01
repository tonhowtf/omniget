#!/usr/bin/env python3
"""BN11 — Arena benchmark (roadmap item 3.8, S23; 84#P4, 78#P3).

3 agents x 4 code tasks (t03-bugfix-py, t04-feature-py, t05-bugfix-js,
t09-rename from `missions_eval.py`, seeded by the harness's own `seed()`).
Each task becomes a git repository (the user's checkout, on `main`) and one
arena with 3 columns. The agents are `fake_claude.py --mode apply`, each run
with its cwd in the worktree of its column, with a known change:

  * a: the change that solves the task (the one `diff_eval.py` uses);
  * b: the same + a `NOTES.md` of its own (diverges on one file);
  * c: the same, but the first file it writes ends with a comment of its own
    (diverges on that file).

So the files where the columns split are known in advance: {NOTES.md, the
file c changed}. The check command of the arena is the task's own `check()`.

Goals:
  * 0 writes outside a column's own worktree: the user's repository (files
    outside .git, HEAD, `ls-files -s`, `status`, branches) hashed before the
    arena and after every column finished, equal in 4/4; and each column's
    commit holds exactly the files its agent wrote (12/12);
  * the comparison marks exactly the expected files as divergent (4/4);
  * the side-by-side has time, cost, diff stats and checks for every column
    (12/12 done, checks passed, cost as reported);
  * "choose this" (column a) merges in the user's checkout through 3.5's
    integrate, in < 2 s (max of 4), the checkout clean after, the merged tree
    = column a's files, and the task solved in the user's repository (4/4);
  * "remove the others": 0 orphans (worktree listed, folder on disk, branch,
    or anything `git worktree prune` would still clean) in 4/4, with the
    losers' work still reachable from the arena's refs (8/8); after removing
    the winner too, only the user's checkout is left and no `omniget/*`
    branch;
  * the turn's argv is unchanged (cost per task = B0: the arena is host-side).

How (`--fake`, the only mode): `commands::central::arena::bn11::*` (omniget)
runs the real threads engine, the git reactor (one worktree per thread), the
arena's finalization (commit, diff, check), `compare_core`, `apply_entry`
and `cleanup_core`; this script seeds the fixtures, checks the results and
runs each task's `check()` on the user's repository after the merge. Level:
e2e below the Tauri command layer; the driver is simulated (fake_claude +
the runtime events a driver emits), not a real Claude turn.

Usage: python3 scripts/llm/arena_eval.py [--fake] [--out FILE]
"""
from prepared import run_tests as run_prepared_tests
import argparse
import datetime as dt
import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
CODE_TASKS = ["t03-bugfix-py", "t04-feature-py", "t05-bugfix-js", "t09-rename"]
COSTS = {"a": 0.031, "b": 0.024, "c": 0.042}
INTEGRATE_MAX_MS = 2000

def load(name, file):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / "llm" / file)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod

def variants(diff_eval, name, seed):
    a = diff_eval.changes_for(name, seed)
    b = json.loads(json.dumps(a))
    b["write"]["NOTES.md"] = "# Notes\n\nAgent b left this file; nobody else did.\n"
    c = json.loads(json.dumps(a))
    first = sorted(c["write"])[0]
    mark = "// agent c was here\n" if first.endswith(".js") else "# agent c was here\n"
    c["write"][first] = c["write"][first] + mark
    return {"a": a, "b": b, "c": c}, sorted({"NOTES.md", first})

CHECK = '''import importlib.util, pathlib, sys
spec = importlib.util.spec_from_file_location("missions_eval", {me!r})
me = importlib.util.module_from_spec(spec); spec.loader.exec_module(me)
task = {{t.id: t for t in me.TASKS}}[sys.argv[1]]
ok, detail = task.check(pathlib.Path.cwd())
print(detail)
sys.exit(0 if ok else 1)
'''

def read_metrics(path):
    per = {}
    if path.exists():
        for line in path.read_text().splitlines():
            try:
                m = json.loads(line)
            except ValueError:
                continue
            per.setdefault(m["task"], {})[m["metric"]] = m["value"]
    return per

def goal(name, value, ok, target):
    return {"goal": name, "target": target, "value": value, "pass": bool(ok)}

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fake", action="store_true", default=True)
    ap.add_argument("--real", action="store_true")
    ap.add_argument("--out", default=str(ROOT / "estudos" / "91-onda3" / "bn11-arena-3.8.json"))
    args = ap.parse_args()
    if args.real:
        sys.exit("--real is not offered: the arena is host-side and never changes the turn, so B0 "
                 "(missions_eval.py --reps 3) stays the cost check. Use --fake.")
    me = load("missions_eval", "missions_eval.py")
    de = load("diff_eval", "diff_eval.py")
    tasks = {t.id: t for t in me.TASKS}
    with tempfile.TemporaryDirectory(prefix="bn11-") as d:
        root = pathlib.Path(d).resolve()
        check_py = root / "check.py"
        check_py.write_text(CHECK.format(me=str(ROOT / "scripts" / "llm" / "missions_eval.py")))
        expected = {}
        for name in CODE_TASKS:
            seed = root / name / "seed"
            seed.mkdir(parents=True)
            tasks[name].seed(seed)
            vs, split = variants(de, name, seed)
            expected[name] = split
            agents = []
            for label, ch in vs.items():
                (root / name / f"changes-{label}.json").write_text(json.dumps(ch))
                agents.append({"label": label, "changes": f"changes-{label}.json", "cost": COSTS[label]})
            (root / name / "agents.json").write_text(json.dumps({
                "prompt": f"Solve {name}",
                "verify": f"PYTHONDONTWRITEBYTECODE=1 {sys.executable} {check_py} {name}",
                "agents": agents,
            }))
        (root / "tasks.json").write_text(json.dumps(CODE_TASKS))
        out = root / "bn11.jsonl"
        env = dict(os.environ, OMNIGET_BENCH_OUT=str(out), BN11_FIXTURES=str(root))
        cmd, p = run_prepared_tests("omniget", "commands::central::arena::bn11::", env)
        m = read_metrics(out)
        solved = {}
        for name in CODE_TASKS:
            repo = root / name / "repo"
            if repo.exists():
                ok, detail = tasks[name].check(repo)
                solved[name] = {"ok": bool(ok), "detail": detail}
            else:
                solved[name] = {"ok": False, "detail": "no repository left"}
    argv = subprocess.run(
        ["git", "diff", "--stat", "HEAD", "--",
         "src-tauri/omniget-core/src/core/llm/cli_runtime/claude.rs",
         "src-tauri/omniget-cli/src/commands/claude.rs",
         "scripts/llm/missions_eval.py"],
        cwd=ROOT, capture_output=True, text=True).stdout.strip()

    per = [m.get(n, {}) for n in CODE_TASKS]
    entries = [e for t in per for e in t.get("entries", [])]
    agents_ok = sum(t.get("agents_ok", 0) for t in per)
    done = sum(1 for e in entries if e.get("status") == "done")
    checks = sum(1 for e in entries if e.get("checks") == "passed")
    costs = sum(1 for e in entries if e.get("cost_usd") is not None
                and abs(e["cost_usd"] - COSTS.get(e.get("label"), -1)) < 1e-9)
    timed = sum(1 for e in entries if (e.get("duration_ms") or 0) > 0 and (e.get("files") or 0) > 0)
    base_same = sum(1 for t in per if t.get("base_unchanged") is True)
    own = sum(t.get("own_files_only", 0) for t in per)
    div_ok = {n: {"got": m.get(n, {}).get("divergent"), "want": expected.get(n)} for n in CODE_TASKS}
    div_pass = sum(1 for v in div_ok.values() if v["got"] == v["want"])
    integ = [t.get("integrate_ms") for t in per]
    integ_max = max((x for x in integ if x is not None), default=None)
    via = [t.get("integrate_via") for t in per]
    winner_ok = sum(1 for t in per if t.get("integrated_is_winner") is True)
    clean_after = sum(1 for t in per if t.get("checkout_clean_after") is True)
    orphans = {n: m.get(n, {}).get("orphans_after_remove_others") for n in CODE_TASKS}
    no_orphans = sum(1 for v in orphans.values() if isinstance(v, dict) and v.get("total") == 0)
    kept = sum(t.get("loser_work_in_arena_refs", 0) for t in per)
    after_all = [(t.get("worktrees_after_all"), t.get("omniget_branches_after_all")) for t in per]
    cleanup_errors = {n: m.get(n, {}).get("cleanup_errors") for n in CODE_TASKS}
    n_entries = 3 * len(CODE_TASKS)
    goals = [
        goal("agents ran (3 per task)", f"{agents_ok}/{n_entries}", agents_ok == n_entries, f"{n_entries}/{n_entries}"),
        goal("0 writes outside the own worktree: user's repository hash before == after all columns",
             f"{base_same}/4", base_same == 4, "4/4"),
        goal("0 writes outside the own worktree: each column's commit = exactly its agent's files",
             f"{own}/{n_entries}", own == n_entries, f"{n_entries}/{n_entries}"),
        goal("divergent files == the files where the agents differ", div_ok, div_pass == 4, "4/4"),
        goal("side by side: every column done with time, diff stats, cost and checks",
             {"done": done, "timed_with_diff": timed, "cost_as_reported": costs, "checks_passed": checks},
             done == timed == costs == checks == n_entries, f"{n_entries} each"),
        goal("'choose this' merges in the user's checkout through integrate (3.5)", via,
             via == ["checkout"] * 4, "checkout x4"),
        goal("integrate time", {"per_task_ms": integ, "max_ms": integ_max},
             integ_max is not None and integ_max < INTEGRATE_MAX_MS, f"< {INTEGRATE_MAX_MS} ms (max)"),
        goal("merged tree = the chosen column's files, checkout clean after",
             {"winner": winner_ok, "clean": clean_after}, winner_ok == 4 and clean_after == 4, "4/4"),
        goal("the chosen change solves the task in the user's repository (missions_eval check)",
             {k: v["ok"] for k, v in solved.items()}, all(v["ok"] for v in solved.values()), "4/4"),
        goal("0 orphans after 'remove the others' (listed, folder, branch, prunable)", orphans,
             no_orphans == 4, "total 0 in 4/4"),
        goal("the losers' work stays reachable in the arena's refs", f"{kept}/8", kept == 8, "8/8"),
        goal("cleanup reported no error", cleanup_errors,
             all(v == [] for v in cleanup_errors.values()), "[] x4"),
        goal("after removing the winner too: only the user's checkout, no omniget/* branch", after_all,
             after_all == [(1, 0)] * 4, "(1 worktree, 0 branches) x4"),
        goal("turn argv unchanged (cost per task = B0)", argv or "no diff", argv == "", "no diff"),
    ]
    report = {
        "bench": "BN11", "item": "3.8", "mode": "fake",
        "when": dt.datetime.now().astimezone().isoformat(timespec="seconds"),
        "level": "threads engine + git reactor + arena finalize/compare/apply/cleanup, driver simulated "
                 "(fake_claude --mode apply in each worktree + the runtime events a driver emits)",
        "run": {"cmd": " ".join(cmd), "exit": p.returncode,
                "summary": [l for l in p.stdout.splitlines() if l.startswith("test ")],
                "stderr_tail": p.stderr[-2000:] if p.returncode else ""},
        "goals": goals,
        "all_pass": p.returncode == 0 and all(g["pass"] for g in goals),
        "per_task": m,
        "solved": solved,
        "not_measured": [
            "real driver turns (Claude/Codex): cost and time per column come from the fake's result line",
            "the Tauri command layer and the UI (headless app not run here)",
        ],
    }
    pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.out).write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    for g in goals:
        print(f"{'PASS' if g['pass'] else 'FAIL'}  {g['goal']}: {json.dumps(g['value'], ensure_ascii=False)}")
    print(f"BN11: {'ALL PASS' if report['all_pass'] else 'FAILED'} -> {args.out}")
    return 0 if report["all_pass"] else 1

if __name__ == "__main__":
    sys.exit(main())
