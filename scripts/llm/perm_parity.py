#!/usr/bin/env python3
"""BN8 — permission parity, native x CLI (roadmap item 3.4, S19, 85#P3).

30 rules x the native harness and Claude Code. Goals:
  * the same verdict (allow / ask / deny) in 30/30;
  * `deny fs_write **/.env` bites a Claude `Write` under `acceptEdits`
    20/20 (without the hooks: 0/20, the CLI lets every edit through);
  * p50 of one hook run < 30 ms;
  * (`--real`) a small sample on the real `claude -p` agrees too.

How:
  1. builds `omniget-hook-shim` and copies it to a private folder (the
     cargo target dir is shared: another worktree may rebuild it);
  2. runs the Rust test `claude_hooks::tests::bn8_native_and_cli_give_the_same_verdict`:
     the native verdict comes from the real `ToolBroker::call` (grant, run
     mode, stored rules; an ask shows on the bus), and it writes each case's
     turn folder with the real `claude_hooks::prepare` pointing at the copied
     binary (rows in $OMNIGET_BENCH_OUT);
  3. for each case runs `fake_claude.py` in mode `hooks` with the app's argv
     (`--permission-mode <run mode>`, the prompt tool, `--settings <turn>`):
     the fake runs the hook commands through the shell as Claude Code does,
     applies the mode's contract, and asks the prompt tool (the same gate)
     for what the mode would ask;
  4. the 20 `.env` writes run through the fake with and without `--settings`;
  5. p50 of the hook from 200 direct runs plus every run the fake timed.

`--real` (spends quota, haiku, ~US$ 0.05): 3 cases on the real CLI with
`--permission-prompts none` (deny `.env`, deny `rm`, allow a normal write).

Usage: python3 scripts/llm/perm_parity.py [--fake] [--real] [--out FILE]
"""
from prepared import run_tests as run_prepared_tests
import argparse
import datetime as dt
import json
import os
import pathlib
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
FAKE = ROOT / "scripts" / "llm" / "fake_claude.py"
PROMPT_TOOL = "mcp__omniget__omniget_permission"
FLAG = {"approval_required": "default", "auto_edits": "acceptEdits", "auto": "auto",
        "full_access": "bypassPermissions", None: "acceptEdits"}
ENV_PATHS = [".env", "app/.env", "a/b/.env", "config/.env", "backend/.env", "frontend/.env", "deep/x/y/z/.env",
             "svc/api/.env", "tools/.env", "infra/.env", "web/.env", "server/.env", "db/.env", "mobile/.env",
             "k8s/.env", "docker/.env", "scripts/.env", "test/.env", "etc/.env", "pkg/core/.env"]

def build_shim(dest):
    target = pathlib.Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "src-tauri" / "target")
    subprocess.run(["cargo", "build", "-p", "omniget-cli", "--bin", "omniget-hook-shim"], cwd=ROOT / "src-tauri",
                   check=True, capture_output=True)
    name = "omniget-hook-shim" + (".exe" if os.name == "nt" else "")
    out = pathlib.Path(dest) / name
    shutil.copy2(target / "debug" / name, out)
    return out

def rust_rows(shim, out_file):
    env = dict(os.environ, OMNIGET_BENCH_OUT=str(out_file), OMNIGET_HOOK_BIN=str(shim))
    cmd, p = run_prepared_tests("omniget-core", "claude_hooks::tests::bn8_", env)
    rows = [json.loads(line) for line in out_file.read_text().splitlines()] if out_file.exists() else []
    return rows, {"cmd": " ".join(map(str, cmd)), "exit": p.returncode,
                  "summary": [l for l in p.stdout.splitlines() if l.startswith("test result")],
                  "stderr_tail": p.stderr[-1500:] if p.returncode else ""}

def fake_turn(calls, run_mode, settings, cwd, trace, surface=True):
    argv = [sys.executable, str(FAKE), "-p", "--output-format", "stream-json", "--verbose"]
    argv += (["--permission-prompts", "host", "--permission-prompt-tool", PROMPT_TOOL] if surface
             else ["--permission-prompts", "none"])
    argv += ["--permission-mode", FLAG[run_mode]]
    if settings:
        argv += ["--settings", str(settings)]
    env = dict(os.environ, FAKE_CLAUDE_MODE="hooks", FAKE_CLAUDE_CALLS=json.dumps(calls),
               FAKE_CLAUDE_TRACE=str(trace))
    if trace.exists():
        trace.unlink()
    subprocess.run(argv, input="User: go", cwd=str(cwd), env=env, capture_output=True, text=True, check=True,
                   timeout=120)
    return [json.loads(l) for l in trace.read_text().splitlines()]

def direct_p50(shim, turn, n=200):
    payload = json.dumps({"tool_name": "Write", "tool_input": {"file_path": "/ws/src/main.py", "content": "x"}})
    times = []
    for _ in range(n):
        t = time.perf_counter()
        subprocess.run([str(shim), "hook", "claude", "--event", "PreToolUse", "--turn", str(turn)], input=payload,
                       capture_output=True, text=True, check=True)
        times.append((time.perf_counter() - t) * 1000)
    return times

def real_sample(rows, work):
    """3 cases on the real claude -p (haiku, acceptEdits, no approval surface)."""
    claude = shutil.which("claude") or os.path.expanduser("~/.local/bin/claude")
    env = {k: v for k, v in os.environ.items() if not k.startswith("CLAUDE")}
    by = {r["case"]: r for r in rows}
    plan = [("env-top", "Use the Write tool to create the file .env containing K=1, then say done. Do not use Bash."),
            ("rm-deny", "Use the Bash tool to run exactly this command: rm build/old.txt (it is a scratch file). "
                        "Then say done."),
            ("env-other-file", "Use the Write tool to create the file app/main.py containing print(1), then say done.")]
    out = []
    for case, prompt in plan:
        r = by[case]
        cwd = work / ("real-" + case)
        cwd.mkdir(parents=True)
        (cwd / "build").mkdir()
        (cwd / "build" / "old.txt").write_text("scratch\n")
        argv = [claude, "-p", "--output-format", "stream-json", "--verbose", "--permission-prompts", "none",
                "--permission-mode", FLAG[r["run_mode"]], "--model", "haiku", "--settings", r["settings"],
                "--tools", "Bash,Read,Write,Edit", "--strict-mcp-config"]
        p = subprocess.run(argv, input=prompt, cwd=str(cwd), env=env, capture_output=True, text=True, timeout=300)
        denied, cost, hook_denied, attempted = [], None, False, False
        for line in p.stdout.splitlines():
            try:
                ev = json.loads(line)
            except ValueError:
                continue
            if ev.get("type") == "result":
                denied = ev.get("permission_denials") or []
                cost = ev.get("total_cost_usd")
            if ev.get("type") == "user" and "[OmniGet]" in json.dumps(ev.get("tool_use_result")):
                hook_denied = True
            if ev.get("type") == "assistant":
                for b in (ev.get("message") or {}).get("content") or []:
                    if b.get("type") == "tool_use" and b.get("name") in ("Write", "Bash"):
                        attempted = True
        effect = {"env-top": (cwd / ".env").exists(), "rm-deny": not (cwd / "build" / "old.txt").exists(),
                  "env-other-file": (cwd / "app" / "main.py").exists()}[case]
        cli = "allow" if effect else ("deny" if (attempted and hook_denied) else "not_attempted")
        out.append({"case": case, "native": r["native"], "real_cli": cli, "effect_happened": effect,
                    "attempted": attempted, "denials": len(denied), "hook_said_why": hook_denied, "cost_usd": cost, "exit": p.returncode})
    return out

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fake", action="store_true", default=True)
    ap.add_argument("--real", action="store_true", help="+ 3 real claude -p turns (haiku, spends quota)")
    ap.add_argument("--out", default=str(ROOT / "estudos" / "91-onda3" / "bn8-perm-3.4.json"))
    args = ap.parse_args()

    work = pathlib.Path(tempfile.mkdtemp(prefix="omniget-bn8-"))
    try:
        shim = build_shim(work)
        rows, test_run = rust_rows(shim, work / "bn8.jsonl")
        parity, fake_ms = [], []
        for r in rows:
            cwd = work / ("cwd-" + r["case"])
            cwd.mkdir()
            got = fake_turn([{"name": r["tool"], "input": r["input"]}], r["run_mode"], r["settings"], cwd,
                            work / "trace.jsonl")[0]
            fake_ms.append(got["hook_ms"])
            parity.append({"case": r["case"], "rules": r["rules"], "run_mode": r["run_mode"], "tool": r["tool"],
                           "native": r["native"], "cli": got["verdict"], "hook": got["hook"],
                           "same": r["native"] == got["verdict"]})
        env_row = next((r for r in rows if r["case"] == "env-top"), None)
        env_with = env_without = []
        if env_row:
            calls = [{"name": "Write", "input": {"file_path": f"/ws/{p}", "content": "K=1"}} for p in ENV_PATHS]
            cwd = work / "cwd-env20"
            cwd.mkdir()
            env_with = fake_turn(calls, "auto_edits", env_row["settings"], cwd, work / "t1.jsonl")
            env_without = fake_turn(calls, "auto_edits", None, cwd, work / "t2.jsonl")
            fake_ms += [c["hook_ms"] for c in env_with]
        direct = direct_p50(shim, env_row["turn"]) if env_row else []
        real = real_sample(rows, work) if args.real else None
    finally:
        shutil.rmtree(work, ignore_errors=True)
        for r in locals().get("rows") or []:
            shutil.rmtree(pathlib.Path(r["turn"]).parent, ignore_errors=True)

    same = sum(1 for p in parity if p["same"])
    env_denied = sum(1 for c in env_with if c["verdict"] == "deny")
    env_denied_before = sum(1 for c in env_without if c["verdict"] == "deny")
    p50 = statistics.median(direct) if direct else None
    p50_fake = statistics.median(fake_ms) if fake_ms else None
    goals = [
        {"goal": "same verdict native x CLI (fake CLI running the real hook)", "target": "30/30",
         "value": f"{same}/{len(parity)}", "pass": len(parity) == 30 and same == 30},
        {"goal": "deny fs_write **/.env under acceptEdits", "target": "20/20 (0/20 before)",
         "value": {"with_hooks": f"{env_denied}/{len(env_with)}", "without": f"{env_denied_before}/{len(env_without)}"},
         "pass": len(env_with) == 20 and env_denied == 20 and env_denied_before == 0},
        {"goal": "hook p50 < 30 ms", "target": 30,
         "value": {"direct_p50_ms": round(p50, 2) if p50 else None,
                   "direct_p90_ms": round(sorted(direct)[int(len(direct) * 0.9)], 2) if direct else None,
                   "through_shell_p50_ms": round(p50_fake, 2) if p50_fake else None},
         "pass": p50 is not None and p50 < 30 and p50_fake < 30},
        {"goal": "the Rust half (broker x hook in process, 30 cases) passed", "target": "exit 0",
         "value": test_run["exit"], "pass": test_run["exit"] == 0},
    ]
    if real is not None:
        agree = sum(1 for r in real if r["native"] == r["real_cli"])
        goals.append({"goal": "real claude -p sample agrees", "target": f"{len(real)}/{len(real)}",
                      "value": f"{agree}/{len(real)}", "pass": agree == len(real)})
    report = {
        "bench": "BN8 perm_parity", "item": "3.4", "when": dt.datetime.now().isoformat(timespec="seconds"),
        "level": "native = real ToolBroker::call; CLI = fake_claude running the real hook binary through the "
                 "shell with the app's settings file" + ("; + real claude -p sample" if real is not None else ""),
        "not_measured": "the fake models Claude's mode contract (acceptEdits lets edits through, reads pass, the "
                        "rest asks); the prompt tool is modelled by the hook's PermissionRequest event, which "
                        "calls the same perm::gate as the app's prompt tool",
        "goals": goals, "all_pass": all(g["pass"] for g in goals),
        "parity": parity, "env20": env_with, "real": real, "rust_test": test_run,
    }
    pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.out).write_text(json.dumps(report, ensure_ascii=False, indent=2))
    for g in goals:
        print(("PASS " if g["pass"] else "FAIL ") + g["goal"] + ": " + json.dumps(g["value"], ensure_ascii=False))
    print("all_pass:", report["all_pass"], "->", args.out)
    return 0 if report["all_pass"] else 1

if __name__ == "__main__":
    sys.exit(main())
