#!/usr/bin/env python3
"""BN12 — run mode matrix (roadmap item 2.7, S13, 82#P5).

4 run modes x 2 CLIs over a fixture that edits a file (`notes.txt`) and runs
`ls`. Goals:
  * questions per mode equal the table of 82#P5: approval_required 2,
    auto_edits 1, auto 0, full_access 0;
  * 0 denials without an explanation (S5): every tool call the CLI denied
    carries OmniGet's reason ("[OmniGet] Denied: ...");
  * plus: `auto` the model refuses (SP2, haiku) still asks and says so; the
    account caps the mode (write -> never bypass, read-only -> plan).

How (`--fake`, the only mode): Rust tests in omniget-core
(`cli_runtime::run_mode_tests::bn12_*`) build the argv with the real
`CliRuntime::plan_with` (run mode under the account's ceiling, the caps of
the installed versions: claude 2.1.284 lists `auto`, codex 0.158 lists
`--approve-for-me`), run `scripts/llm/fake_claude.py` in mode `perm` as the
CLI, and read its stream through the real `ClaudeParser`/`CodexParser`. The
fake decides each call from the permission flags it got, by the contract of
the mode; a question goes to the prompt tool when the argv has one, and is a
denial (Claude's own wording, or a `declined` Codex item) when it has none.
Each row lands in $OMNIGET_BENCH_OUT and is checked here.

Level: argv + real parsers + real pipes; the CLI's behaviour is the fake's
model of the mode contract, not a real claude/codex (see `not_measured`).

Usage: python3 scripts/llm/runmode_matrix.py [--fake] [--out FILE]
"""
from prepared import run_tests as run_prepared_tests
import argparse
import datetime as dt
import json
import os
import pathlib
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
TABLE = {"approval_required": 2, "auto_edits": 1, "auto": 0, "full_access": 0}

def run_tests(crate, filter_, out_file):
    env = dict(os.environ, OMNIGET_BENCH_OUT=str(out_file))
    cmd, p = run_prepared_tests(crate, filter_, env)
    summary = [l for l in p.stdout.splitlines() if l.startswith("test result")]
    return {"cmd": " ".join(str(c) for c in cmd), "exit": p.returncode, "summary": summary,
            "stderr_tail": p.stderr[-1500:] if p.returncode else ""}

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fake", action="store_true", default=True)
    ap.add_argument("--real", action="store_true")
    ap.add_argument("--out", default=str(ROOT / "estudos" / "88-onda2" / "bn12-runmode-2.7.json"))
    args = ap.parse_args()
    if args.real:
        sys.exit("--real needs a prompt-tool MCP server and both CLIs logged in; not offered. Use --fake.")

    with tempfile.TemporaryDirectory() as d:
        out = pathlib.Path(d) / "bn12.jsonl"
        runs = [run_tests("omniget-core", "cli_runtime::run_mode_tests::bn12_", out)]
        rows = [json.loads(l) for l in out.read_text().splitlines()] if out.exists() else []

    matrix = [r for r in rows if r["account"] == "full" and r["model"] == "sonnet"]
    table_rows = [r for r in matrix if r["questions"] == TABLE[r["run_mode"]]]
    unexplained = sum(max(0, r["denials"] - r["explained"]) for r in rows)
    fallback = [r for r in rows if r["model"] == "haiku"]
    capped = [r for r in rows if r["account"] != "full"]
    cells = {(r["cli"], r["run_mode"]) for r in matrix}
    goals = [
        {"goal": "4 modes x 2 CLIs covered", "target": 8, "value": len(cells), "pass": len(cells) == 8},
        {"goal": "questions per mode = 82#P5 table (2/1/0/0)", "target": f"{len(matrix)}/{len(matrix)}",
         "value": f"{len(table_rows)}/{len(matrix)}",
         "pass": bool(matrix) and len(table_rows) == len(matrix)},
        {"goal": "0 denials without explanation", "target": 0,
         "value": {"unexplained": unexplained, "denials": sum(r["denials"] for r in rows)},
         "pass": bool(rows) and unexplained == 0},
        {"goal": "auto refused by the model (haiku) asks and says so", "target": "2 questions + note",
         "value": [{"questions": r["questions"], "note": r["fallback_note"]} for r in fallback],
         "pass": len(fallback) == 1 and fallback[0]["questions"] == 2 and fallback[0]["fallback_note"]},
        {"goal": "the account caps the mode (write: no bypass; read-only: plan)", "target": "no bypass flag",
         "value": [{"cli": r["cli"], "account": r["account"], "argv": r["argv"]} for r in capped],
         "pass": len(capped) == 3 and not any("bypassPermissions" in r["argv"] or "danger-full-access" in r["argv"]
                                              for r in capped)},
    ]
    report = {
        "bench": "BN12 runmode_matrix (item 2.7)",
        "when": dt.datetime.now().isoformat(timespec="seconds"),
        "mode": "fake",
        "level": "argv from CliRuntime::plan_with, real ClaudeParser/CodexParser and pipes; the CLI is fake_claude.py --mode perm",
        "runs": runs,
        "rows": rows,
        "goals": goals,
        "not_measured": [
            "a real claude/codex: whether Claude in auto_edits asks for a read-only `ls` and whether Codex's untrusted policy asks for it (it trusts known-safe commands) is the CLI's call; the fake follows the mode contract",
            "Codex questions reaching a person: `codex exec` has none (A8), they are declined with a reason until 3.6 (app-server)",
        ],
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
