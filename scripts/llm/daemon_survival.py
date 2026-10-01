#!/usr/bin/env python3
"""BN6 — the app as a daemon: dead app, cron with no window, kill -9 (3.1, 3.2).

Runs the REAL desktop app (a debug build) on an isolated profile
(`OMNIGET_DATA_DIR` in a temporary folder: own settings, own bridge port,
no single-instance lock) with `scripts/llm/fake_claude.py --mode bn6` as the
agent's Claude CLI, and the real `omniget-cli`. No model, no account, no
network. Every process it starts is tracked by pid and only those are
killed (other sessions run debug builds of the app too: never by name).

Scenarios (`--scenario`, default all):
  * `app_dead` (3.2, 79#P5): the app is not running; `omniget agent run`
    (which waits by default) starts it hidden through the breadcrumb the app
    left (`app-launch.json`), waits for the bridge and completes the job.
    Goals: completes every rep; bridge ready < 10 s (the CLI's own
    measurement, "its bridge answered in N ms"); the window never visible
    (`GET /v1/state` → `app.window_visible`); exit 69 + ERR_APP_NOT_RUNNING
    with `OMNIGET_NO_LAUNCH=1`.
  * `cron` (A8 of BENCH B1): a routine `*/1 * * * *`, saved while the app is
    down, then the app starts hidden (as at login); it fires
    3 times within 3 min (+ one 30 s tick of slack), every run ends well,
    the window is never shown.
  * `kill9` (3.1, 80#P7): a Loop with `auto_resume: true` whose round runs a
    12 s read-only turn. Each rep: `kill -9` the app mid-round (the fake CLI
    survives it, orphaned), reopen the app hidden (as the login item would).
    Goals per rep: the Loop runs again with no click in <= 30 s; the orphan
    of the dead session is gone before the new turn starts (the new fake
    sees no other fake alive: `concurrent == []`); the new turn continues the
    session (`--resume <id>`) with the "check before repeating" note; after
    all reps, no round's effect (ledger line) exists twice. 10 reps.

Phase 2 (3.2b, `omniget-agentd`) is only needed if these fail: the report
says which goal would have asked for it.

Usage:
  python3 scripts/llm/daemon_survival.py [--reps 10] [--scenario app_dead,cron,kill9]
      [--app PATH] [--cli PATH] [--no-build] [--keep] [--out FILE]
Python 3.9+, stdlib only.
"""
import argparse
import datetime as dt
import json
import os
import pathlib
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[2]
TARGET = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", str(pathlib.Path.home() / ".cache/omniget/target")))
FAKE = ROOT / "scripts" / "llm" / "fake_claude.py"
BIN = ".exe" if os.name == "nt" else ""

def now():
    return time.time()

class Lab:
    """One isolated profile, the copied binaries and every pid we started."""

    def __init__(self, app, cli, keep):
        self.dir = pathlib.Path(tempfile.mkdtemp(prefix="omniget-bn6-"))
        self.keep = keep
        self.profile = self.dir / "profile"
        (self.profile / "bin").mkdir(parents=True)
        (self.profile / "llm").mkdir(parents=True)
        self.app = self.dir / ("omniget" + BIN)
        self.cli = self.dir / ("omniget-cli" + BIN)
        shutil.copy2(app, self.app)
        shutil.copy2(cli, self.cli)
        self.trace = self.dir / "trace.jsonl"
        self.ledger = self.dir / "ledger.jsonl"
        self.live = self.dir / "live"
        self.logs = self.dir / "logs"
        self.logs.mkdir()
        self.started_pids = set()
        shim = self.profile / "bin" / "claude"
        shim.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{FAKE}" "$@"\n')
        shim.chmod(0o755)
        (self.profile / "llm" / "accounts.json").write_text(json.dumps([{
            "id": "fake", "cli": "claude", "config_dir": "", "label": "Fake Claude (BN6)",
            "disabled": False, "sandbox": "write"}]))

    def env(self, hold, with_app_path=False, extra=None):
        e = dict(os.environ)
        for k in ("OMNIGET_APP", "OMNIGET_NO_LAUNCH", "OMNIGET_TOKEN", "OMNIGET_JOB_ID", "OMNIGET_PORTABLE"):
            e.pop(k, None)
        e.update({
            "OMNIGET_DATA_DIR": str(self.profile),
            "FAKE_CLAUDE_MODE": "bn6",
            "FAKE_CLAUDE_HOLD_S": str(hold),
            "FAKE_CLAUDE_TRACE": str(self.trace),
            "FAKE_CLAUDE_LEDGER": str(self.ledger),
            "FAKE_CLAUDE_LIVE_DIR": str(self.live),
            "FAKE_CLAUDE_HELP_EXTRA": "--resume --include-partial-messages --system-prompt "
                                      "--tools --no-session-persistence --strict-mcp-config",
            "RUST_LOG": e.get("RUST_LOG", "info"),
        })
        if with_app_path:
            e["OMNIGET_APP"] = str(self.app)
        if extra:
            e.update(extra)
        return e

    def breadcrumb(self):
        try:
            return json.loads((self.profile / "app-launch.json").read_text())
        except (OSError, ValueError):
            return None

    def is_our_app(self, pid):
        """Only a process running our copied binary on our profile."""
        if not pid:
            return False
        out = subprocess.run(["ps", "-p", str(pid), "-o", "command="], capture_output=True, text=True).stdout
        return str(self.app) in out

    def app_pid(self):
        b = self.breadcrumb()
        pid = b and b.get("pid")
        return pid if self.is_our_app(pid) else None

    def kill_app(self, sig=signal.SIGKILL, wait_s=10):
        pid = self.app_pid()
        if not pid:
            return None
        os.kill(pid, sig)
        t0 = now()
        while now() - t0 < wait_s and self.is_our_app(pid):
            time.sleep(0.05)
        return pid

    def open_app(self, hold, tag):
        """Starts the app hidden, as the login item would."""
        log = open(self.logs / f"app-{tag}.log", "w")
        p = subprocess.Popen([str(self.app), "--background"], env=self.env(hold), stdout=log, stderr=log,
                             stdin=subprocess.DEVNULL, start_new_session=True)
        self.started_pids.add(p.pid)
        return p.pid

    def base(self):
        try:
            s = json.loads((self.profile / "settings.json").read_text())
            port = s["app_settings"]["bridge"]["port"]
            return f"http://127.0.0.1:{port}" if port else None
        except (OSError, ValueError, KeyError, TypeError):
            return None

    def token(self):
        try:
            return (self.profile / "llm" / "cli-token").read_text().strip()
        except OSError:
            return None

    def call(self, method, path, body=None, timeout=5):
        base, token = self.base(), self.token()
        if not base or not token:
            raise RuntimeError("no bridge yet")
        req = urllib.request.Request(base + path, method=method,
                                     data=None if body is None else json.dumps(body).encode(),
                                     headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read() or b"null")

    def wait_bridge(self, timeout=30):
        t0 = now()
        while now() - t0 < timeout:
            try:
                self.call("GET", "/v1/state", timeout=2)
                return now() - t0
            except Exception:  # noqa: BLE001 - not up yet
                time.sleep(0.1)
        return None

    def window_visible(self):
        try:
            return bool(self.call("GET", "/v1/state", timeout=3).get("app", {}).get("window_visible"))
        except Exception:  # noqa: BLE001
            return None

    def cli_run(self, args, hold, extra=None, timeout=120):
        t0 = now()
        p = subprocess.run([str(self.cli)] + args, env=self.env(hold, extra=extra), capture_output=True,
                           text=True, timeout=timeout, cwd=str(self.dir))
        b = self.breadcrumb()
        if b and self.is_our_app(b.get("pid")):
            self.started_pids.add(b["pid"])
        return p, now() - t0

    def events(self, tag=None):
        out = []
        try:
            for line in self.trace.read_text().splitlines():
                ev = json.loads(line)
                if tag is None or ev.get("tag") == tag:
                    out.append(ev)
        except (OSError, ValueError):
            pass
        return out

    def cleanup(self):
        pid = self.app_pid()
        if pid:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        for f in (self.live.iterdir() if self.live.exists() else []):
            try:
                os.kill(int(f.name), signal.SIGKILL)
            except (ValueError, ProcessLookupError, PermissionError):
                pass
        for pid in self.started_pids:
            if self.is_our_app(pid):
                os.kill(pid, signal.SIGKILL)
        ps = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
        for line in ps.splitlines():
            pid, _, command = line.strip().partition(" ")
            if str(self.dir) in command and pid.isdigit() and int(pid) != os.getpid():
                try:
                    os.kill(int(pid), signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass
        if not self.keep:
            shutil.rmtree(self.dir, ignore_errors=True)

def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except (ProcessLookupError, PermissionError):
        return False

def prepare(lab):
    """First start (through the CLI, with OMNIGET_APP), the fake agent, then
    the app is killed: the scenarios start from a dead app that left its
    breadcrumb behind."""
    p, took = lab.cli_run(["agent", "status"], hold=1, extra={"OMNIGET_APP": str(lab.app)})
    if p.returncode != 0:
        raise RuntimeError(f"first start failed: {p.stderr[-2000:]}")
    lab.call("POST", "/v1/agents/cli", {"id": "fake", "name": "Fake", "cli": "claude", "account": "fake"})
    lab.kill_app()
    return {"first_start_s": round(took, 2), "cli_stderr": p.stderr.strip()[-300:]}

def scenario_app_dead(lab, reps):
    runs, goals = [], []
    lab.kill_app()
    p, _ = lab.cli_run(["--json", "agent", "status"], hold=1, extra={"OMNIGET_NO_LAUNCH": "1"})
    structured = p.returncode == 69 and "ERR_APP_NOT_RUNNING" in p.stdout and "ERR_APP_NOT_RUNNING" in p.stderr
    for i in range(reps):
        lab.kill_app()
        p, took = lab.cli_run(["agent", "run", "--agent", "fake", "--no-workspace", f"bn6-run hello {i}"], hold=1)
        ready_ms = None
        for line in p.stderr.splitlines():
            if "bridge answered in" in line:
                ready_ms = int(line.rsplit("in", 1)[1].split("ms")[0].strip())
        visible = lab.window_visible()
        states = [ln.strip()[1:-1] for ln in p.stderr.splitlines()
                  if ln.strip().startswith("[") and ln.strip().endswith("]") and " " not in ln.strip()]
        state = states[-1] if states else None
        runs.append({"rep": i, "exit": p.returncode, "state": state, "cli_s": round(took, 2),
                     "bridge_ready_ms": ready_ms, "window_visible": visible,
                     "stderr": p.stderr.strip()[-400:], "stdout": p.stdout.strip()[-300:]})
    ok = [r for r in runs if r["exit"] == 0 and r["state"] in ("done", "done_unreported", "review")]
    ready = [r["bridge_ready_ms"] for r in runs if r["bridge_ready_ms"] is not None]
    goals.append({"goal": "agent run completes with the app dead", "value": f"{len(ok)}/{reps}", "pass": len(ok) == reps})
    goals.append({"goal": "bridge ready < 10 s (CLI's measurement)", "value": ready,
                  "pass": len(ready) == reps and max(ready) < 10_000})
    goals.append({"goal": "the window never visible", "value": [r["window_visible"] for r in runs],
                  "pass": all(r["window_visible"] is False for r in runs)})
    goals.append({"goal": "OMNIGET_NO_LAUNCH=1 → exit 69, ERR_APP_NOT_RUNNING on stdout (--json) and stderr",
                  "value": {"exit": p.returncode, "stdout": p.stdout.strip()[-200:]}, "pass": structured})
    return runs, goals

def scenario_cron(lab, minutes=3):
    lab.kill_app()
    db = sqlite3.connect(str(lab.profile / "llm" / "jobs.db"), timeout=10)
    t_insert = now()
    next_minute_ms = (int(t_insert // 60) + 1) * 60 * 1000
    body = {"id": "tbn6", "name": "bn6 cron", "kind": "cron", "cron": "*/1 * * * *", "agent_id": "fake",
            "prompt": "bn6-cron tick", "enabled": True, "created_ms": int(t_insert * 1000), "tz": "local"}
    db.execute("INSERT OR REPLACE INTO triggers (id, body, next_run_ms) VALUES (?, ?, ?)",
               ("tbn6", json.dumps(body), next_minute_ms))
    db.commit()
    lab.open_app(hold=1, tag="cron")
    lab.wait_bridge()
    deadline = t_insert + minutes * 60 + 35
    visible_seen = []
    fires = []
    while now() < deadline:
        time.sleep(5)
        visible_seen.append(lab.window_visible())
        try:
            jobs = lab.call("GET", "/v1/jobs")
        except Exception:  # noqa: BLE001
            continue
        fires = [j for j in jobs if j.get("trigger_id") == "tbn6"]
        if len(fires) >= minutes and all(j.get("state") in ("done", "done_unreported", "review") for j in fires):
            break
    db.execute("DELETE FROM triggers WHERE id='tbn6'")
    db.commit()
    db.close()
    at = sorted((j["created_ms"] / 1000 - t_insert) for j in fires)
    runs = [{"fires": len(fires), "fired_at_s_after_insert": [round(x, 1) for x in at],
             "states": [j.get("state") for j in fires], "window_visible_samples": visible_seen}]
    goals = [
        {"goal": f"cron */1 fires {minutes}x within {minutes} min (+30 s tick)", "value": len(fires),
         "pass": len(fires) >= minutes and at and at[minutes - 1] <= minutes * 60 + 35},
        {"goal": "every firing ended well", "value": [j.get("state") for j in fires],
         "pass": bool(fires) and all(j.get("state") in ("done", "done_unreported", "review") for j in fires)},
        {"goal": "the window never visible", "value": sorted({str(v) for v in visible_seen}),
         "pass": all(v is False for v in visible_seen)},
    ]
    return runs, goals

def scenario_kill9(lab, reps, hold=12):
    lab.kill_app()
    lab.open_app(hold=hold, tag="kill9-0")
    if lab.wait_bridge() is None:
        raise RuntimeError("the app did not come up")
    loop = lab.call("POST", "/v1/agent/loop", {"agent_id": "fake", "prompt": "bn6-loop keep going",
                                              "max_rounds": 500, "auto_resume": True})
    runs = []
    for i in range(reps):
        t0 = now()
        cur = None
        while now() - t0 < 60:
            starts = [e for e in lab.events("bn6-loop") if e["event"] == "start"]
            ends = {e["pid"] for e in lab.events("bn6-loop") if e["event"] == "end"}
            live = [e for e in starts if e["pid"] not in ends and alive(e["pid"])]
            if live and now() - live[-1]["t"] >= 1.0 and now() - live[-1]["t"] < hold - 6:
                cur = live[-1]
                break
            time.sleep(0.1)
        if not cur:
            runs.append({"rep": i, "error": "no round in flight within 60 s"})
            break
        killed = lab.kill_app()
        t_kill = now()
        orphan_alive = alive(cur["pid"])
        lab.open_app(hold=hold, tag=f"kill9-{i + 1}")
        t_open = now()
        nxt = None
        while now() - t_open < 60:
            later = [e for e in lab.events("bn6-loop") if e["event"] == "start" and e["t"] > t_open]
            if later:
                nxt = later[0]
                break
            time.sleep(0.1)
        rec = {"rep": i, "app_pid_killed": killed, "round": cur["round"], "old_pid": cur["pid"],
               "orphan_alive_after_kill": orphan_alive,
               "orphan_alive_at_new_start": None if not nxt else alive(cur["pid"]),
               "continued_s": None if not nxt else round(nxt["t"] - t_open, 2),
               "kill_to_continue_s": None if not nxt else round(nxt["t"] - t_kill, 2)}
        if nxt:
            rec.update({"new_pid": nxt["pid"], "new_round": nxt["round"], "resumed_session": nxt["resume"],
                        "old_session": cur["session"], "concurrent": nxt["concurrent"],
                        "restart_note": nxt["restart_note"]})
            t1 = now()
            while now() - t1 < hold + 20:
                if any(e["event"] == "end" and e["pid"] == nxt["pid"] for e in lab.events("bn6-loop")):
                    break
                time.sleep(0.2)
        runs.append(rec)
    try:
        lab.call("POST", f"/v1/loops/{loop['id']}/cancel", {})
    except Exception:  # noqa: BLE001 - the app goes down at cleanup anyway
        pass
    effects = {}
    try:
        for line in lab.ledger.read_text().splitlines():
            e = json.loads(line)
            if e.get("tag") == "bn6-loop":
                effects[e["round"]] = effects.get(e["round"], 0) + 1
    except OSError:
        pass
    dup_rounds = {r: n for r, n in effects.items() if n > 1}
    done = [r for r in runs if r.get("continued_s") is not None]
    goals = [
        {"goal": "kill -9 + reopen → the Loop continues with no click in <= 30 s", "value": [r.get("continued_s") for r in runs],
         "pass": len(done) == reps and max(r["continued_s"] for r in done) <= 30},
        {"goal": "0 duplicates: no other fake alive when the new turn starts", "value": [r.get("concurrent") for r in runs],
         "pass": len(done) == reps and all(r["concurrent"] == [] for r in done)},
        {"goal": "0 duplicates: no round's effect written twice", "value": dup_rounds or effects, "pass": not dup_rounds and bool(effects)},
        {"goal": "the orphan of the dead session was alive and got stopped (the reaper, not luck)",
         "value": [(r.get("orphan_alive_after_kill"), r.get("orphan_alive_at_new_start")) for r in runs],
         "pass": len(done) == reps and all(r["orphan_alive_at_new_start"] is False for r in done)
                 and sum(1 for r in done if r["orphan_alive_after_kill"]) >= reps // 2},
        {"goal": "the same round again, --resume of its session, with the check-first note",
         "value": [(r.get("round"), r.get("new_round"), r.get("resumed_session") == r.get("old_session"), r.get("restart_note")) for r in done],
         "pass": len(done) == reps and all(r["new_round"] == r["round"] and r["resumed_session"] == r["old_session"]
                                            and r["restart_note"] for r in done)},
    ]
    return runs, goals

SCENARIOS = {"app_dead": scenario_app_dead, "cron": scenario_cron, "kill9": scenario_kill9}

def build():
    env = dict(os.environ, CARGO_TARGET_DIR=str(TARGET))
    subprocess.run(["cargo", "build", "-p", "omniget", "-p", "omniget-cli"], cwd=ROOT / "src-tauri", env=env, check=True)

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fake", action="store_true", default=True, help="the only mode: fake Claude CLI")
    ap.add_argument("--reps", type=int, default=10, help="kill -9 repetitions (app_dead runs min(reps, 3))")
    ap.add_argument("--scenario", default="", help="comma-separated: " + ",".join(SCENARIOS))
    ap.add_argument("--app", default=str(TARGET / "debug" / ("omniget" + BIN)))
    ap.add_argument("--cli", default=str(TARGET / "debug" / ("omniget-cli" + BIN)))
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--keep", action="store_true", help="keep the temporary profile and logs")
    ap.add_argument("--out", default=str(ROOT / "estudos" / "91-onda3" / "bn6-daemon.json"))
    args = ap.parse_args()
    if os.name == "nt":
        raise SystemExit("BN6 drives signals and ps: macOS and Linux only (Windows is declared, not measured)")
    names = [s.strip() for s in args.scenario.split(",") if s.strip()] or list(SCENARIOS)
    if not args.no_build:
        build()
    lab = Lab(pathlib.Path(args.app), pathlib.Path(args.cli), args.keep)
    report = {"bench": "BN6 daemon_survival", "when": dt.datetime.now().astimezone().isoformat(timespec="seconds"),
              "platform": sys.platform, "profile": str(lab.dir) if args.keep else "(removed)",
              "scenarios": {}, "goals": []}
    t_all = now()
    try:
        report["prepare"] = prepare(lab)
        for name in names:
            t0 = now()
            if name == "app_dead":
                runs, goals = scenario_app_dead(lab, min(args.reps, 3))
            elif name == "kill9":
                runs, goals = scenario_kill9(lab, args.reps)
            else:
                runs, goals = SCENARIOS[name](lab)
            report["scenarios"][name] = {"seconds": round(now() - t0, 1), "runs": runs}
            report["goals"] += [dict(g, scenario=name) for g in goals]
            print(f"{name}: " + ", ".join(f"{'PASS' if g['pass'] else 'FAIL'} {g['goal']}" for g in goals), flush=True)
    except Exception as e:  # noqa: BLE001 - the report says what broke
        report["error"] = repr(e)
    finally:
        lab.cleanup()
    report["wall_s"] = round(now() - t_all, 1)
    report["all_pass"] = "error" not in report and bool(report["goals"]) and all(g["pass"] for g in report["goals"])
    report["phase_2_needed"] = not report["all_pass"]
    pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.out).write_text(json.dumps(report, indent=2, ensure_ascii=False, default=str))
    print("all_pass:", report["all_pass"], "->", args.out)
    return 0 if report["all_pass"] else 1

if __name__ == "__main__":
    sys.exit(main())
