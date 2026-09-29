//! 2.7 — run mode in the argv, end to end at the turn level.
//!
//! The argv comes from the real `CliRuntime::plan_with` (the turn's run mode
//! under the account's ceiling, the probed caps); the CLI is
//! `scripts/llm/fake_claude.py --mode perm` (an Edit of `notes.txt` and a
//! Bash `ls`, decided by the permission flags it was given); the stream goes
//! through the real `ClaudeParser`/`CodexParser` and pipes. BN12
//! (`scripts/llm/runmode_matrix.py`) reads the rows these tests write to
//! `$OMNIGET_BENCH_OUT`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::super::run_mode::RunMode;
use super::super::types::{GenParams, Message, ModelRef, ProviderId, Role, TurnEvent};
use super::*;

fn fake() -> Option<PathBuf> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/llm/fake_claude.py");
    let ok = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    (ok && script.is_file()).then_some(script)
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "omniget-bn12-{name}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "draft\n").unwrap();
    dir
}

fn account(root: &Path, cli: CliKind, sandbox: SandboxMode) -> (Arc<AccountStore>, CliAccount) {
    let store = AccountStore::at(root.join("accounts.json"));
    let account = CliAccount {
        id: "acc".into(),
        cli,
        config_dir: root.join("profile"),
        label: "BN12".into(),
        disabled: false,
        sandbox,
    };
    store.create(account.clone()).unwrap();
    (Arc::new(store), account)
}

/// What the turn saw.
#[derive(Debug, Default)]
struct Row {
    questions: u64,
    denials: u64,
    /// Denied tool results that carry OmniGet's reason.
    explained: u64,
    /// The `auto` fallback note was shown.
    fallback_note: bool,
    permission_flags: String,
    finished: bool,
}

struct Case<'a> {
    cli: CliKind,
    mode: RunMode,
    sandbox: SandboxMode,
    surface: bool,
    model: &'a str,
}

async fn run(case: &Case<'_>) -> Row {
    let script = fake().expect("fake");
    let root = folder(&format!("{}-{}", case.cli, case.mode));
    let (store, account) = account(&root, case.cli, case.sandbox);
    let runtime = CliRuntime::new(store, Arc::new(CliCapacity::new()));
    let launch = LaunchPlan {
        cwd: Some(root.clone()),
        projectless: false,
        resume: None,
        resume_kind: crate::core::assist::runs::ResumeKind::New,
        mcp_config: None,
        permission_prompt_tool: case
            .surface
            .then(|| "mcp__omniget__permission_prompt".to_string()),
        restrict_tools: true,
        external: false,
        lean_project: true,
        effort_flag: true,
        run_mode: Some(case.mode),
        // The installed 2.1.284 lists `auto`; codex 0.158 lists `--approve-for-me`.
        auto_mode: true,
        approve_for_me: true,
    };
    let req = TurnRequest {
        model: ModelRef {
            provider: ProviderId::new("cli"),
            model: case.model.into(),
        },
        messages: vec![Message::text(Role::User, "edit notes.txt and run ls")],
        tools: vec![],
        params: GenParams::default(),
        cancel: CancellationToken::new(),
        agent_id: Some("worker".into()),
    };
    let (mut spec, parser) = runtime.plan_with(
        &account,
        PathBuf::from("python3"),
        &req,
        None,
        Some(&launch),
    );
    let flags: Vec<String> = spec
        .args
        .windows(2)
        .filter(|w| {
            matches!(
                w[0].as_str(),
                "--permission-mode" | "--permission-prompts" | "--sandbox" | "-c"
            )
        })
        .map(|w| format!("{} {}", w[0], w[1]))
        .chain(
            spec.args
                .iter()
                .filter(|a| *a == "--approve-for-me")
                .cloned(),
        )
        .collect();
    spec.args.insert(0, script.display().to_string());
    let trace = root.join("trace.jsonl");
    spec.env = BTreeMap::from([
        ("FAKE_CLAUDE_MODE".to_string(), "perm".to_string()),
        ("FAKE_CLAUDE_TRACE".to_string(), trace.display().to_string()),
    ]);
    let mut stream =
        session::spawn_turn_with(spec, parser, CancellationToken::new(), Default::default())
            .unwrap();
    let mut row = Row {
        permission_flags: flags.join(" "),
        ..Row::default()
    };
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        match event {
            TurnEvent::ToolResult {
                is_error: true,
                content,
                ..
            } => {
                if content.contains("[OmniGet] Denied: ") {
                    row.explained += 1;
                }
            }
            TurnEvent::TextDelta { text: t } => text.push_str(&t),
            TurnEvent::Finished { .. } => row.finished = true,
            _ => {}
        }
    }
    row.fallback_note = text.contains("Run mode `auto` is not available");
    let last = std::fs::read_to_string(&trace).unwrap_or_default();
    let totals: serde_json::Value = last
        .lines()
        .last()
        .and_then(|l| serde_json::from_str(l).ok())
        .unwrap_or_default();
    row.questions = totals["questions"].as_u64().unwrap_or(u64::MAX);
    row.denials = totals["denials"].as_u64().unwrap_or(u64::MAX);
    let _ = std::fs::remove_dir_all(&root);
    row
}

fn bench(case: &Case<'_>, row: &Row, expected: u64) {
    if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(
                f,
                "{}",
                serde_json::json!({
                    "bench": "bn12",
                    "cli": case.cli.as_str(),
                    "run_mode": case.mode.as_str(),
                    "account": case.sandbox.as_str(),
                    "surface": case.surface,
                    "model": case.model,
                    "argv": row.permission_flags,
                    "questions": row.questions,
                    "expected_questions": expected,
                    "denials": row.denials,
                    "explained": row.explained,
                    "fallback_note": row.fallback_note,
                })
            );
        }
    }
}

/// Questions per mode, the table of `82#P5`: 2 in `approval_required` (the
/// edit and the command), 1 in `auto_edits` (the command), 0 in `auto` (the
/// CLI's reviewer) and `full_access`.
fn table(mode: RunMode) -> u64 {
    match mode {
        RunMode::ApprovalRequired => 2,
        RunMode::AutoEdits => 1,
        RunMode::Auto | RunMode::FullAccess => 0,
    }
}

#[tokio::test]
async fn bn12_questions_per_mode_follow_the_table_and_no_denial_goes_unexplained() {
    if fake().is_none() {
        return eprintln!("python3 or fake_claude.py missing: skipped");
    }
    let mut cases = Vec::new();
    for mode in super::super::run_mode::ALL {
        // Claude with the prompt tool (the app), Claude without it (nobody
        // to ask: every question is a denial with its reason), and Codex
        // (`exec` never has anyone to ask).
        for (cli, surface) in [
            (CliKind::Claude, true),
            (CliKind::Claude, false),
            (CliKind::Codex, false),
        ] {
            cases.push(Case {
                cli,
                mode,
                sandbox: SandboxMode::Full,
                surface,
                model: "sonnet",
            });
        }
    }
    for case in &cases {
        let row = run(case).await;
        let expected = table(case.mode);
        bench(case, &row, expected);
        let label = format!("{} {} surface={}", case.cli, case.mode, case.surface);
        assert!(row.finished, "{label}: {row:?}");
        assert_eq!(row.questions, expected, "{label}: {row:?}");
        let expected_denials = if case.surface { 0 } else { expected };
        assert_eq!(row.denials, expected_denials, "{label}: {row:?}");
        assert_eq!(row.explained, row.denials, "{label}: every denial says why");
        assert!(!row.fallback_note, "{label}");
    }
}

/// SP2: `auto` on a haiku model silently runs `default`. The turn still asks
/// (safe) and the user is told why in the first line of the reply.
#[tokio::test]
async fn bn12_auto_that_the_model_refuses_asks_and_says_so() {
    if fake().is_none() {
        return eprintln!("python3 or fake_claude.py missing: skipped");
    }
    let case = Case {
        cli: CliKind::Claude,
        mode: RunMode::Auto,
        sandbox: SandboxMode::Full,
        surface: true,
        model: "haiku",
    };
    let row = run(&case).await;
    bench(&case, &row, 2);
    assert!(
        row.permission_flags.contains("--permission-mode auto"),
        "{row:?}"
    );
    assert!(row.fallback_note, "{row:?}");
    assert_eq!((row.questions, row.denials), (2, 0), "{row:?}");
}

/// The account is the ceiling: a write account never runs `full_access`
/// (it gets `auto`), a read-only one never writes, and its denial says it
/// is the account.
#[tokio::test]
async fn bn12_the_account_caps_the_run_mode() {
    if fake().is_none() {
        return eprintln!("python3 or fake_claude.py missing: skipped");
    }
    for (cli, bypass) in [
        (CliKind::Claude, "bypassPermissions"),
        (CliKind::Codex, "danger-full-access"),
    ] {
        let case = Case {
            cli,
            mode: RunMode::FullAccess,
            sandbox: SandboxMode::Write,
            surface: false,
            model: "sonnet",
        };
        let row = run(&case).await;
        bench(&case, &row, 0);
        assert!(!row.permission_flags.contains(bypass), "{cli}: {row:?}");
        assert_eq!(row.questions, 0, "{cli} runs auto: {row:?}");
    }
    let case = Case {
        cli: CliKind::Claude,
        mode: RunMode::FullAccess,
        sandbox: SandboxMode::ReadOnly,
        surface: false,
        model: "sonnet",
    };
    let row = run(&case).await;
    bench(&case, &row, 1);
    assert!(
        row.permission_flags.contains("--permission-mode plan"),
        "{row:?}"
    );
    assert!(row.denials >= 1 && row.explained == row.denials, "{row:?}");
}

/// The chat default (`auto_edits`, or no mode at all) launches exactly the
/// argv every turn had before 2.7: the missions baseline (B0) measures the
/// same command line.
#[test]
fn auto_edits_is_the_argv_of_before() {
    let root = folder("argv");
    let (store, account) = account(&root, CliKind::Claude, SandboxMode::Write);
    let runtime = CliRuntime::new(store, Arc::new(CliCapacity::new()));
    let req = TurnRequest {
        model: ModelRef {
            provider: ProviderId::new("cli"),
            model: "sonnet".into(),
        },
        messages: vec![Message::text(Role::User, "hi")],
        tools: vec![],
        params: GenParams::default(),
        cancel: CancellationToken::new(),
        agent_id: None,
    };
    let plan = |run_mode| LaunchPlan {
        cwd: Some(root.clone()),
        projectless: false,
        resume: None,
        resume_kind: crate::core::assist::runs::ResumeKind::New,
        mcp_config: None,
        permission_prompt_tool: None,
        restrict_tools: true,
        external: false,
        lean_project: true,
        effort_flag: true,
        run_mode,
        auto_mode: true,
        approve_for_me: true,
    };
    let argv = |p: &LaunchPlan| {
        runtime
            .plan_with(&account, PathBuf::from("claude"), &req, None, Some(p))
            .0
            .args
    };
    let none = argv(&plan(None));
    assert_eq!(none, argv(&plan(Some(RunMode::AutoEdits))));
    let joined = none.join(" ");
    assert!(
        joined.starts_with(
            "-p --output-format stream-json --verbose --permission-prompts none --permission-mode acceptEdits"
        ),
        "{joined}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
