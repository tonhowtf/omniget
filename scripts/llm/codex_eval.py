#!/usr/bin/env python3
"""BN10 — Codex over `codex app-server` (roadmap item 3.6, S21, 82#P7, 81#P7).

Goals:
  * a 10-turn Codex conversation: the input of turn 10 stops growing with
    the history once the thread is resumed (`thread/resume` + only the new
    message) instead of the flattened transcript of `exec` (reference, Claude
    `--resume`: ~125k -> ~32k, BENCH B2);
  * cancel < 2 s (turn/interrupt, then the tree is killed).

Modes:
  --fake (default, CI): Rust tests in omniget-core
    (`cli_runtime::app_server_tests::bn10_` and `cancel_interrupts_`) drive the
    real Coordinator -> CliRuntime -> session (duplex) -> run registry against
    `scripts/llm/fake_codex_app_server.py`, and write one row per turn to
    $OMNIGET_BENCH_OUT: what OmniGet sent (chars) and the fake's usage model
    (1000-token prelude, 4 chars/token, a resumed history is a cached prefix).
    The exec side is the stdin `CliRuntime::plan` builds for the same history.
    Level: our argv/protocol/pipes/registry are real; the tokens are the
    fake's model, not Codex's.
  --real: the same conversation against the installed codex, both
    transports, through the protocol mirror of `missions_eval.py`
    (`codex_app_server_turn` = one app-server per turn with thread/resume;
    `codex_exec_turn` = `exec --json` with `render_prompt` of the history),
    plus a cancel measured on the real app-server. Uses real quota: keep
    --turns small and the cheapest model (`gpt-5.6-luna` by default, the
    cheapest `model/list` offered on 0.158). If the account is out of quota
    the report says so (`blocked`) instead of inventing numbers.

Usage: python3 scripts/llm/codex_eval.py [--fake | --real] [--turns 10]
       [--model M] [--effort low] [--out FILE]
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
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
DEFAULT_OUT = ROOT / "estudos" / "91-onda3" / "bn10-codex-3.6.json"


def question(i):
    # The same text as the Rust bn10 test.
    return (f"Question {i}: summarise in two sentences why incremental builds matter for a Rust "
            f"workspace with {i} crates.")


def run_tests(filter_, out_file):
    env = dict(os.environ, OMNIGET_BENCH_OUT=str(out_file))
    cmd, p = run_prepared_tests("omniget-core", filter_, env)
    summary = [l for l in p.stdout.splitlines() if l.startswith("test result")]
    return {"cmd": " ".join(str(c) for c in cmd), "exit": p.returncode, "summary": summary,
            "stderr_tail": p.stderr[-1500:] if p.returncode else ""}


def goals_of(turns, cancels, series=("app_server", "exec")):
    """The same goals for fake and real rows."""
    by = {r["turn"]: r for r in turns}
    goals = []
    if 2 in by and max(by) >= 10:
        a2, a10 = by[2]["app_server"], by[10]["app_server"]
        e2, e10 = by[2]["exec"], by[10]["exec"]
        goals.append({
            "goal": "app-server: uncached input of turn 10 stops growing (<= 1.5x turn 2)",
            "target": "<= 1.5x", "value": {"turn2": a2["uncached_tokens"], "turn10": a10["uncached_tokens"]},
            "pass": a10["uncached_tokens"] <= 1.5 * max(1, a2["uncached_tokens"])})
        goals.append({
            "goal": "exec (flattened) grows with the history (turn 10 > 3x turn 2), the before",
            "target": "> 3x", "value": {"turn2": e2["uncached_tokens"], "turn10": e10["uncached_tokens"]},
            "pass": e10["uncached_tokens"] > 3 * max(1, e2["uncached_tokens"])})
        goals.append({
            "goal": "turn 10: app-server uncached input below exec's",
            "target": "less", "value": {"app_server": a10["uncached_tokens"], "exec": e10["uncached_tokens"]},
            "pass": a10["uncached_tokens"] < e10["uncached_tokens"]})
        goals.append({
            "goal": "turns 2..10 resume the thread and send only the new message",
            "target": "9/9", "value": sum(1 for r in turns if r["turn"] >= 2 and r.get("resumed")),
            "pass": all(r.get("resumed") for r in turns if r["turn"] >= 2)})
    goals.append({
        "goal": "cancel < 2 s (turn/interrupt, then kill)", "target": "< 2000 ms",
        "value": [{k: c[k] for k in ("server", "ms")} for c in cancels],
        "pass": bool(cancels) and all(c["ms"] < 2000 for c in cancels)})
    return goals


def fake(args):
    with tempfile.TemporaryDirectory() as d:
        out = pathlib.Path(d) / "bn10.jsonl"
        runs = [run_tests("cli_runtime::app_server_tests::bn10_", out),
                run_tests("cli_runtime::app_server_tests::cancel_interrupts_", out)]
        rows = [json.loads(l) for l in out.read_text().splitlines()] if out.exists() else []
    turns = [r for r in rows if r["kind"] == "turn"]
    cancels = [r for r in rows if r["kind"] == "cancel"]
    goals = goals_of(turns, cancels)
    return {
        "mode": "fake",
        "level": ("Coordinator -> CliRuntime -> session (duplex) -> run registry are real; the CLI is "
                  "fake_codex_app_server.py; tokens are the fake's model (1000-token prelude, chars/4, "
                  "resumed history cached)"),
        "runs": runs, "rows": rows, "goals": goals,
        "all_pass": all(g["pass"] for g in goals) and all(r["exit"] == 0 for r in runs),
    }


def real(args):
    import missions_eval as me
    binary = me.codex_binary()
    version = me.codex_version(binary)
    transport = me.codex_transport(binary)
    work = pathlib.Path(tempfile.mkdtemp(prefix="omniget-bn10-"))
    rows, blocked = [], None
    history = []  # (role, text) of the exec series
    thread = None
    for i in range(1, args.turns + 1):
        q = question(i)
        # The app-server series: one process per turn, resumed thread, only the new message.
        a = me.codex_app_server_turn(q, work, "auto_edits", args.model, args.effort, resume=thread,
                                     timeout=args.timeout, binary=binary)
        # The exec series: the flattened transcript on stdin, as the fallback does.
        history.append(("User", q))
        e = me.codex_exec_turn(me.render_prompt(history), work, "auto_edits", args.model, args.timeout, binary)
        history.append(("Assistant", e["text"]))
        err = a["error"] or e["error"]
        if err:
            blocked = {"turn": i, "app_server_error": a["error"], "exec_error": e["error"],
                       "codex_error_info": a.get("error_info")}
            break
        thread = a["thread"] or thread

        def side(r, sent):
            u = r["usage"]
            return {"sent_chars": len(sent), "input_tokens": u["input"], "cached_tokens": u["cached"],
                    "uncached_tokens": u["input"] - u["cached"], "output_tokens": u["output"],
                    "requests": u["requests"], "wall_s": r.get("wall_s")}
        rows.append({"kind": "turn", "turn": i, "resumed": a["resumed"], "fallback": a["fallback"],
                     "app_server": side(a, q), "exec": side(e, me.render_prompt(history[:-1]))})
        print(f"turn {i}: app-server uncached={rows[-1]['app_server']['uncached_tokens']} "
              f"exec uncached={rows[-1]['exec']['uncached_tokens']}", flush=True)
    # Cancel on the real app-server: 1 s after the turn started (before any
    # answer), mirrored from session.rs (turn/interrupt, <= 0.8 s grace, kill).
    c = me.codex_app_server_turn("Count slowly from 1 to 500, one number per line.", work, "auto_edits",
                                 args.model, args.effort, cancel_after=1.0, timeout=60, binary=binary)
    cancels = [{"kind": "cancel", "server": "real", "ms": c["cancel_ms"], "status": c["status"],
                "error": c["error"]}] if c["cancel_ms"] is not None else []
    goals = goals_of(rows, cancels)
    report = {
        "mode": "real", "codex_version": ".".join(map(str, version or ())), "transport": transport,
        "model": args.model, "effort": args.effort, "turns_asked": args.turns, "work": str(work),
        "rows": rows, "cancel": c, "blocked": blocked, "goals": goals,
    }
    if blocked:
        report["all_pass"] = None
        report["not_measured"] = [
            f"turn {blocked['turn']}: Codex refused ({blocked['app_server_error'] or blocked['exec_error']}); "
            "the token goal is not measured on the real service, see the fake run and the Rust tests"]
    else:
        report["all_pass"] = all(g["pass"] for g in goals)
    return report


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fake", action="store_true", default=True)
    ap.add_argument("--real", action="store_true")
    ap.add_argument("--turns", type=int, default=10)
    ap.add_argument("--model", default="gpt-5.6-luna")
    ap.add_argument("--effort", default="low")
    ap.add_argument("--timeout", type=int, default=180)
    ap.add_argument("--out", default=str(DEFAULT_OUT))
    args = ap.parse_args()
    body = real(args) if args.real else fake(args)
    report = {"bench": "BN10 codex_eval (item 3.6)", "when": dt.datetime.now().isoformat(timespec="seconds")}
    report.update(body)
    pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.out).write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    for g in report["goals"]:
        print(("PASS " if g["pass"] else "FAIL ") + g["goal"] + f"  ({g['value']})")
    print("all_pass:", report["all_pass"], "->", args.out)
    return 0 if report["all_pass"] in (True, None) else 1


if __name__ == "__main__":
    sys.exit(main())
