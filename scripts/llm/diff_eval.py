#!/usr/bin/env python3
"""BN4 — job diff benchmark (roadmap items 1.7, shared mode, and 2.4, worktree mode).

Fixtures: the 4 code tasks of `missions_eval.py` (t03-bugfix-py,
t04-feature-py, t05-bugfix-js, t09-rename), seeded by the harness's own
`seed()`, each with a known change that solves it (checked with the task's
own `check()`), plus a 50-file change (`f50`). The known changes cover every
kind of entry: edits (all), an added file (t04, f50), a deleted file (t05,
f50) and a binary file (a `.pyc` in t03).

Goals:
  * the job's diff == `diff -ruN seed/ final/`, normalized (per file: the
    hunk ranges and lines byte for byte, or "binary"; file headers and git's
    hunk section heading dropped), in 4/4 code tasks (and in f50);
  * the job's patch, applied with `git apply` to a copy of the seed,
    reproduces the final folder byte for byte (binary files excluded: the
    patch carries them as one "Binary files differ" line, by design);
  * `job_diff` with 50 files < 200 ms, and the stored read behind
    `GET /v1/jobs/{id}/diff` < 200 ms;
  * 100% of the writing jobs have a non-empty `files`;
  * controls: a job that wrote nothing is available and empty; a job whose
    folder has no snapshot is "unavailable", never an empty diff;
  * the turn's argv is unchanged (cost per task = B0: the diff is host-side
    post-processing).

Worktree mode (2.4): the same fixtures, each seed made a git repository, the
job run in a worktree of it (`vcs::job_worktree`), and the diff taken by
`snapshot::worktree_diff` = merge-base(base, HEAD)..worktree, first with the
work uncommitted (private index), then after it is committed on the job's
branch (what `run_job` does). Same goals, plus: both diffs identical and
`head_oid` = the commit on the branch in 4/4.

How (`--fake`, the only mode): the Rust test `core::llm::snapshot::bn4::*`
(omniget-core) runs `fake_claude.py` in `apply` mode through the real
`spawn_turn_with` + `ClaudeParser`, bracketed by the real snapshot the way the
Coordinator brackets a CLI turn, then the real `job_diff`, and leaves
`work/`, `job.patch` and `job.json` next to each fixture; this script compares
them with `diff -ruN` and `git apply`. `jobs::bn4_read` (omniget) times the
stored read. Level: e2e from the turn down (process, parser, shadow-git
snapshot, job_diff); NOT through `Jobs::run_job`/the Tauri app (headless is
not feasible here) — `run_job` calls the same `job_diff` after the stream
ends (unit-tested in `jobs::diff_tests`); the HTTP route is the measured
`diff_view` without axum and the socket.

Usage: python3 scripts/llm/diff_eval.py [--fake] [--reps 20] [--out FILE]
"""
from prepared import run_tests as run_prepared_tests
import argparse
import base64
import datetime as dt
import importlib.util
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
TAURI = ROOT / "src-tauri"
CODE_TASKS = ["t03-bugfix-py", "t04-feature-py", "t05-bugfix-js", "t09-rename"]


def load_missions_eval():
    spec = importlib.util.spec_from_file_location("missions_eval", ROOT / "scripts" / "llm" / "missions_eval.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# ── the known changes (each one solves its task) ───────────────────────

PYC = base64.b64encode(bytes([0x6F, 0x0D, 0x0D, 0x0A, 0, 0, 0, 0]) + bytes(range(256)) * 2).decode()

KNOWN = {
    "t03-bugfix-py": {
        "write": {
            "stats.py": (
                "def mean(xs):\n"
                "    if not xs:\n"
                "        raise ValueError(\"mean of an empty list\")\n"
                "    return sum(xs) / len(xs)\n"
                "\n\n"
                "def median(xs):\n"
                "    s = sorted(xs)\n"
                "    n = len(s)\n"
                "    if n % 2:\n"
                "        return s[n // 2]\n"
                "    return (s[n // 2 - 1] + s[n // 2]) / 2\n"
                "\n\n"
                "def mode(xs):\n"
                "    counts = {}\n"
                "    for x in xs:\n"
                "        counts[x] = counts.get(x, 0) + 1\n"
                "    return max(counts, key=counts.get)\n"
            ),
        },
        # What `python3 -m unittest` leaves behind: a binary file.
        "write_b64": {"__pycache__/stats.cpython-312.pyc": PYC},
    },
    "t04-feature-py": {
        "write": {
            "slug.py": (
                "import re\n"
                "import unicodedata\n"
                "\n\n"
                "def slugify(texto):\n"
                "    base = unicodedata.normalize(\"NFKD\", texto).encode(\"ascii\", \"ignore\").decode(\"ascii\")\n"
                "    base = re.sub(r\"[^a-zA-Z0-9]+\", \"-\", base.lower())\n"
                "    return base.strip(\"-\")\n"
            ),
            "slug_cli.py": (
                "import sys\n"
                "\n"
                "from slug import slugify\n"
                "\n"
                "if __name__ == \"__main__\":\n"
                "    print(slugify(\" \".join(sys.argv[1:])))\n"
            ),
        },
    },
    "t05-bugfix-js": {
        "edit": {
            "src/queue.js": [
                ("if (this.items[i].priority <= this.items[best].priority) best = i;",
                 "if (this.items[i].priority < this.items[best].priority) best = i;"),
                ("return this.items.splice(best, 1)[0];", "return this.items.splice(best, 1)[0].value;"),
                ("get size() { return this.items.length - 1; }", "get size() { return this.items.length; }"),
            ],
        },
        # node runs CommonJS without it: a deleted file.
        "delete": ["package.json"],
    },
    "t09-rename": {
        "edit": {
            "shop/pricing.py": [("def calc_total(", "def compute_order_total(")],
            "shop/cart.py": [("import calc_total", "import compute_order_total"),
                             ("return calc_total(", "return compute_order_total(")],
            "shop/checkout.py": [("pricing.calc_total(", "pricing.compute_order_total(")],
        },
    },
}


def changes_for(name, seed):
    """`changes.json` for fake_claude --mode apply: edits become whole files."""
    spec = KNOWN[name]
    out = {"write": dict(spec.get("write", {})), "write_b64": dict(spec.get("write_b64", {})),
           "delete": list(spec.get("delete", []))}
    for rel, subs in spec.get("edit", {}).items():
        text = (seed / rel).read_text(encoding="utf-8")
        for old, new in subs:
            assert old in text, (name, rel, old)
            text = text.replace(old, new)
        out["write"][rel] = text
    return out


def f50_fixture(root):
    """60 files; 40 edited, 5 added, 5 deleted: 50 changed."""
    seed = root / "f50" / "seed"
    write, delete = {}, []
    for i in range(60):
        rel = f"src/m{i % 6:02d}/f{i:02d}.txt"
        body = "".join(f"file {i} line {l}\n" for l in range(40))
        (seed / rel).parent.mkdir(parents=True, exist_ok=True)
        (seed / rel).write_bytes(body.encode("utf-8"))
        if i < 40:
            write[rel] = body.replace(" line 7\n", " line 7 (edited)\n").replace(" line 30\n", "")
        elif i < 45:
            delete.append(rel)
    for i in range(5):
        write[f"new/n{i}.txt"] = f"new file {i}\nsecond line\n"
    (root / "f50" / "changes.json").write_text(json.dumps({"write": write, "delete": delete}))


# ── comparison ─────────────────────────────────────────────────────────

HEADER = ("--- ", "+++ ", "index ", "new file mode", "deleted file mode", "old mode", "new mode",
          "similarity", "rename ", "Only in ")


def normalize(text, kind, work="work"):
    """{path: [hunk lines] | "BINARY"} from a unified diff. `kind`: "git"
    (`diff --git a/x b/x`) or "bsd" (`diff -ruN seed/x work/x`).

    Normalized away: the file headers (`diff`, `index`, `---`/`+++` with
    their timestamps, mode lines) and the section heading git adds after a
    hunk's `@@ … @@`. Kept byte for byte: every hunk range and every
    context, added and removed line (and `\ No newline at end of file`)."""
    files, cur, in_hunk = {}, None, False
    for line in text.splitlines():
        if kind == "git" and line.startswith("diff --git "):
            cur = line.split(" b/", 1)[1]
            files[cur], in_hunk = [], False
            continue
        if kind == "bsd" and line.startswith("diff -ruN "):
            cur = line.split(f" {work}/", 1)[1]
            files[cur], in_hunk = [], False
            continue
        if line.startswith("Binary files "):
            m = re.match(r"Binary files (.+) and (.+) differ", line)
            path = m.group(2) if m.group(2) != "/dev/null" else m.group(1)
            path = re.sub(rf"^(b/|a/|{re.escape(work)}/|seed/)", "", path)
            files[path] = "BINARY"
            cur, in_hunk = None, False
            continue
        if cur is None:
            continue
        if not in_hunk and line.startswith(HEADER):
            continue
        if line.startswith("@@"):
            in_hunk = True
            # git appends the enclosing function to the header
            # (`@@ -9,4 +9,4 @@ class Cart:`); BSD/GNU diff -u does not.
            # Metadata, not content: compared without it.
            line = re.match(r"(@@ -\S+ \+\S+ @@)", line).group(1)
        if in_hunk:
            files[cur].append(line)
    return {p: v for p, v in files.items() if v}


def tree(root):
    out = {}
    for p in sorted(root.rglob("*")):
        if p.is_file():
            out[str(p.relative_to(root))] = p.read_bytes()
    return out


def compare(fx, tmp, suffix=""):
    """`suffix` "" = shared mode (`work/`, `job.*`), "-wt" = worktree mode."""
    work_name = "work" + suffix
    seed, work = fx / "seed", fx / work_name
    job = json.loads((fx / f"job{suffix}.json").read_text())
    patch = (fx / f"job{suffix}.patch").read_text(encoding="utf-8")
    ref = subprocess.run(["diff", "-ruN", "seed", work_name], cwd=fx, capture_output=True, text=True).stdout
    a, b = normalize(patch, "git"), normalize(ref, "bsd", work_name)
    mismatched = sorted(p for p in set(a) | set(b) if a.get(p) != b.get(p))
    binary = sorted(f["path"] for f in job["files"] if f.get("binary"))
    # The patch applied to a copy of the seed must give the final folder.
    copy = tmp / (fx.name + suffix + "-applied")
    shutil.copytree(seed, copy)
    cmd = ["git", "apply", "--whitespace=nowarn"] + [f"--exclude={p}" for p in binary] + [str(fx / f"job{suffix}.patch")]
    applied = subprocess.run(cmd, cwd=copy, capture_output=True, text=True) if patch else None
    got, want = tree(copy), tree(work)
    for p in binary:
        got.pop(p, None)
        want.pop(p, None)
    bytes_equal = got == want and (applied is None or applied.returncode == 0)
    # Statuses against the folders.
    s_tree, w_tree = tree(seed), tree(work)
    expected = {}
    for p in set(s_tree) | set(w_tree):
        if p not in s_tree:
            expected[p] = "A"
        elif p not in w_tree:
            expected[p] = "D"
        elif s_tree[p] != w_tree[p]:
            expected[p] = "M"
    statuses = {f["path"]: f["status"] for f in job["files"]}
    return {
        "files": len(job["files"]),
        "additions": job["additions"],
        "deletions": job["deletions"],
        "binary_files": binary,
        "statuses_ok": statuses == expected,
        "normalized_equal": not mismatched,
        "mismatched_paths": mismatched,
        "patch_applies_byte_for_byte": bytes_equal,
        "git_apply_stderr": (applied.stderr[-400:] if applied is not None and applied.returncode else ""),
        "job_diff_ms_p50": job["job_diff_ms_p50"],
        "job_diff_ms_max": job["job_diff_ms_max"],
        "turn_ok": job["turn_ok"],
        "clipped": job["clipped"],
        "head_oid": job.get("head_oid"),
        "same_uncommitted_and_committed": job.get("same_uncommitted_and_committed"),
    }


def run_tests(crate, filter_, env_extra, out_file):
    env = dict(os.environ, OMNIGET_BENCH_OUT=str(out_file), **env_extra)
    cmd, p = run_prepared_tests(crate, filter_, env)
    summary = [l for l in p.stdout.splitlines() if l.startswith("test result") or l.startswith("test ")]
    return {"cmd": " ".join(f"{k}={v}" for k, v in env_extra.items()) + " " + " ".join(cmd),
            "exit": p.returncode, "summary": summary, "stderr_tail": p.stderr[-2000:] if p.returncode else ""}


def read_metrics(path):
    per = {}
    if path.exists():
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
    ap.add_argument("--reps", type=int, default=20, help="job_diff timing repetitions per fixture")
    ap.add_argument("--out", default=str(ROOT / "estudos" / "87-onda1" / "bn4-diff.json"))
    args = ap.parse_args()
    if args.real:
        sys.exit("--real is not offered: the diff is host-side post-processing and never changes the turn, "
                 "so B0 (missions_eval.py --reps 3) stays the cost check. Use --fake.")
    me = load_missions_eval()
    tasks = {t.id: t for t in me.TASKS}
    with tempfile.TemporaryDirectory(prefix="bn4-") as d:
        tmp = pathlib.Path(d)
        fixtures = tmp / "fixtures"
        for name in CODE_TASKS:
            seed = fixtures / name / "seed"
            seed.mkdir(parents=True)
            tasks[name].seed(seed)
            (fixtures / name / "changes.json").write_text(json.dumps(changes_for(name, seed)))
        f50_fixture(fixtures)
        out = tmp / "bn4.jsonl"
        core = run_tests("omniget-core", "core::llm::snapshot::bn4::",
                         {"BN4_FIXTURES": str(fixtures), "BN4_TIMING_REPS": str(args.reps)}, out)
        app = run_tests("omniget", "jobs::bn4_read::", {}, out)
        m = read_metrics(out)
        per = {}
        per_wt = {}
        solved = {}
        for name in CODE_TASKS + ["f50"]:
            fx = fixtures / name
            if not (fx / "job.json").exists():
                per[name] = {"error": "the Rust harness left no job.json"}
                continue
            per[name] = compare(fx, tmp)
            per_wt[name] = (compare(fx, tmp, "-wt") if (fx / "job-wt.json").exists()
                            else {"error": "the Rust harness left no job-wt.json"})
            if name in tasks:
                ok, detail = tasks[name].check(fx / "work")
                solved[name] = {"ok": bool(ok), "detail": detail}
    diff = subprocess.run(
        ["git", "diff", "--stat", "HEAD", "--",
         "src-tauri/omniget-core/src/core/llm/cli_runtime/claude.rs",
         "src-tauri/omniget-cli/src/commands/claude.rs",
         "scripts/llm/missions_eval.py"],
        cwd=ROOT, capture_output=True, text=True).stdout.strip()

    code = [per.get(n, {}) for n in CODE_TASKS]
    norm = sum(1 for r in code if r.get("normalized_equal"))
    applied = sum(1 for r in code if r.get("patch_applies_byte_for_byte"))
    statuses = sum(1 for r in code if r.get("statuses_ok"))
    f50 = per.get("f50", {})
    writing = m.get("writing_jobs") or 0
    with_files = m.get("writing_jobs_with_files") or 0
    wt = [per_wt.get(n, {}) for n in CODE_TASKS]
    wt_norm = sum(1 for r in wt if r.get("normalized_equal"))
    wt_applied = sum(1 for r in wt if r.get("patch_applies_byte_for_byte"))
    wt_status = sum(1 for r in wt if r.get("statuses_ok"))
    wt_head = sum(1 for r in wt if r.get("head_oid") and r.get("same_uncommitted_and_committed"))
    wt_f50 = per_wt.get("f50", {})
    goals = [
        goal("job diff == diff -ruN seed/ final/ (normalized), code tasks", f"{norm}/4", norm == 4, "4/4"),
        goal("patch applied to the seed == final folder, byte for byte (text files)", f"{applied}/4", applied == 4, "4/4"),
        goal("A/M/D of every file matches the folders", f"{statuses}/4", statuses == 4, "4/4"),
        goal("f50: diff -ruN and git apply agree too", {k: f50.get(k) for k in ("files", "normalized_equal", "patch_applies_byte_for_byte")},
             f50.get("files") == 50 and f50.get("normalized_equal") and f50.get("patch_applies_byte_for_byte"), "50 files, equal"),
        goal("job_diff with 50 files", {"p50_ms": f50.get("job_diff_ms_p50"), "max_ms": f50.get("job_diff_ms_max")},
             (f50.get("job_diff_ms_max") or 1e9) < 200, "< 200 ms (max of the reps)"),
        goal("stored read behind GET /v1/jobs/{id}/diff, 50 files",
             {"p50_ms": m.get("stored_read_ms_p50"), "max_ms": m.get("stored_read_ms_max")},
             (m.get("stored_read_ms_max") or 1e9) < 200, "< 200 ms"),
        goal("writing jobs with a non-empty files list", f"{with_files}/{writing}",
             writing > 0 and with_files == writing, "100%"),
        goal("binary counted as binary (t03 .pyc)", per.get("t03-bugfix-py", {}).get("binary_files"),
             per.get("t03-bugfix-py", {}).get("binary_files") == ["__pycache__/stats.cpython-312.pyc"], "1 binary file"),
        goal("control: a job that wrote nothing is available and empty", m.get("control_readonly_available_files"),
             (m.get("control_readonly_available_files") or {}).get("files") == 0, "0 files, available"),
        goal("control: no snapshot -> unavailable, never empty", m.get("control_nosnap"),
             m.get("control_nosnap") == "ERR_DIFF_UNAVAILABLE", "ERR_DIFF_UNAVAILABLE"),
        goal("the known changes solve their tasks (missions_eval check)", {k: v["ok"] for k, v in solved.items()},
             len(solved) == 4 and all(v["ok"] for v in solved.values()), "4/4"),
        goal("worktree: diff == diff -ruN seed/ final/ (normalized), code tasks", f"{wt_norm}/4", wt_norm == 4, "4/4"),
        goal("worktree: patch applied to the seed == final folder (text files)", f"{wt_applied}/4", wt_applied == 4, "4/4"),
        goal("worktree: A/M/D of every file matches the folders", f"{wt_status}/4", wt_status == 4, "4/4"),
        goal("worktree: same diff uncommitted and committed, head_oid = the branch commit", f"{wt_head}/4",
             wt_head == 4, "4/4"),
        goal("worktree: f50 diff -ruN and git apply agree",
             {k: wt_f50.get(k) for k in ("files", "normalized_equal", "patch_applies_byte_for_byte")},
             wt_f50.get("files") == 50 and wt_f50.get("normalized_equal") and wt_f50.get("patch_applies_byte_for_byte"),
             "50 files, equal"),
        goal("worktree: worktree_diff with 50 files",
             {"p50_ms": wt_f50.get("job_diff_ms_p50"), "max_ms": wt_f50.get("job_diff_ms_max")},
             (wt_f50.get("job_diff_ms_max") or 1e9) < 200, "< 200 ms (max of the reps)"),
        goal("worktree: writing jobs with a non-empty files list",
             f"{m.get('wt_writing_jobs_with_files')}/{m.get('wt_writing_jobs')}",
             (m.get("wt_writing_jobs") or 0) > 0 and m.get("wt_writing_jobs_with_files") == m.get("wt_writing_jobs"),
             "100%"),
        goal("argv of the turn unchanged (cost per task = B0)", diff or "(empty)", not diff,
             "empty diff of cli_runtime/claude.rs, omniget-cli claude.rs, missions_eval.py"),
    ]
    report = {
        "bench": "BN4 diff_eval",
        "when": dt.datetime.now().isoformat(timespec="seconds"),
        "mode": "fake",
        "level": {
            "e2e": "fake CLI process (fake_claude.py --mode apply) -> spawn_turn_with -> ClaudeParser; "
                   "shadow-git snapshot before/after like the Coordinator; snapshot::job_diff; "
                   "compared here with `diff -ruN` and `git apply`",
            "unit": "the stored read behind GET /v1/jobs/{id}/diff (row + patch file + diff_view, no axum/socket); "
                    "run_job -> job_diff -> columns (jobs::diff_tests); MCP/CLI shapes",
            "not_run": "Jobs::run_job inside the Tauri app (headless not feasible here)",
        },
        "platform": sys.platform,
        "params": {"timing_reps": args.reps, "reference": "diff -ruN seed work (" + subprocess.run(
            ["diff", "--version"], capture_output=True, text=True).stdout.splitlines()[0] + ")"},
        "not_measured": {"cost_per_task": "unchanged by construction (host-side post-processing; argv goal); "
                                          "missions_eval.py --real not rerun on purpose"},
        "runs": {"core": core, "app": app},
        "per_fixture": per,
        "per_fixture_worktree": per_wt,
        "tasks_solved": solved,
        "metrics": m,
        "goals": goals,
        "all_pass": all(g["pass"] for g in goals) and core["exit"] == 0 and app["exit"] == 0,
    }
    pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.out).write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    for g in goals:
        print(("PASS " if g["pass"] else "FAIL ") + g["goal"] + f"  ({g['value']})")
    print("all_pass:", report["all_pass"], "->", args.out)
    return 0 if report["all_pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
