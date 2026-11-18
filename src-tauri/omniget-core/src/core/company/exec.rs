use std::time::Duration;

use serde_json::{json, Value};

use super::heartbeat::{Claim, RunOutcome};

const OUT_MAX: usize = 1024 * 1024;

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
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(ws) = workspace {
        cmd.current_dir(ws);
    }
    let limit = Duration::from_secs(timeout_sec.unwrap_or(600).clamp(1, 24 * 3600));
    run_bounded(cmd, command, body.to_string(), limit).await
}

async fn run_bounded(
    mut cmd: tokio::process::Command,
    command: &str,
    payload: String,
    limit: Duration,
) -> RunOutcome {
    use crate::core::llm::drivers::acp::client::read_tail;
    use tokio::io::AsyncWriteExt;
    let spawn_failed = |e: std::io::Error| RunOutcome {
        state: "failed".into(),
        failure_class: Some("spawn".into()),
        error: Some(format!("{command}: {e}")),
        ..Default::default()
    };
    #[cfg(unix)]
    cmd.process_group(0);
    crate::core::process::suspend_on_spawn(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return spawn_failed(e),
    };
    let mut tree = match crate::core::process::ProcessTreeGuard::new(&child) {
        Ok(t) => t,
        Err(e) => return spawn_failed(e),
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let waited = tokio::time::timeout(limit, async {
        let feed = async {
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(payload.as_bytes()).await;
            }
        };
        let (_, out, err, status) = tokio::join!(
            feed,
            read_tail(stdout, OUT_MAX),
            read_tail(stderr, OUT_MAX),
            child.wait()
        );
        (out, err, status)
    })
    .await;
    match waited {
        Err(_) => {
            tree.terminate();
            RunOutcome {
                state: "timed_out".into(),
                failure_class: Some("deadline".into()),
                error: Some("the command ran past its timeout".into()),
                ..Default::default()
            }
        }
        Ok((_, _, Err(e))) => {
            tree.terminate();
            spawn_failed(e)
        }
        Ok((out, err, Ok(status))) => {
            tree.release();
            let stdout = out.text().to_string();
            if status.success() {
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
                        status.code().unwrap_or(-1),
                        err.text().chars().take(2000).collect::<String>()
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> tokio::process::Command {
        let mut c = tokio::process::Command::new("/bin/sh");
        c.arg("-c")
            .arg(script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        c
    }

    #[tokio::test]
    async fn the_body_reaches_stdin_and_stdout_is_the_summary() {
        let out = run_bounded(sh("cat"), "sh", "hello".into(), Duration::from_secs(20)).await;
        assert_eq!(out.state, "succeeded");
        assert_eq!(out.summary.as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn the_summary_keeps_only_the_tail() {
        let out = run_bounded(
            sh("head -c 3000000 /dev/zero | tr '\\0' 'a'; printf END"),
            "sh",
            String::new(),
            Duration::from_secs(30),
        )
        .await;
        let summary = out.summary.unwrap_or_default();
        assert_eq!(summary.len(), OUT_MAX);
        assert!(summary.ends_with("END"));
    }

    #[tokio::test]
    async fn a_timeout_kills_the_children_of_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("pid");
        let script = format!("sleep 60 & echo $! > '{}'; wait", mark.display());
        let out = run_bounded(
            sh(&script),
            "sh",
            String::new(),
            Duration::from_millis(1500),
        )
        .await;
        assert_eq!(out.state, "timed_out");
        let pid: i32 = std::fs::read_to_string(&mark)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut alive = true;
        for _ in 0..50 {
            // SAFETY: signal 0 only probes whether the pid exists.
            alive = unsafe { libc::kill(pid, 0) } == 0;
            if !alive {
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        assert!(!alive);
    }
}
