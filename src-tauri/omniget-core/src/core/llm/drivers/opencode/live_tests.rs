//! Live test against a real `opencode serve` (ignored by default). Same
//! environment variables as the ACP live tests:
//!
//! ```text
//! OMNIGET_ACP_LIVE_HOME=/tmp/acp-home OMNIGET_ACP_LIVE_CWD=/tmp/acp-work \
//!   cargo test -p omniget-core --lib drivers::opencode::live_tests -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::mpsc;

use super::*;
use crate::core::llm::drivers::{EnvVar, RequestType};

fn env_of(home: &str) -> Vec<EnvVar> {
    let v = |k: &str, val: String| EnvVar {
        name: k.into(),
        value: val,
        sensitive: false,
        value_redacted: false,
    };
    let mut out = vec![
        v("HOME", home.to_string()),
        v("XDG_CONFIG_HOME", format!("{home}/.config")),
        v("XDG_DATA_HOME", format!("{home}/.local/share")),
        v("XDG_CACHE_HOME", format!("{home}/.cache")),
        v("XDG_STATE_HOME", format!("{home}/.local/state")),
        v("npm_config_cache", format!("{home}/.npm")),
    ];
    if let Ok(extra) = std::env::var("OMNIGET_ACP_LIVE_EXTRA_ENV") {
        for pair in extra.split(';') {
            if let Some((k, val)) = pair.split_once('=') {
                out.push(v(k, val.to_string()));
            }
        }
    }
    out
}

async fn until(
    rx: &mut mpsc::UnboundedReceiver<RuntimeEvent>,
    log: &mut Vec<RuntimeEvent>,
    secs: u64,
    pred: impl Fn(&RuntimeEvent) -> bool,
) -> Option<RuntimeEvent> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let ev = tokio::time::timeout_at(deadline, rx.recv()).await.ok()??;
        println!("{}", serde_json::to_string(&ev).unwrap());
        log.push(ev.clone());
        if pred(&ev) {
            return Some(ev);
        }
    }
}

fn turn(thread: &str, id: &str, text: &str, access: AccessMode) -> TurnStart {
    TurnStart {
        thread_id: thread.into(),
        turn_id: id.into(),
        message_id: format!("m-{id}"),
        text: text.into(),
        attachments: vec![],
        model: None,
        access_mode: access,
        interaction_mode: InteractionMode::Default,
    }
}

#[tokio::test]
#[ignore]
async fn opencode_serve_turns_approvals_rollback_fork_interrupt() {
    let (Ok(home), Ok(cwd)) = (
        std::env::var("OMNIGET_ACP_LIVE_HOME"),
        std::env::var("OMNIGET_ACP_LIVE_CWD"),
    ) else {
        eprintln!("OMNIGET_ACP_LIVE_HOME / _CWD not set");
        return;
    };
    let cwd = PathBuf::from(cwd);
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join("README.txt"), "hello from fixture\n").unwrap();
    let mut inst = DriverInstance::new("opencode", KIND, "OpenCode");
    inst.command = Some("npx".into());
    inst.args = vec!["-y".into(), "opencode-ai".into()];
    inst.env = env_of(&home);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let driver = OpenCodeDriver::new(inst, tx).unwrap();
    let mut log = Vec::new();
    let start = |thread: &str, cursor: Option<Value>| SessionStart {
        thread_id: thread.into(),
        instance_id: "opencode".into(),
        cwd: Some(cwd.clone()),
        model: None,
        agent_id: None,
        access_mode: AccessMode::ApprovalRequired,
        interaction_mode: InteractionMode::Default,
        resume_cursor: cursor,
    };
    driver
        .start_session(start("thr_oc", None))
        .await
        .expect("session");
    until(&mut rx, &mut log, 30, |e| {
        e.type_name() == "session.started"
    })
    .await
    .expect("started");

    // Turn 1: an edit and a command, both approved through respond_request.
    driver
        .start_turn(turn("thr_oc", "t1", "Edit README.txt so it says 'serve driver' (use the edit tool), then run the shell command: echo ok. Reply in 3 words.", AccessMode::ApprovalRequired))
        .await
        .expect("turn 1");
    let mut kinds = Vec::new();
    loop {
        let ev = until(&mut rx, &mut log, 240, |e| {
            matches!(e.type_name(), "request.opened" | "turn.completed")
        })
        .await
        .expect("event");
        match &ev.kind {
            RuntimeEventKind::RequestOpened(p) => {
                kinds.push(p.request_type);
                driver
                    .respond_request(
                        "thr_oc",
                        ev.request_id.as_deref().unwrap(),
                        ApprovalDecision::Accept,
                    )
                    .await
                    .expect("reply");
            }
            RuntimeEventKind::TurnCompleted(p) => {
                assert_eq!(p.state, TurnEndState::Completed, "{p:?}");
                assert!(p.usage.as_ref().unwrap().output_tokens > 0);
                break;
            }
            _ => {}
        }
    }
    assert!(
        kinds.contains(&RequestType::FileChangeApproval),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&RequestType::CommandExecutionApproval),
        "{kinds:?}"
    );
    assert_eq!(
        std::fs::read_to_string(cwd.join("README.txt"))
            .unwrap()
            .trim(),
        "serve driver"
    );

    // Turn 2, then roll back to 1 turn: the new session keeps one user message.
    driver
        .start_turn(turn(
            "thr_oc",
            "t2",
            "Reply with exactly: two",
            AccessMode::FullAccess,
        ))
        .await
        .expect("turn 2");
    until(&mut rx, &mut log, 240, |e| {
        e.type_name() == "turn.completed"
    })
    .await
    .expect("t2 done");
    let before = driver.live("thr_oc").await.unwrap().sid();
    driver.rollback("thr_oc", 1).await.expect("rollback");
    let s = driver.live("thr_oc").await.unwrap();
    assert_ne!(s.sid(), before);
    assert_eq!(s.user_message_ids().await.unwrap().len(), 1);

    // Fork the thread into another one (keep 1 turn) and start it.
    driver.fork("thr_oc", "thr_fork", 1).await.expect("fork");
    driver
        .start_session(start("thr_fork", None))
        .await
        .expect("fork session");
    let f = driver.live("thr_fork").await.unwrap();
    assert_eq!(f.user_message_ids().await.unwrap().len(), 1);
    driver.stop("thr_fork").await.unwrap();

    // Interrupt a long turn.
    driver
        .start_turn(turn(
            "thr_oc",
            "t3",
            "Count slowly from 1 to 300, one number per line, each with a short sentence.",
            AccessMode::FullAccess,
        ))
        .await
        .expect("turn 3");
    until(&mut rx, &mut log, 120, |e| {
        e.type_name() == "content.delta" && e.turn_id.as_deref() == Some("t3")
    })
    .await
    .expect("streaming");
    driver
        .interrupt("thr_oc", Some("t3"))
        .await
        .expect("interrupt");
    let end = until(&mut rx, &mut log, 60, |e| {
        e.type_name() == "turn.completed" && e.turn_id.as_deref() == Some("t3")
    })
    .await
    .expect("t3 done");
    match &end.kind {
        RuntimeEventKind::TurnCompleted(p) => assert_eq!(p.state, TurnEndState::Interrupted),
        _ => unreachable!(),
    }
    driver.stop("thr_oc").await.unwrap();
    if let Ok(dir) = std::env::var("OMNIGET_ACP_LIVE_OUT") {
        let body: String = log
            .iter()
            .map(|e| serde_json::to_string(e).unwrap() + "\n")
            .collect();
        let _ = std::fs::write(
            PathBuf::from(dir).join("opencode-serve-driver.ndjson"),
            body,
        );
    }
}
