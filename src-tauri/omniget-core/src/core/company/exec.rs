//! Executors of the non-roster adapters: a local `process` (the heartbeat as
//! JSON on stdin, stdout = summary) and an `http` endpoint (POST the
//! heartbeat, body = summary). They live in the core so the engine and the
//! benches run the same code.

use std::time::Duration;

use serde_json::{json, Value};

use super::heartbeat::{Claim, RunOutcome};

/// The JSON a process or HTTP agent receives.
pub fn heartbeat_json(claim: &Claim, prompt: &str) -> Value {
    json!({
        "run_id": claim.run.id,
        "company_id": claim.staff.company_id,
        "staff": {"id": claim.staff.id, "name": claim.staff.name, "role": claim.staff.role},
        "issue": claim.issue,
        "wakes": claim.wakes,
        "prompt": prompt,
    })
}

pub async fn run_process(
    claim: &Claim,
    body: &Value,
    command: &str,
    args: &[String],
    workspace: Option<&str>,
    timeout_sec: Option<u64>,
    env: &[(String, String)],
) -> RunOutcome {
    use tokio::io::AsyncWriteExt;
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args)
        .env("OMNIGET_COMPANY_ID", &claim.staff.company_id)
        .env("OMNIGET_STAFF_ID", &claim.staff.id)
        .env("OMNIGET_RUN_ID", &claim.run.id)
        .env(
            "OMNIGET_ISSUE_ID",
            claim.issue.as_ref().map(|i| i.id.as_str()).unwrap_or(""),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Only the secrets bound to this agent, never the app's whole store.
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(ws) = workspace {
        cmd.current_dir(ws);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return RunOutcome {
                state: "failed".into(),
                failure_class: Some("spawn".into()),
                error: Some(format!("{command}: {e}")),
                ..Default::default()
            }
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.to_string().as_bytes()).await;
    }
    let limit = Duration::from_secs(timeout_sec.unwrap_or(600).clamp(1, 24 * 3600));
    match tokio::time::timeout(limit, child.wait_with_output()).await {
        Err(_) => RunOutcome {
            state: "timed_out".into(),
            failure_class: Some("deadline".into()),
            error: Some("the command ran past its timeout".into()),
            ..Default::default()
        },
        Ok(Err(e)) => RunOutcome {
            state: "failed".into(),
            failure_class: Some("spawn".into()),
            error: Some(format!("{command}: {e}")),
            ..Default::default()
        },
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            if out.status.success() {
                RunOutcome {
                    state: "succeeded".into(),
                    summary: Some(stdout),
                    ..Default::default()
                }
            } else {
                RunOutcome {
                    state: "failed".into(),
                    failure_class: Some("agent_error".into()),
                    error: Some(format!(
                        "exit {}: {}",
                        out.status.code().unwrap_or(-1),
                        String::from_utf8_lossy(&out.stderr)
                            .chars()
                            .take(2000)
                            .collect::<String>()
                    )),
                    summary: Some(stdout),
                    ..Default::default()
                }
            }
        }
    }
}

pub async fn run_http(body: &Value, url: &str, timeout_sec: Option<u64>) -> RunOutcome {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(
            timeout_sec.unwrap_or(120).clamp(1, 3600),
        ))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return RunOutcome {
                state: "failed".into(),
                failure_class: Some("spawn".into()),
                error: Some(e.to_string()),
                ..Default::default()
            }
        }
    };
    match client.post(url).json(body).send().await {
        Err(e) => RunOutcome {
            state: "failed".into(),
            failure_class: Some("network".into()),
            error: Some(e.to_string()),
            ..Default::default()
        },
        Ok(r) => {
            let status = r.status();
            let text = r.text().await.unwrap_or_default();
            if status.is_success() {
                RunOutcome {
                    state: "succeeded".into(),
                    summary: Some(text),
                    ..Default::default()
                }
            } else {
                let class = if status.as_u16() == 429 {
                    "rate_limit"
                } else if status.is_server_error() {
                    "network"
                } else {
                    "agent_error"
                };
                RunOutcome {
                    state: "failed".into(),
                    failure_class: Some(class.into()),
                    error: Some(format!(
                        "HTTP {status}: {}",
                        text.chars().take(500).collect::<String>()
                    )),
                    ..Default::default()
                }
            }
        }
    }
}
