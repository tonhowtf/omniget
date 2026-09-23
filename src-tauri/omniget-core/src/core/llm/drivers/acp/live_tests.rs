//! Live tests against real agents (ignored by default; nothing is installed
//! globally). Run with, for example:
//!
//! ```text
//! OMNIGET_ACP_LIVE_HOME=/tmp/acp-home OMNIGET_ACP_LIVE_CWD=/tmp/acp-work \
//!   cargo test -p omniget-core --lib drivers::acp::live_tests -- --ignored --nocapture
//! ```
//!
//! The agents run through `npx -y` with `HOME`/XDG pointed at the temporary
//! home, so no login or config of the machine is touched.

use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::mpsc;

use super::*;
use crate::core::llm::drivers::{EnvVar, RequestType, RuntimeEvent};

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
        v("GEMINI_API_KEY", String::new()),
        v("GOOGLE_API_KEY", String::new()),
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

fn setup() -> Option<(String, PathBuf)> {
    let home = std::env::var("OMNIGET_ACP_LIVE_HOME").ok()?;
    let cwd = PathBuf::from(std::env::var("OMNIGET_ACP_LIVE_CWD").ok()?);
    std::fs::create_dir_all(&home).ok()?;
    std::fs::create_dir_all(&cwd).ok()?;
    Some((home, cwd))
}

async fn next_matching(
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

fn save(log: &[RuntimeEvent], name: &str) {
    if let Ok(dir) = std::env::var("OMNIGET_ACP_LIVE_OUT") {
        let body: String = log
            .iter()
            .map(|e| serde_json::to_string(e).unwrap() + "\n")
            .collect();
        let _ = std::fs::write(PathBuf::from(dir).join(name), body);
    }
}

#[tokio::test]
#[ignore]
async fn opencode_acp_turn_with_approvals() {
    let Some((home, cwd)) = setup() else {
        eprintln!("OMNIGET_ACP_LIVE_HOME / _CWD not set");
        return;
    };
    std::fs::write(cwd.join("README.txt"), "hello from fixture\n").unwrap();
    let mut inst = DriverInstance::new("acp-opencode", KIND, "OpenCode");
    inst.command = Some("npx".into());
    inst.args = vec!["-y".into(), "opencode-ai".into(), "acp".into()];
    inst.env = env_of(&home);
    inst.env.push(EnvVar {
        name: "OPENCODE_CONFIG_CONTENT".into(),
        value: r#"{"permission":{"edit":"ask","bash":"ask"}}"#.into(),
        sensitive: false,
        value_redacted: false,
    });
    let (tx, mut rx) = mpsc::unbounded_channel();
    let driver = AcpDriver::new(inst, tx).unwrap();
    let mut log = Vec::new();
    driver
        .start_session(SessionStart {
            thread_id: "thr_live".into(),
            instance_id: "acp-opencode".into(),
            cwd: Some(cwd.clone()),
            model: None,
            agent_id: None,
            access_mode: AccessMode::ApprovalRequired,
            interaction_mode: InteractionMode::Default,
            resume_cursor: None,
        })
        .await
        .expect("session");
    let started = next_matching(&mut rx, &mut log, 30, |e| {
        e.type_name() == "session.started"
    })
    .await
    .expect("session.started");
    let cursor = match &started.kind {
        RuntimeEventKind::SessionStarted(p) => p.resume.clone().unwrap(),
        _ => unreachable!(),
    };
    driver
        .start_turn(TurnStart {
            thread_id: "thr_live".into(),
            turn_id: "turn-1".into(),
            message_id: "m1".into(),
            text: "Edit README.txt so it says 'driver edited' (use your edit tool), then run the shell command: echo done. Reply in 3 words.".into(),
            attachments: vec![],
            model: None,
            access_mode: AccessMode::ApprovalRequired,
            interaction_mode: InteractionMode::Default,
        })
        .await
        .expect("turn");
    let mut approvals = 0;
    loop {
        let ev = next_matching(&mut rx, &mut log, 240, |e| {
            matches!(e.type_name(), "request.opened" | "turn.completed")
        })
        .await
        .expect("event");
        match &ev.kind {
            RuntimeEventKind::RequestOpened(p) => {
                approvals += 1;
                assert!(matches!(
                    p.request_type,
                    RequestType::FileChangeApproval | RequestType::CommandExecutionApproval
                ));
                if p.request_type == RequestType::FileChangeApproval {
                    assert!(p
                        .detail
                        .as_ref()
                        .unwrap()
                        .diff
                        .as_deref()
                        .unwrap_or("")
                        .contains("+driver edited"));
                }
                driver
                    .respond_request(
                        "thr_live",
                        ev.request_id.as_deref().unwrap(),
                        ApprovalDecision::Accept,
                    )
                    .await
                    .unwrap();
            }
            RuntimeEventKind::TurnCompleted(p) => {
                assert_eq!(p.state, TurnEndState::Completed);
                break;
            }
            _ => {}
        }
    }
    assert!(approvals >= 1, "at least one approval");
    assert_eq!(
        std::fs::read_to_string(cwd.join("README.txt"))
            .unwrap()
            .trim(),
        "driver edited"
    );
    // Resume: stop, start again with the cursor → session/load.
    driver.stop("thr_live").await.unwrap();
    driver
        .start_session(SessionStart {
            thread_id: "thr_live".into(),
            instance_id: "acp-opencode".into(),
            cwd: Some(cwd.clone()),
            model: None,
            agent_id: None,
            access_mode: AccessMode::FullAccess,
            interaction_mode: InteractionMode::Default,
            resume_cursor: Some(cursor.clone()),
        })
        .await
        .expect("resume");
    let again = next_matching(&mut rx, &mut log, 60, |e| {
        e.type_name() == "session.started"
    })
    .await
    .expect("resumed");
    match &again.kind {
        RuntimeEventKind::SessionStarted(p) => {
            assert_eq!(p.resume.as_ref().unwrap()["sessionId"], cursor["sessionId"])
        }
        _ => unreachable!(),
    }
    driver
        .start_turn(TurnStart {
            thread_id: "thr_live".into(),
            turn_id: "turn-2".into(),
            message_id: "m2".into(),
            text:
                "What exact text did you write into README.txt earlier? Answer with just that text."
                    .into(),
            attachments: vec![],
            model: None,
            access_mode: AccessMode::FullAccess,
            interaction_mode: InteractionMode::Default,
        })
        .await
        .expect("turn 2");
    next_matching(&mut rx, &mut log, 240, |e| {
        e.type_name() == "turn.completed"
    })
    .await
    .expect("turn 2 done");
    let answer: String = log
        .iter()
        .filter(|e| e.turn_id.as_deref() == Some("turn-2"))
        .filter_map(|e| match &e.kind {
            RuntimeEventKind::ContentDelta(p)
                if p.stream_kind == crate::core::llm::drivers::StreamKind::AssistantText =>
            {
                Some(p.delta.clone())
            }
            _ => None,
        })
        .collect();
    println!("turn 2 answer: {answer}");
    // Free models are terse and sometimes odd; the resumed session must at
    // least answer (the replayed history is not re-emitted as new items).
    assert!(!answer.trim().is_empty(), "resumed session answers");
    assert!(!log
        .iter()
        .any(|e| e.turn_id.is_none() && e.type_name() == "item.started"));
    // Interrupt an ongoing turn.
    driver
        .start_turn(TurnStart {
            thread_id: "thr_live".into(),
            turn_id: "turn-3".into(),
            message_id: "m3".into(),
            text:
                "Count slowly from 1 to 300, one number per line, with a short sentence for each."
                    .into(),
            attachments: vec![],
            model: None,
            access_mode: AccessMode::FullAccess,
            interaction_mode: InteractionMode::Default,
        })
        .await
        .expect("turn 3");
    next_matching(&mut rx, &mut log, 120, |e| e.type_name() == "content.delta")
        .await
        .expect("streaming");
    driver.interrupt("thr_live", Some("turn-3")).await.unwrap();
    let end = next_matching(&mut rx, &mut log, 60, |e| e.type_name() == "turn.completed")
        .await
        .expect("interrupted");
    match &end.kind {
        RuntimeEventKind::TurnCompleted(p) => assert_eq!(p.state, TurnEndState::Interrupted),
        _ => unreachable!(),
    }
    driver.stop("thr_live").await.unwrap();
    save(&log, "opencode-acp-driver.ndjson");
}

#[tokio::test]
#[ignore]
async fn gemini_without_login_stops_at_authenticate() {
    let Some((home, cwd)) = setup() else {
        eprintln!("OMNIGET_ACP_LIVE_HOME / _CWD not set");
        return;
    };
    let mut inst = DriverInstance::new("acp-gemini", KIND, "Gemini CLI");
    inst.command = Some("npx".into());
    inst.args = vec!["-y".into(), "@google/gemini-cli".into(), "--acp".into()];
    inst.env = env_of(&home);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let driver = AcpDriver::new(inst, tx).unwrap();
    let err = driver
        .start_session(SessionStart {
            thread_id: "thr_gem".into(),
            instance_id: "acp-gemini".into(),
            cwd: Some(cwd),
            model: None,
            agent_id: None,
            access_mode: AccessMode::ApprovalRequired,
            interaction_mode: InteractionMode::Default,
            resume_cursor: None,
        })
        .await
        .expect_err("needs login");
    println!("error: {err}");
    assert!(err.message.contains(ERR_ACP_AUTH));
    let mut log = Vec::new();
    let auth = next_matching(&mut rx, &mut log, 5, |e| e.type_name() == "auth.status")
        .await
        .expect("auth.status");
    match &auth.kind {
        RuntimeEventKind::AuthStatus(p) => {
            assert!(p.output.iter().any(|l| l.starts_with("$ ")));
            assert!(p.output.iter().any(|l| l.contains("oauth-personal")));
        }
        _ => unreachable!(),
    }
    save(&log, "gemini-acp-driver.ndjson");
}
