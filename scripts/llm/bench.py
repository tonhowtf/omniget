#!/usr/bin/env python3
"""One entry point for the /llm benchmarks: build once, run only what changed, in parallel.

Usage (from the repo root):
    python3 scripts/llm/bench.py              # what the working tree touches, fake benches only
    python3 scripts/llm/bench.py --quick      # same, fewer repetitions (seconds, for iterating)
    python3 scripts/llm/bench.py --all        # every fake bench (before a commit)
    python3 scripts/llm/bench.py --real       # + B0 (missions_eval, real Claude turns, spends quota)
    python3 scripts/llm/bench.py --only bn1,bn4

Why this exists: each BN script runs its own `cargo test`, and run one after the
other they rebuild, then wait on each other. Here the test binaries are built
once, the benches the diff can affect run side by side, and B0 (the only one
that costs money) runs only when the turn's argv or the harness moved, with its
missions in parallel.

Selection is by path: a bench runs when a changed file (vs HEAD, plus untracked)
matches one of its prefixes. Cargo.toml/Cargo.lock changes select every fake
bench. Results land in `estudos/bench/<timestamp>/` with a `summary.json`.
Exit code 0 only when every bench that ran passed.

Python 3.9, stdlib only.
"""
from prepared import prepare
import argparse
import json
import os
import pathlib
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

ROOT = pathlib.Path(__file__).resolve().parents[2]
TAURI = ROOT / "src-tauri"
CORE_LLM = "src-tauri/omniget-core/src/core/llm/"

# name -> (script, prefixes that select it, quick args, crates its tests live in)
BENCHES = {
    "bn1": (
        "watchdog_eval.py",
        [CORE_LLM + "cli_runtime/", "src-tauri/omniget-core/src/core/process", "scripts/llm/fake_claude.py",
         "scripts/llm/watchdog_eval.py", "src-tauri/src/commands/llm/watchdog.rs"],
        [],
        ["omniget-core", "omniget"],
    ),
    "bn2": (
        "jobs_parallel_eval.py",
        [CORE_LLM + "job_gate.rs", CORE_LLM + "folder_key.rs", CORE_LLM + "snapshot.rs", "src-tauri/src/jobs.rs",
         "src-tauri/omniget-core/src/core/vcs/worktree.rs", "src-tauri/omniget-core/src/core/vcs/job_worktree",
         "src-tauri/omniget-core/src/core/vcs/checkpoint.rs",
         "src-tauri/src/missions.rs", "scripts/llm/fake_claude.py", "scripts/llm/jobs_parallel_eval.py"],
        ["--reps", "5", "--read-s", "3"],
        ["omniget-core"],
    ),
    "bn3": (
        "approvals_eval.py",
        [CORE_LLM + "broker.rs", "src-tauri/src/llm_manager.rs", "src-tauri/src/llm_attention.rs",
         "src-tauri/src/jobs.rs", "scripts/llm/approvals_eval.py"],
        [],
        ["omniget-core", "omniget"],
    ),
    "bn4": (
        "diff_eval.py",
        [CORE_LLM + "snapshot.rs", "src-tauri/src/jobs.rs", "src-tauri/src/local_bridge_jobs.rs",
         "src-tauri/src/mcp/orchestration.rs", "scripts/llm/fake_claude.py", "scripts/llm/diff_eval.py"],
        ["--reps", "5"],
        ["omniget-core", "omniget"],
    ),
}
# The argv of a real turn: if any of these moved, B0 no longer measures production.
B0_PATHS = [CORE_LLM + "cli_runtime/claude.rs", CORE_LLM + "cli_runtime/mod.rs", "scripts/llm/missions_eval.py"]
EVERYTHING = ["src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "src-tauri/omniget-core/Cargo.toml"]
# B0 gate (estudos/86-sintese-orquestracao/ROADMAP.md, B0): median per mission.
B0_COST, B0_TIME = 0.029, 10.4
# Run-to-run variance of real turns is high (the same argv measured US$ 0.029
# and 0.031 on different days): within this margin above the gate is a warning,
# beyond it a failure. A tighter gate would cry wolf on every run.
B0_TOLERANCE = 0.10


def changed_files():
    result = subprocess.run(["git", "status", "--porcelain=v1", "-z", "--untracked-files=all"], cwd=ROOT,
                            capture_output=True, check=True)
    entries = result.stdout.decode().split('\0')
    files = []
    i = 0
    while i < len(entries):
        entry = entries[i]
        if entry:
            files.append(entry[3:])
            if 'R' in entry[:2] or 'C' in entry[:2]:
                i += 1
                files.append(entries[i])
        i += 1
    return files


def select(files, only, everything):
    if only:
        return sorted(only), "--only"
    if everything or any(f in EVERYTHING for f in files):
        return sorted(BENCHES), "all"
    known = [p for _, prefixes, _, _ in BENCHES.values() for p in prefixes]
    if any(f.startswith(('src/', 'src-tauri/', 'static/', 'scripts/')) and not any(f.startswith(p) for p in known) for f in files):
        return sorted(BENCHES), 'unmapped-conservative'
    picked = [n for n, (_, prefixes, _, _) in BENCHES.items() if any(f.startswith(p) for f in files for p in prefixes)]
    return sorted(picked), "diff"


def build(crates, run_dir):
    if not crates:
        return 0.0
    manifest, seconds = prepare(run_dir / 'prepared')
    os.environ['OMNIGET_PREPARED_TESTS'] = str(manifest)
    return seconds


def run_bench(name, quick, run_dir):
    script, _, quick_args, _ = BENCHES[name]
    out = run_dir / f"{name}.json"
    cmd = [sys.executable, str(ROOT / "scripts" / "llm" / script), "--out", str(out)] + (quick_args if quick else [])
    t0 = time.time()
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    took = time.time() - t0
    (run_dir / f"{name}.log").write_text(p.stdout + "\n--- stderr ---\n" + p.stderr)
    report = json.loads(out.read_text()) if out.exists() else {}
    ok = p.returncode == 0 and report.get("all_pass", p.returncode == 0)
    return {"bench": name, "ok": bool(ok), "seconds": round(took, 1), "json": str(out.relative_to(ROOT)),
            "exit": p.returncode}


def run_b0(run_dir, reps, jobs):
    out = run_dir / "b0.json"
    cmd = [sys.executable, str(ROOT / "scripts" / "llm" / "missions_eval.py"), "--reps", str(reps),
           "--jobs", str(jobs), "--out", str(out)]
    t0 = time.time()
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    (run_dir / "b0.log").write_text(p.stdout + "\n--- stderr ---\n" + p.stderr)
    total = (json.loads(out.read_text()) if out.exists() else {}).get("total", {})
    n, ok_n = total.get("tarefas", 0), total.get("sucesso", 0)
    cost, secs = total.get("mediana_custo_usd"), total.get("mediana_tempo_s")
    within = cost is not None and secs is not None and cost <= B0_COST and secs <= B0_TIME
    tolerated = (cost is not None and secs is not None
                 and cost <= B0_COST * (1 + B0_TOLERANCE) and secs <= B0_TIME * (1 + B0_TOLERANCE))
    ok = bool(n) and ok_n == n and tolerated
    return {"bench": "b0", "ok": ok, "warning": ok and not within, "seconds": round(time.time() - t0, 1), "json": str(out.relative_to(ROOT)),
            "success": f"{ok_n}/{n}", "median_cost_usd": cost, "median_time_s": secs,
            "gate": f"<= US$ {B0_COST} and <= {B0_TIME} s per mission (+{int(B0_TOLERANCE*100)}% tolerated), all succeeded"}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--prepared", help="immutable prepared.json; execute without Cargo")
    ap.add_argument("--all", action="store_true", help="every fake bench, whatever changed")
    ap.add_argument("--quick", action="store_true", help="fewer repetitions (iteration, not a gate)")
    ap.add_argument("--only", default="", help="comma-separated: bn1,bn2,bn3,bn4")
    ap.add_argument("--real", action="store_true", help="also run B0 (real Claude turns; spends quota)")
    ap.add_argument("--real-if-needed", action="store_true",
                    help="run B0 only when the turn argv or the harness changed")
    ap.add_argument("--reps", type=int, default=3, help="B0 repetitions (the gate needs 3)")
    ap.add_argument("--jobs", type=int, default=4, help="B0 missions in parallel")
    ap.add_argument("--dry-run", action="store_true", help="print the selection and stop")
    args = ap.parse_args()

    files = [] if args.only or args.all else changed_files()
    only = {x.strip().lower() for x in args.only.split(",") if x.strip()}
    unknown = only - set(BENCHES)
    if unknown:
        raise SystemExit(f"bench: unknown {sorted(unknown)}; choose from {sorted(BENCHES)}")
    names, why = select(files, only, args.all)
    b0 = args.real or (args.real_if_needed and any(f.startswith(p) for f in files for p in B0_PATHS))
    print(f"bench: {len(files)} changed file(s); fake benches ({why}): {', '.join(names) or 'none'}; "
          f"B0: {'yes' if b0 else 'no'}{' (quick)' if args.quick else ''}", flush=True)
    if args.dry_run:
        return 0

    run_dir = ROOT / "estudos" / "bench" / time.strftime("%Y%m%d-%H%M%S")
    run_dir.mkdir(parents=True, exist_ok=True)
    t_all = time.time()
    if args.prepared:
        os.environ['OMNIGET_PREPARED_TESTS'] = str(pathlib.Path(args.prepared).resolve())
        build_s = 0.0
    else:
        build_s = build({c for n in names for c in BENCHES[n][3]}, run_dir)
    results = []
    # B0 talks to the network and the fake benches to the CPU: run them together.
    with ThreadPoolExecutor(max_workers=len(names) + 1) as pool:
        futures = [pool.submit(run_bench, n, args.quick, run_dir) for n in names]
        if b0:
            futures.append(pool.submit(run_b0, run_dir, args.reps, args.jobs))
        for f in futures:
            r = f.result()
            results.append(r)
            extra = (f"  {r['success']} median ${r['median_cost_usd']} {r['median_time_s']} s"
                     + ("  (above the gate, within tolerance)" if r.get("warning") else "")) if r["bench"] == "b0" else ""
            print(f"  {r['bench']:4} {'PASS' if r['ok'] else 'FAIL'} {r['seconds']:7.1f}s  {r['json']}{extra}", flush=True)
    summary = {"when": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "selection": why, "quick": args.quick,
               "prepared": os.environ.get("OMNIGET_PREPARED_TESTS"), "mode": "iterate",
               "build_s": round(build_s, 1), "wall_s": round(time.time() - t_all, 1),
               "all_pass": all(r["ok"] for r in results), "results": results, "changed_files": files}
    (run_dir / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False))
    print(f"bench: {'ALL PASS' if summary['all_pass'] else 'FAILED'} in {summary['wall_s']} s "
          f"(build {summary['build_s']} s) -> {run_dir.relative_to(ROOT)}/summary.json")
    return 0 if summary["all_pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
