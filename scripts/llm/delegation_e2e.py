#!/usr/bin/env python3
"""BN5 — external delegation, end to end (roadmap items 2.1 and 2.5).

Scenarios (`--scenario`, default: every one implemented):
  * `auth` (2.1): who may use the agent routes of the local bridge.
      - the browser extension's key: works during the grace period (with a
        `Deprecation` header and the use recorded for the UI), 403 once the
        grace flag is off, and 403 on `/v1/agents/cli` even during grace;
      - a user token with `jobs:read` only: 403 on start and on agent
        creation; with `jobs:operate`: 200 (`/v1/agents/cli` requires it);
      - a job's own token (`OMNIGET_JOB_TOKEN`): 200 on its job, 403 on
        another job and on the global routes, 401 after the job ends;
      - revoked/expired/unknown tokens: 401;
      - 0 calls blocking > 2 s;
      - the child of a turn has no bridge bearer in its environment, under
        any variable name, and does get its job token.
  * `delegate` (2.5): without a window, over MCP (`agent_run -> job_wait ->
    job_get.pendingApprovals -> approval_respond -> job_wait -> job_diff`)
    and over HTTP (`POST /v1/commands` job.start -> `GET /v1/jobs/{id}/wait`
    -> `approval.respond` -> wait): 10/10 each; the same command_id 5x -> 1
    job on both (the copies flagged `Idempotent-Replayed`), another payload
    under that id -> 409; a reader token, the extension key and a request
    from a job (`X-Omniget-Job`) cannot start one; 0 non-wait calls > 2 s.
    Level: `job_commands::tests::bn5_delegate_end_to_end_without_a_window`
    (omniget): the production command code, receipts table, wait, HTTP
    handlers behind the production guard and MCP tool bodies, in front of a
    stand-in engine that runs a scripted turn on a real broker and run
    record (ask shell_exec -> answer -> write a.txt -> review with OUTCOME).
    The real-agent end to end (<= 10.4 s + 1 s with Claude) is B0's number:
    the delegated job is the same turn as a mission job.

  * `workflow` (3.10): `.omniget/workflow.md` in a repository.
      - "delegue a correção X" with no flag at all (MCP `agent_run`,
        `/v1/commands`, the CLI's body from a subfolder): 5/5 jobs get the
        workflow's agent, run mode, isolation, deadline and rendered prompt;
      - a flag the caller gives wins; `workflow: false` opts out;
      - an invalid reload keeps the previous version in effect (the job says
        `from_last_good` and carries the error); with no earlier valid
        version the start is refused with `ERR_WORKFLOW_INVALID`;
      - an untrusted (changed) file never loosens the run mode;
      - hooks: untrusted never run (0 runs), trusted run once per worktree, a
        failing `before_remove` keeps the worktree, a changed file stops them
        again; checks land in the proof only when trusted;
      - `job_report` answers in one call, without host paths, only to the
        client that started the job, behind the deny-by-default grant.
    Level: `job_commands::tests::bn5_workflow_delegation_without_flags` (the
    production `Commands::execute` and MCP tool bodies over the stand-in
    engine, a real workflow store) and `job_workflow::tests` (real git
    repositories and worktrees, real hook processes through
    `jobs::remove_job_worktree_with`), plus the parser/store/runner tests of
    `omniget-core` (`llm::workflow`).

How the grace period is tested: the flag (`bridge.legacy_agent_routes`,
default on in this version) is flipped inside the test on the same running
server; on = 200 + `Deprecation: true`, off = 403. The next version flips the
default and the case "after grace" becomes the only behavior.

How (`--fake`, the only mode for `auth`): the Rust test
`bridge_tokens::bn5::bn5_auth` (omniget) serves the production guard
(`bridge_tokens::guard`, the same `route_layer` as `local_bridge_jobs`) over
real HTTP on 127.0.0.1 in front of stand-in handlers laid out like the agent
routes, with a real `jobs.db` token table (in memory) and the same
`JobTokenLease` `run_job` holds; every case is one `reqwest` call.
`cli_runtime::session::tests::the_child_env_has_no_bridge_bearer`
(omniget-core) spawns a real child through `spawn_turn` and greps its `env`.
Level: HTTP + guard + token store + lease, not the Tauri app (the real
handlers need an AppHandle); `every_agent_route_has_a_scope` checks that each
route of `local_bridge_jobs.rs` is mapped and that the router carries the
layer.

Extending: add a scenario function to SCENARIOS returning (runs, goals,
detail); keep one JSON report.

Usage: python3 scripts/llm/delegation_e2e.py [--fake] [--scenario auth] [--out FILE]
"""
from prepared import run_tests as run_prepared_tests
import argparse
import datetime as dt
import json
import pathlib
import os
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]

def run_tests(crate, filter_, out_file):
    env = dict(os.environ, OMNIGET_BENCH_OUT=str(out_file))
    cmd, p = run_prepared_tests(crate, filter_, env)
    summary = [l for l in p.stdout.splitlines() if l.startswith("test result")]
    return {"cmd": " ".join(cmd), "exit": p.returncode, "summary": summary,
            "stderr_tail": p.stderr[-1500:] if p.returncode else ""}

def read_lines(path):
    if not path.exists():
        return []
    return [json.loads(l) for l in path.read_text().splitlines() if l.strip()]

AUTH_CASES = {
    "extension_grace_run": 200, "extension_grace_jobs": 200, "extension_grace_agents_cli": 403,
    "extension_after_grace_run": 403, "extension_after_grace_jobs": 403, "extension_after_grace_cancel": 403,
    "no_token": 401, "unknown_token": 401,
    "reader_lists": 200, "reader_runs": 403, "reader_creates_agent": 403,
    "operator_creates_agent": 200, "operator_runs": 200,
    "job_token_own_job": 200, "job_token_own_diff": 200, "job_token_other_job": 403,
    "job_token_other_explain": 403, "job_token_global_list": 403, "job_token_starts_job": 403,
    "job_token_cancels_itself": 403, "job_token_after_end": 401,
    "user_token_before_expiry": 200, "user_token_after_expiry": 401,
}

def scenario_auth():
    runs = []
    with tempfile.TemporaryDirectory() as d:
        out = pathlib.Path(d) / "bn5.jsonl"
        runs.append(run_tests("omniget", "bridge_tokens::", out))
        runs.append(run_tests("omniget-core", "cli_runtime::session::tests::the_child_env_has_no_bridge_bearer", out))
        lines = [l for l in read_lines(out) if l.get("scenario") == "auth"]
    cases = {l["case"]: l for l in lines if "case" in l}
    metrics = {l["metric"]: l for l in lines if "metric" in l}
    status = {c: cases.get(c, {}).get("status") for c in AUTH_CASES}

    def all_of(names):
        return all(status[n] == AUTH_CASES[n] for n in names)

    def val(metric):
        return metrics.get(metric, {}).get("value")

    goals = [
        {"goal": "extension key works during grace, with Deprecation header, recorded for the UI",
         "target": "200 + deprecation; last use seen",
         "value": {"run": status["extension_grace_run"], "jobs": status["extension_grace_jobs"],
                   "deprecation": cases.get("extension_grace_run", {}).get("deprecation"),
                   "seen": val("legacy_use_seen_by_ui")},
         "pass": all_of(["extension_grace_run", "extension_grace_jobs"])
         and cases.get("extension_grace_run", {}).get("deprecation") is True and val("legacy_use_seen_by_ui") is not None},
        {"goal": "extension key -> 403 on agent routes after the grace flag", "target": 403,
         "value": {k: status[k] for k in ["extension_after_grace_run", "extension_after_grace_jobs", "extension_after_grace_cancel"]},
         "pass": all_of(["extension_after_grace_run", "extension_after_grace_jobs", "extension_after_grace_cancel"])},
        {"goal": "/v1/agents/cli requires jobs:operate (extension key refused even during grace)",
         "target": "403 extension / 403 jobs:read / 200 jobs:operate",
         "value": {k: status[k] for k in ["extension_grace_agents_cli", "reader_creates_agent", "operator_creates_agent"]},
         "pass": all_of(["extension_grace_agents_cli", "reader_creates_agent", "operator_creates_agent"])},
        {"goal": "job token on another job -> 403 (and on global routes, and it cannot operate)", "target": 403,
         "value": {k: status[k] for k in ["job_token_own_job", "job_token_other_job", "job_token_other_explain",
                                          "job_token_global_list", "job_token_starts_job", "job_token_cancels_itself"]},
         "pass": all_of(["job_token_own_job", "job_token_own_diff", "job_token_other_job", "job_token_other_explain",
                         "job_token_global_list", "job_token_starts_job", "job_token_cancels_itself"])},
        {"goal": "job token after the job ends -> 401; its env entry is gone", "target": 401,
         "value": {"status": status["job_token_after_end"], "env_cleared": val("job_env_cleared_at_end")},
         "pass": all_of(["job_token_after_end"]) and val("job_env_cleared_at_end") is True},
        {"goal": "scopes, expiry and unknown tokens", "target": "per case",
         "value": {k: status[k] for k in ["no_token", "unknown_token", "reader_lists", "reader_runs", "operator_runs",
                                          "user_token_before_expiry", "user_token_after_expiry"]},
         "pass": all_of(["no_token", "unknown_token", "reader_lists", "reader_runs", "operator_runs",
                         "user_token_before_expiry", "user_token_after_expiry"])},
        {"goal": "0 calls blocking > 2 s", "target": "< 2000 ms",
         "value": val("slowest_call_ms"), "pass": val("slowest_call_ms") is not None and val("slowest_call_ms") < 2000},
        {"goal": "child env has no bridge bearer (grep of `env`)", "target": 0,
         "value": {"hits": val("child_env_bearer_hits"), "env_lines": metrics.get("child_env_bearer_hits", {}).get("env_lines")},
         "pass": val("child_env_bearer_hits") == 0},
    ]
    missing = [c for c in AUTH_CASES if c not in cases]
    if missing:
        goals.append({"goal": "every auth case ran", "target": len(AUTH_CASES), "value": {"missing": missing}, "pass": False})
    return runs, goals, {"cases": cases, "metrics": {k: v.get("value") for k, v in metrics.items()}}

def scenario_delegate():
    runs = []
    with tempfile.TemporaryDirectory() as d:
        out = pathlib.Path(d) / "bn5.jsonl"
        runs.append(run_tests("omniget", "job_commands::", out))
        lines = [l for l in read_lines(out) if l.get("scenario") == "delegate"]
    m = {l["metric"]: l.get("value") for l in lines if "metric" in l}
    st = m.get("route_statuses") or {}
    goals = [
        {"goal": "MCP agent_run -> approval -> approval_respond -> job_diff, no window", "target": "10/10",
         "value": m.get("mcp_delegations_ok"), "pass": m.get("mcp_delegations_ok") == 10},
        {"goal": "HTTP /v1/commands job.start -> wait -> approval.respond -> wait", "target": "10/10",
         "value": m.get("http_delegations_ok"), "pass": m.get("http_delegations_ok") == 10},
        {"goal": "same command_id 5x -> 1 job (MCP and HTTP; copies flagged)", "target": "1 / 1 / 4 replays",
         "value": {"mcp": m.get("mcp_same_command_5x_jobs"), "http": m.get("http_same_command_5x_jobs"),
                   "replays": m.get("http_replays_flagged")},
         "pass": m.get("mcp_same_command_5x_jobs") == 1 and m.get("http_same_command_5x_jobs") == 1
         and m.get("http_replays_flagged") == 4},
        {"goal": "reader token, extension key and a job context cannot start a job", "target": 403,
         "value": st, "pass": all(st.get(k) == 403 for k in ("reader_starts", "extension_key", "from_a_job"))},
        {"goal": "every approved call ran exactly once", "target": 20, "value": m.get("tools_ran"),
         "pass": m.get("tools_ran") == 20},
        {"goal": "0 non-wait calls blocking > 2 s", "target": "< 2000 ms", "value": m.get("slowest_non_wait_call_ms"),
         "pass": m.get("slowest_non_wait_call_ms") is not None and m["slowest_non_wait_call_ms"] < 2000},
        {"goal": "orchestration overhead end to end (stand-in turn, no model)", "target": "p50 < 1000 ms",
         "value": {"p50_ms": m.get("e2e_ms_p50"), "max_ms": m.get("e2e_ms_max")},
         "pass": m.get("e2e_ms_p50") is not None and m["e2e_ms_p50"] < 1000},
    ]
    return runs, goals, {"metrics": m}

def scenario_workflow():
    runs = []
    with tempfile.TemporaryDirectory() as d:
        out = pathlib.Path(d) / "bn5.jsonl"
        runs.append(run_tests("omniget", "job_commands::tests::bn5_workflow", out))
        runs.append(run_tests("omniget", "job_workflow::", out))
        runs.append(run_tests("omniget-core", "llm::workflow", out))
        lines = [l for l in read_lines(out) if l.get("scenario") == "workflow"]
    m = {l["metric"]: l.get("value") for l in lines if "metric" in l}
    goals = [
        {"goal": "\"delegue a correção X\" without flags gets the workflow's agent/run_mode/isolation/prompt",
         "target": "5/5", "value": m.get("workflow_delegations_ok"), "pass": m.get("workflow_delegations_ok") == 5},
        {"goal": "a flag the caller gives wins over the workflow", "target": True,
         "value": m.get("flags_win"), "pass": m.get("flags_win") is True},
        {"goal": "invalid reload keeps the previous config (error surfaced on the job)", "target": True,
         "value": m.get("invalid_reload_keeps_previous"), "pass": m.get("invalid_reload_keeps_previous") is True},
        {"goal": "invalid file without a last good version refuses the start", "target": "ERR_WORKFLOW_INVALID",
         "value": m.get("invalid_without_last_good_refused"), "pass": m.get("invalid_without_last_good_refused") is True},
        {"goal": "untrusted file never loosens run_mode", "target": True,
         "value": m.get("untrusted_run_mode_held"), "pass": m.get("untrusted_run_mode_held") is True},
        {"goal": "untrusted hooks never run", "target": 0,
         "value": {"untrusted": m.get("untrusted_hook_runs"), "after_change": m.get("changed_file_hook_runs")},
         "pass": m.get("untrusted_hook_runs") == 0 and m.get("changed_file_hook_runs") == 0},
        {"goal": "trusted after_create runs once; failing before_remove blocks removal", "target": "1 / blocked",
         "value": {"after_create": m.get("trusted_after_create_runs"), "blocked": m.get("before_remove_failure_blocks_removal")},
         "pass": m.get("trusted_after_create_runs") == 1 and m.get("before_remove_failure_blocks_removal") is True},
        {"goal": "job_report: one call for the delegating agent, deny-by-default grant", "target": True,
         "value": {"one_call": m.get("job_report_one_call"), "deny_by_default": m.get("job_report_denied_by_default")},
         "pass": m.get("job_report_one_call") is True and m.get("job_report_denied_by_default") is True},
    ]
    return runs, goals, {"metrics": m}

SCENARIOS = {"auth": scenario_auth, "delegate": scenario_delegate, "workflow": scenario_workflow}
NOT_YET = {}

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fake", action="store_true", default=True)
    ap.add_argument("--real", action="store_true")
    ap.add_argument("--scenario", default="", help="comma-separated: " + ",".join(SCENARIOS))
    ap.add_argument("--out", default=str(ROOT / "estudos" / "88-onda2" / "bn5-delegation.json"))
    args = ap.parse_args()
    if args.real:
        sys.exit("--real: every scenario is --fake (the real-model number is B0, missions_eval.py --reps 3).")
    wanted = [s.strip() for s in args.scenario.split(",") if s.strip()] or list(SCENARIOS)
    unknown = [s for s in wanted if s not in SCENARIOS]
    if unknown:
        sys.exit(f"unknown scenario(s) {unknown}; implemented: {list(SCENARIOS)}; later: {list(NOT_YET)}")

    runs, goals, detail = [], [], {}
    for name in wanted:
        r, g, d = SCENARIOS[name]()
        runs += r
        goals += [dict(x, scenario=name) for x in g]
        detail[name] = d
    report = {
        "bench": "BN5 delegation_e2e",
        "when": dt.datetime.now().isoformat(timespec="seconds"),
        "mode": "fake",
        "scenarios": wanted,
        "level": {"auth": "HTTP on 127.0.0.1 + production guard + jobs.db token table + JobTokenLease; child env via real spawn_turn",
                  "delegate": "production job_commands (execute, receipts, wait, HTTP handlers, MCP tools) + guard over HTTP; stand-in engine with a real broker and run record",
                  "workflow": "production Commands::execute + MCP tools + workflow store over a stand-in engine; hooks against real git worktrees and processes"},
        "runs": runs,
        "goals": goals,
        "detail": detail,
        "not_measured": NOT_YET,
        "all_pass": all(g["pass"] for g in goals) and all(r["exit"] == 0 for r in runs),
    }
    pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.out).write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    for g in goals:
        print(("PASS " if g["pass"] else "FAIL ") + f"[{g['scenario']}] " + g["goal"] + f"  ({g['value']})")
    print("all_pass:", report["all_pass"], "->", args.out)
    return 0 if report["all_pass"] else 1

if __name__ == "__main__":
    sys.exit(main())
