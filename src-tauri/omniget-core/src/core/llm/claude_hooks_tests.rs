use super::*;
use crate::core::llm::perm::{Action, Rule};
use serde_json::json;

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "omniget-hooks-{name}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rule(tool: &str, pattern: &str, action: Action) -> Rule {
    Rule {
        tool: tool.into(),
        pattern: pattern.into(),
        action,
    }
}

const WS: &str = "/ws";

fn policy(rules: Vec<Rule>, run_mode: Option<RunMode>) -> Policy {
    Policy {
        agent: "agent".into(),
        rules_file: None,
        rules,
        run_mode,
        workspace: Some(PathBuf::from(WS)),
    }
}

fn payload(tool: &str, input: Value) -> Vec<u8> {
    json!({
        "session_id": "s", "transcript_path": "/t", "cwd": WS,
        "hook_event_name": "PreToolUse", "permission_mode": "acceptEdits",
        "tool_name": tool, "tool_input": input, "tool_use_id": "toolu_1"
    })
    .to_string()
    .into_bytes()
}

fn turn(policy: &Policy) -> TurnHooks {
    prepare(&tmp("root"), Path::new("/bin/omniget-hook-shim"), policy).unwrap()
}

#[test]
fn allow_prints_nothing_and_deny_says_why() {
    let h = turn(&policy(
        vec![rule("fs_write", "**/.env", Action::Deny)],
        Some(RunMode::AutoEdits),
    ));
    let ok = run(
        "PreToolUse",
        &h.dir,
        &payload(
            "Write",
            json!({"file_path": "/ws/src/main.py", "content": "x"}),
        ),
    );
    assert_eq!(ok, Outcome::silent());
    let denied = run(
        "PreToolUse",
        &h.dir,
        &payload("Write", json!({"file_path": "/ws/.env", "content": "K=1"})),
    );
    assert_eq!(denied.code, 0);
    let out: Value = serde_json::from_str(&denied.stdout).unwrap();
    assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
    let why = out["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap();
    assert!(why.contains("deny fs_write **/.env"), "{why}");
    h.cleanup();
}

#[test]
fn only_a_stored_ask_forces_a_question() {
    // The shell asks by grant in auto_edits: that is the CLI's own question,
    // so nothing is printed (it would override the user's allow list).
    let h = turn(&policy(
        vec![rule("fs_edit", "*", Action::Ask)],
        Some(RunMode::AutoEdits),
    ));
    let bash = run(
        "PreToolUse",
        &h.dir,
        &payload("Bash", json!({"command": "ls"})),
    );
    assert_eq!(bash, Outcome::silent());
    let edit = run(
        "PreToolUse",
        &h.dir,
        &payload("Edit", json!({"file_path": "/ws/a.txt"})),
    );
    let out: Value = serde_json::from_str(&edit.stdout).unwrap();
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "ask");
    h.cleanup();
}

#[test]
fn omniget_own_tools_are_not_judged_twice() {
    let h = turn(&policy(vec![rule("*", "*", Action::Deny)], None));
    let own = run(
        "PreToolUse",
        &h.dir,
        &payload("mcp__omniget__omniget_permission", json!({})),
    );
    assert_eq!(own, Outcome::silent());
    let other = run(
        "PreToolUse",
        &h.dir,
        &payload("Read", json!({"file_path": "/ws/a"})),
    );
    assert!(other.stdout.contains("\"deny\""));
    h.cleanup();
}

#[test]
fn pre_fails_closed_and_post_fails_open() {
    let gone = tmp("gone").join("nope");
    let pre = run(
        "PreToolUse",
        &gone,
        &payload("Read", json!({"file_path": "/ws/a"})),
    );
    assert_eq!(pre.code, 2, "a missing policy blocks the call");
    assert!(pre.stderr.contains("[OmniGet]"));
    let h = turn(&policy(vec![], None));
    assert_eq!(run("PreToolUse", &h.dir, b"not json").code, 2);
    let post = run("PostToolUse", &gone, b"not json");
    assert_eq!(post, Outcome::silent());
    // A malformed rules file is an error, never "no rules".
    let rules = h.dir.join("rules.json");
    std::fs::write(&rules, b"{broken").unwrap();
    let mut p = policy(vec![], None);
    p.rules_file = Some(rules);
    let h2 = turn(&p);
    let broken = run(
        "PreToolUse",
        &h2.dir,
        &payload("Read", json!({"file_path": "/ws/a"})),
    );
    assert_eq!(broken.code, 2);
    h.cleanup();
    h2.cleanup();
}

#[test]
fn post_tells_the_model_about_a_denied_change() {
    let h = turn(&policy(
        vec![rule("fs_write", "**/.env", Action::Deny)],
        None,
    ));
    let out = run(
        "PostToolUse",
        &h.dir,
        &payload("Write", json!({"file_path": "/ws/.env"})),
    );
    let v: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    assert!(v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("deny fs_write"));
    h.cleanup();
}

#[test]
fn a_rule_saved_mid_turn_counts_from_the_next_call() {
    let root = tmp("live");
    let file = root.join("permission-rules.json");
    let mut p = policy(vec![], Some(RunMode::AutoEdits));
    p.rules_file = Some(file.clone());
    let h = turn(&p);
    let call = payload("Write", json!({"file_path": "/ws/.env"}));
    assert_eq!(run("PreToolUse", &h.dir, &call), Outcome::silent());
    std::fs::write(
        &file,
        json!({ "agent": [rule("fs_write", "**/.env", Action::Deny)] }).to_string(),
    )
    .unwrap();
    assert!(run("PreToolUse", &h.dir, &call).stdout.contains("\"deny\""));
    h.cleanup();
}

#[cfg(unix)]
#[test]
fn the_turn_folder_is_private_and_goes_away() {
    use std::os::unix::fs::PermissionsExt;
    let root = tmp("private");
    let h = prepare(
        &root,
        Path::new("/opt/My Tools/omniget-hook-shim"),
        &policy(vec![], None),
    )
    .unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&h.dir), 0o700);
    assert_eq!(mode(&h.settings), 0o600);
    assert_eq!(mode(&h.dir.join("policy.json")), 0o600);
    let s: Value = serde_json::from_slice(&std::fs::read(&h.settings).unwrap()).unwrap();
    let pre = &s["hooks"]["PreToolUse"][0];
    assert_eq!(pre["matcher"], "*");
    let cmd = pre["hooks"][0]["command"].as_str().unwrap();
    assert!(
        cmd.starts_with("'/opt/My Tools/omniget-hook-shim' hook claude --event PreToolUse --turn "),
        "{cmd}"
    );
    assert_eq!(s["hooks"]["PostToolUse"][0]["matcher"], POST_MATCHER);
    h.cleanup();
    assert!(!h.dir.exists());
}

#[test]
fn the_empty_sidecar_placeholder_is_not_a_hook_binary() {
    let dir = tmp("shim");
    let empty = dir.join("omniget-hook-shim");
    let real = dir.join("real-shim");
    let text = dir.join("not-a-binary");
    std::fs::write(&empty, b"").unwrap();
    std::fs::write(&real, b"#!/bin/sh\n").unwrap();
    std::fs::write(&text, b"placeholder\n").unwrap();
    let missing = dir.join("missing");
    assert_eq!(
        first_usable([missing, empty.clone(), text.clone(), real.clone()]),
        Some(real)
    );
    assert_eq!(first_usable([empty, text]), None);
    // The test binary itself is a real executable of this platform.
    let me = std::env::current_exe().unwrap();
    assert_eq!(first_usable([me.clone()]), Some(me));
}

#[test]
fn claude_tools_map_onto_the_native_ones() {
    let ws = Some(Path::new(WS));
    let (t, i) = perm::from_claude("Write", &json!({"file_path": "/ws/a/.env"}), ws);
    assert_eq!(
        (t.as_str(), i["path"].as_str()),
        ("fs_write", Some("a/.env"))
    );
    let (t, i) = perm::from_claude("Bash", &json!({"command": "git status"}), ws);
    assert_eq!(
        (t.as_str(), i["command"].as_str()),
        ("shell_exec", Some("git status"))
    );
    let (t, _) = perm::from_claude("MultiEdit", &json!({"file_path": "/ws/x"}), ws);
    assert_eq!(t, "fs_edit");
    let (t, i) = perm::from_claude("Read", &json!({"file_path": "/elsewhere/x"}), ws);
    assert_eq!(
        (t.as_str(), i["path"].as_str()),
        ("fs_read", Some("/elsewhere/x"))
    );
    assert_eq!(
        perm::from_claude("WebFetch", &json!({}), ws).0,
        "cli:WebFetch"
    );
    // `**/.env` also covers the top-level `.env`.
    assert!(perm::matches("**/.env", ".env"));
    assert!(perm::matches("**/.env", "a/b/.env"));
    assert!(!perm::matches("**/.env", "a/.envrc"));
}

// ── BN8: native and CLI give the same verdict ────────────────────────────

/// One BN8 case: rules, run mode and a Claude tool call; the native harness
/// gets the same call under the native name, with the grants a CLI tool has
/// ([`perm::cli_grant`]).
struct Case {
    id: &'static str,
    rules: Vec<Rule>,
    mode: Option<RunMode>,
    tool: &'static str,
    input: Value,
    want: GrantMode,
}

fn cases() -> Vec<Case> {
    use Action::*;
    use GrantMode as G;
    use RunMode::*;
    let env_deny = || vec![rule("fs_write", "**/.env", Deny)];
    let c = |id, rules, mode, tool, input, want| Case {
        id,
        rules,
        mode,
        tool,
        input,
        want,
    };
    vec![
        c(
            "env-top",
            env_deny(),
            Some(AutoEdits),
            "Write",
            json!({"file_path": "/ws/.env", "content": "K=1"}),
            G::Deny,
        ),
        c(
            "env-nested",
            env_deny(),
            Some(AutoEdits),
            "Write",
            json!({"file_path": "/ws/app/.env", "content": "K=1"}),
            G::Deny,
        ),
        c(
            "env-other-file",
            env_deny(),
            Some(AutoEdits),
            "Write",
            json!({"file_path": "/ws/app/main.py", "content": "x"}),
            G::Auto,
        ),
        c(
            "env-edit",
            vec![rule("fs_edit", "**/.env", Deny)],
            Some(AutoEdits),
            "Edit",
            json!({"file_path": "/ws/.env", "old_string": "a", "new_string": "b"}),
            G::Deny,
        ),
        c(
            "rm-deny",
            vec![rule("shell_exec", "rm *", Deny)],
            Some(AutoEdits),
            "Bash",
            json!({"command": "rm -rf build"}),
            G::Deny,
        ),
        c(
            "rm-chained",
            vec![rule("shell_exec", "rm *", Deny)],
            Some(AutoEdits),
            "Bash",
            json!({"command": "ls && rm -rf x"}),
            G::Deny,
        ),
        c(
            "git-status-allow",
            vec![rule("shell_exec", "git status *", Allow)],
            Some(AutoEdits),
            "Bash",
            json!({"command": "git status --short"}),
            G::Auto,
        ),
        c(
            "git-status-chained",
            vec![rule("shell_exec", "git status *", Allow)],
            Some(AutoEdits),
            "Bash",
            json!({"command": "git status && curl x"}),
            G::Ask,
        ),
        c(
            "shell-by-grant",
            vec![],
            Some(AutoEdits),
            "Bash",
            json!({"command": "ls"}),
            G::Ask,
        ),
        c(
            "approval-write",
            vec![],
            Some(ApprovalRequired),
            "Write",
            json!({"file_path": "/ws/a.txt", "content": "x"}),
            G::Ask,
        ),
        c(
            "approval-read",
            vec![],
            Some(ApprovalRequired),
            "Read",
            json!({"file_path": "/ws/a.txt"}),
            G::Auto,
        ),
        c(
            "full-shell",
            vec![],
            Some(FullAccess),
            "Bash",
            json!({"command": "ls"}),
            G::Auto,
        ),
        c(
            "full-write",
            vec![],
            Some(FullAccess),
            "Write",
            json!({"file_path": "/ws/a.txt", "content": "x"}),
            G::Auto,
        ),
        c(
            "secret-read",
            vec![rule("fs_read", "**/secret*", Deny)],
            Some(AutoEdits),
            "Read",
            json!({"file_path": "/ws/secret.txt"}),
            G::Deny,
        ),
        c(
            "ask-every-edit",
            vec![rule("fs_edit", "*", Ask)],
            Some(AutoEdits),
            "Edit",
            json!({"file_path": "/ws/a.txt", "old_string": "a", "new_string": "b"}),
            G::Ask,
        ),
        c(
            "ask-src-write",
            vec![rule("fs_write", "src/*", Ask)],
            Some(AutoEdits),
            "Write",
            json!({"file_path": "/ws/src/x.rs", "content": "x"}),
            G::Ask,
        ),
        c(
            "ask-src-other",
            vec![rule("fs_write", "src/*", Ask)],
            Some(AutoEdits),
            "Write",
            json!({"file_path": "/ws/docs/x.md", "content": "x"}),
            G::Auto,
        ),
        c(
            "allow-write-approval",
            vec![rule("fs_write", "*", Allow)],
            Some(ApprovalRequired),
            "Write",
            json!({"file_path": "/ws/a.txt", "content": "x"}),
            G::Auto,
        ),
        c(
            "deny-everything",
            vec![rule("*", "*", Deny)],
            Some(FullAccess),
            "Read",
            json!({"file_path": "/ws/a.txt"}),
            G::Deny,
        ),
        c(
            "glob-deny",
            vec![rule("fs_glob", "*", Deny)],
            Some(AutoEdits),
            "Glob",
            json!({"pattern": "**/*.rs"}),
            G::Deny,
        ),
        c(
            "grep-deny",
            vec![rule("fs_grep", "*", Deny)],
            Some(AutoEdits),
            "Grep",
            json!({"pattern": "TODO"}),
            G::Deny,
        ),
        c(
            "webfetch-deny",
            vec![rule("cli:WebFetch", "*", Deny)],
            Some(AutoEdits),
            "WebFetch",
            json!({"url": "https://x"}),
            G::Deny,
        ),
        c(
            "webfetch-grant",
            vec![],
            Some(AutoEdits),
            "WebFetch",
            json!({"url": "https://x"}),
            G::Ask,
        ),
        c(
            "npm-test-allow",
            vec![rule("shell_exec", "npm run test *", Allow)],
            Some(AutoEdits),
            "Bash",
            json!({"command": "npm run test -- -t x"}),
            G::Auto,
        ),
        c(
            "push-deny-full",
            vec![rule("shell_exec", "git push *", Deny)],
            Some(FullAccess),
            "Bash",
            json!({"command": "git push origin main"}),
            G::Deny,
        ),
        c(
            "auto-edit",
            vec![],
            Some(Auto),
            "Edit",
            json!({"file_path": "/ws/a.txt", "old_string": "a", "new_string": "b"}),
            G::Auto,
        ),
        c(
            "auto-shell",
            vec![],
            Some(Auto),
            "Bash",
            json!({"command": "ls"}),
            G::Ask,
        ),
        c(
            "env-approval",
            env_deny(),
            Some(ApprovalRequired),
            "Write",
            json!({"file_path": "/ws/.env", "content": "K=1"}),
            G::Deny,
        ),
        c(
            "lock-multiedit",
            vec![rule("fs_edit", "*.lock", Deny)],
            Some(AutoEdits),
            "MultiEdit",
            json!({"file_path": "/ws/Cargo.lock", "edits": []}),
            G::Deny,
        ),
        c(
            "opaque-shell",
            vec![rule("shell_exec", "ls *", Allow)],
            Some(AutoEdits),
            "Bash",
            json!({"command": "ls $(pwd)"}),
            G::Ask,
        ),
    ]
}

fn word(mode: GrantMode) -> &'static str {
    match mode {
        GrantMode::Auto => "allow",
        GrantMode::Ask => "ask",
        GrantMode::Deny => "deny",
    }
}

/// The native harness's verdict: the real `ToolBroker::call` under the
/// case's run mode and rules. An ask shows up as `ToolAsk` on the bus (and
/// is refused); a call that runs is `allow`; an error without an ask is
/// `deny`.
async fn native_verdict(case: &Case, n: usize) -> GrantMode {
    use crate::core::llm::agent::{ToolGrant, ToolSource};
    use crate::core::llm::broker::{ToolBroker, ToolExecutor};
    use crate::core::llm::error::LlmError;
    use crate::core::llm::types::ToolSpec;
    use crate::core::omni::bus::{Bus, BusEvent};
    use std::sync::Arc;

    struct Echo;
    #[async_trait::async_trait]
    impl ToolExecutor for Echo {
        async fn execute(&self, name: &str, _input: Value) -> Result<String, LlmError> {
            Ok(name.to_string())
        }
    }
    let (native, input) = perm::from_claude(case.tool, &case.input, Some(Path::new(WS)));
    let bus = Arc::new(Bus::new());
    let broker = Arc::new(ToolBroker::new(
        vec![ToolSpec {
            name: native.clone(),
            description: String::new(),
            input_schema: json!({ "type": "object" }),
        }],
        Arc::new(Echo),
        bus.clone(),
    ));
    let agent = format!("bn8-agent-{n}-{}", std::process::id());
    let conv = format!("bn8-conv-{n}-{}", std::process::id());
    perm::set_rules(&agent, case.rules.clone());
    if let Some(mode) = case.mode {
        crate::core::llm::run_mode::bind(&conv, mode);
    }
    let grants = vec![ToolGrant {
        source: ToolSource::Internal {
            name: native.clone(),
        },
        mode: perm::cli_grant(&native),
    }];
    let mut rx = bus.subscribe();
    let (b, a, c, name) = (broker.clone(), agent.clone(), conv.clone(), native.clone());
    let task = tokio::spawn(async move {
        let a2 = a.clone();
        crate::core::llm::code_tools::scope(&c, &a, "bn8-req", async move {
            b.call(&a2, &grants, "bn8-req", "bn8-call", &name, input)
                .await
        })
        .await
    });
    let mut asked = false;
    let out = loop {
        tokio::select! {
            ev = rx.recv() => {
                if let Ok(BusEvent::ToolAsk { tool_call_id, .. }) = ev {
                    asked = true;
                    while !broker.answer(&tool_call_id, false) {
                        tokio::task::yield_now().await;
                    }
                }
            }
            _ = async { while !task.is_finished() { tokio::task::yield_now().await; } } => {
                break task.await.unwrap();
            }
        }
    };
    crate::core::llm::run_mode::unbind(&conv);
    perm::set_rules(&agent, Vec::new());
    match (asked, out) {
        (true, _) => GrantMode::Ask,
        (false, Ok(_)) => GrantMode::Auto,
        (false, Err(_)) => GrantMode::Deny,
    }
}

/// The CLI's final verdict as Claude Code would reach it with these hooks:
/// the hook's `PreToolUse` answer first; when it says nothing, the CLI's own
/// mode (edits pass in `acceptEdits`/`auto`/`bypass`, reads always pass,
/// everything passes in `bypass`), and what the mode would ask goes to the
/// prompt tool, which asks the same gate (`PermissionRequest` here).
fn cli_verdict(case: &Case, dir: &Path) -> GrantMode {
    let pre = run("PreToolUse", dir, &payload(case.tool, case.input.clone()));
    assert_eq!(pre.code, 0, "{}: {}", case.id, pre.stderr);
    if !pre.stdout.is_empty() {
        let v: Value = serde_json::from_str(&pre.stdout).unwrap();
        match v["hookSpecificOutput"]["permissionDecision"].as_str() {
            Some("deny") => return GrantMode::Deny,
            Some("ask") => {}
            other => panic!("{}: unexpected decision {other:?}", case.id),
        }
    } else {
        let flag = case.mode.unwrap_or_default();
        let read = matches!(case.tool, "Read" | "Glob" | "Grep" | "LS");
        let edit = matches!(case.tool, "Write" | "Edit" | "MultiEdit" | "NotebookEdit");
        let passes = read
            || flag == RunMode::FullAccess
            || (edit && matches!(flag, RunMode::AutoEdits | RunMode::Auto));
        if passes {
            return GrantMode::Auto;
        }
    }
    let prompt = run(
        "PermissionRequest",
        dir,
        &payload(case.tool, case.input.clone()),
    );
    if prompt.stdout.is_empty() {
        return GrantMode::Ask;
    }
    let v: Value = serde_json::from_str(&prompt.stdout).unwrap();
    match v["hookSpecificOutput"]["decision"]["behavior"].as_str() {
        Some("allow") => GrantMode::Auto,
        _ => GrantMode::Deny,
    }
}

#[tokio::test]
async fn bn8_native_and_cli_give_the_same_verdict() {
    let root = tmp("bn8");
    // BN8 (perm_parity.py) runs the fake CLI against these turn folders
    // with the real hook binary; a plain `cargo test` checks in process.
    let shim = std::env::var("OMNIGET_HOOK_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/bin/omniget-hook-shim"));
    let keep = std::env::var("OMNIGET_BENCH_OUT").ok();
    let mut rows = Vec::new();
    let mut agree = 0;
    let all = cases();
    assert_eq!(all.len(), 30);
    for (n, case) in all.iter().enumerate() {
        let native = native_verdict(case, n).await;
        let rules_file = root.join(format!("rules-{n}.json"));
        std::fs::write(&rules_file, json!({ "agent": case.rules }).to_string()).unwrap();
        let p = Policy {
            agent: "agent".into(),
            rules_file: Some(rules_file),
            rules: Vec::new(),
            run_mode: case.mode,
            workspace: Some(PathBuf::from(WS)),
        };
        let h = prepare(&root, &shim, &p).unwrap();
        let cli = cli_verdict(case, &h.dir);
        if native == cli && native == case.want {
            agree += 1;
        }
        rows.push(json!({
            "case": case.id, "tool": case.tool, "input": case.input,
            "run_mode": case.mode.map(|m| m.as_str()),
            "rules": case.rules, "want": word(case.want),
            "native": word(native), "cli_in_process": word(cli),
            "settings": h.settings, "turn": h.dir,
        }));
        if keep.is_none() {
            h.cleanup();
        }
    }
    if let Some(out) = keep {
        let body: String = rows.iter().map(|r| format!("{r}\n")).collect();
        std::fs::write(out, body).unwrap();
    }
    let diverged: Vec<&Value> = rows
        .iter()
        .filter(|r| r["native"] != r["cli_in_process"] || r["native"] != r["want"])
        .collect();
    assert_eq!(agree, 30, "diverged: {diverged:#?}");
}

#[test]
fn the_missions_harness_mirrors_the_settings() {
    // B0 measures what the app launches: keep scripts/llm/missions_eval.py
    // in step with `settings_json` (the briefing's argv rule).
    let harness = include_str!("../../../../../scripts/llm/missions_eval.py");
    assert!(harness.contains(&format!("HOOK_POST_MATCHER = '{POST_MATCHER}'")));
    assert!(harness.contains(&format!("HOOK_TIMEOUT_S = {HOOK_TIMEOUT_S}")));
    assert!(harness.contains("hook claude --event {event} --turn"));
    assert!(harness.contains("out += ['--settings'"));
    let s = settings_json(Path::new("/x/shim"), Path::new("/t"));
    assert_eq!(
        s["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"],
        HOOK_TIMEOUT_S
    );
}

#[test]
fn the_parser_hands_the_baselines_of_its_turn_to_the_sink() {
    use crate::core::llm::cli_runtime::claude::ClaudeParser;
    use crate::core::llm::cli_runtime::session::LineParser;
    use crate::core::llm::file_changes::{self, FileTouch};
    use std::sync::{Arc, Mutex};
    let got: Arc<Mutex<Vec<(String, FileTouch)>>> = Arc::default();
    let sink = got.clone();
    file_changes::set_sink(Some(Arc::new(move |req: &str, t| {
        sink.lock().unwrap().push((req.to_string(), t));
    })));
    let request = format!("req-baseline-{}", uuid::Uuid::new_v4().simple());
    let parser = ClaudeParser::new(false).with_baselines(request.clone());
    // SP3: a Write over an existing file, as the stream carries it.
    let line = json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "toolu_1", "content": "updated"}]},
        "tool_use_result": {"type": "update", "filePath": "/w/b.txt", "content": "new",
            "structuredPatch": [], "originalFile": "old content\n", "userModified": false}});
    parser.parse_line(&line.to_string());
    // A parser without a turn records nothing.
    ClaudeParser::new(false).parse_line(&line.to_string());
    let mine: Vec<FileTouch> = got
        .lock()
        .unwrap()
        .iter()
        .filter(|(r, _)| r == &request)
        .map(|(_, t)| t.clone())
        .collect();
    file_changes::set_sink(None);
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].path, "/w/b.txt");
    assert_eq!(mine[0].original.as_deref(), Some("old content\n"));
    assert_eq!(got.lock().unwrap().len(), 1);
}
