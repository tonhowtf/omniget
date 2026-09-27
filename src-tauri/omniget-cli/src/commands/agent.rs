//! `omniget agent run|loop|jobs|explain|status|diff|agents`: talks to the running desktop app
//! through the local bridge (`127.0.0.1:<port>`, bearer from `settings.json`).
//! The app owns the jobs, so a run started here survives this process.

use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

struct Bridge {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Bridge {
    fn open() -> anyhow::Result<Self> {
        let dir =
            omniget_core::core::paths::app_data_dir().ok_or_else(|| anyhow!("no app data dir"))?;
        let raw = std::fs::read_to_string(dir.join("settings.json"))
            .context("settings.json not found: open the OmniGet app once")?;
        let json: Value = serde_json::from_str(&raw)?;
        let bridge = &json["app_settings"]["bridge"];
        let port = bridge["port"]
            .as_u64()
            .filter(|p| *p > 0)
            .ok_or_else(|| anyhow!("the app has no bridge port yet: open OmniGet once"))?;
        let token = bridge["token"].as_str().unwrap_or("").to_string();
        Ok(Self {
            base: format!("http://127.0.0.1:{port}"),
            token,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()?,
        })
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        let res = req
            .bearer_auth(&self.token)
            .send()
            .await
            .context("the OmniGet app is not running (the bridge did not answer)")?;
        let status = res.status();
        let body: Value = res.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            bail!(
                "{}: {}",
                status,
                body["error"].as_str().unwrap_or("request failed")
            );
        }
        Ok(body)
    }

    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.send(self.http.get(format!("{}{path}", self.base)))
            .await
    }

    /// A text body (`format=patch`); an error status still reads the JSON
    /// `error` the bridge sends.
    async fn get_text(&self, path: &str) -> anyhow::Result<String> {
        let res = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("the OmniGet app is not running (the bridge did not answer)")?;
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        if !status.is_success() {
            let why = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["error"].as_str().map(String::from))
                .unwrap_or_else(|| "request failed".into());
            bail!("{status}: {why}");
        }
        Ok(body)
    }

    async fn post(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        self.send(self.http.post(format!("{}{path}", self.base)).json(&body))
            .await
    }
}

fn workspace(arg: Option<String>, no_workspace: bool) -> Option<String> {
    if no_workspace {
        return None;
    }
    arg.or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    })
}

fn print_job_line(j: &Value) {
    println!(
        "{:<12} {:<17} {:<8} {:<16} {}",
        j["id"].as_str().unwrap_or(""),
        j["state"].as_str().unwrap_or(""),
        j["kind"].as_str().unwrap_or(""),
        j["agent_id"].as_str().unwrap_or(""),
        j["prompt"]
            .as_str()
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(60)
            .collect::<String>()
    );
}

async fn follow(bridge: &Bridge, id: &str, json_out: bool) -> anyhow::Result<()> {
    let mut last_state = String::new();
    let mut seen_log = 0usize;
    loop {
        let job = bridge.get(&format!("/v1/jobs/{id}")).await?;
        let state = job["state"].as_str().unwrap_or("").to_string();
        let log = job["log"].as_str().unwrap_or("");
        if !json_out {
            if log.len() > seen_log && log.is_char_boundary(seen_log) {
                eprint!("{}", &log[seen_log..]);
            }
            seen_log = log.len();
            if state != last_state {
                eprintln!("[{state}]");
                if matches!(state.as_str(), "queued" | "waiting_workspace") {
                    if let Some(why) = job["wait_reason"].as_str() {
                        let text = match why.split_once(':') {
                            Some(("workspace", holder)) => {
                                format!("its folder is busy: job {holder} is writing there")
                            }
                            _ if why == "agent_cap" => "the agent is at its job cap".to_string(),
                            _ => "all job slots are busy".to_string(),
                        };
                        eprintln!("  waiting: {text} (`omniget agent status` shows the queue)");
                    }
                }
                if state == "waiting_approval" {
                    eprintln!("  the agent is asking for permission: answer in the app (LLM → Jobs) or on the pet");
                    eprintln!("  (`omniget agent explain {id}` shows the question and when it expires)");
                }
                last_state = state.clone();
            }
        }
        if matches!(state.as_str(), "done" | "failed" | "cancelled") {
            if json_out {
                println!("{}", serde_json::to_string_pretty(&job)?);
            } else {
                if let Some(text) = job["result"].as_str().filter(|t| !t.is_empty()) {
                    println!("{text}");
                }
                if let Some(err) = job["error"].as_str() {
                    eprintln!("error: {err}");
                }
                let u = &job["usage"];
                if u.is_object() {
                    let cost = u["cost_usd"]
                        .as_f64()
                        .map(|c| format!(", ${c:.4}"))
                        .unwrap_or_default();
                    eprintln!(
                        "[{}: {} tokens in, {} out{cost}]",
                        u["model"].as_str().unwrap_or("?"),
                        u["input_tokens"].as_u64().unwrap_or(0),
                        u["output_tokens"].as_u64().unwrap_or(0),
                    );
                }
            }
            if state != "done" {
                std::process::exit(1);
            }
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
}

/// `omniget agent explain` in plain lines: what the watchdog and the deadline
/// see right now, so "why is it still running?" has an answer without the app.
fn explain_text(v: &Value) -> String {
    let mut out = String::new();
    let secs = |k: &str| v[k].as_u64().map(|s| format!("{s} s"));
    out.push_str(&format!(
        "{}  {} (seq {})  {}\n",
        v["id"].as_str().unwrap_or(""),
        v["state"].as_str().unwrap_or(""),
        v["state_seq"],
        v["kind"].as_str().unwrap_or("")
    ));
    if let Some(reason) = v["failure_reason"].as_str() {
        out.push_str(&format!("  failure_reason: {reason}\n"));
    }
    if let Some(age) = secs("last_event_age_s") {
        out.push_str(&format!("  last_event_age: {age}\n"));
    }
    out.push_str(&format!(
        "  ask_pending: {}\n",
        v["ask_pending"].as_bool().unwrap_or(false)
    ));
    for a in v["asks"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "    {}  asked {} s ago, expires in {} s  {}\n",
            a["tool"].as_str().unwrap_or("?"),
            a["age_s"].as_u64().unwrap_or(0),
            a["expires_in_s"]
                .as_u64()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "?".into()),
            a["preview"].as_str().unwrap_or("").chars().take(60).collect::<String>()
        ));
    }
    if let Some(left) = secs("deadline_left_s") {
        out.push_str(&format!("  deadline_left: {left}\n"));
    }
    if let Some(pid) = v["pid"].as_i64() {
        out.push_str(&format!("  pid: {pid}\n"));
    }
    out.push_str(&format!(
        "  holds_slot: {}\n",
        v["slot"].as_bool().unwrap_or(false)
    ));
    if let Some(err) = v["error"].as_str() {
        out.push_str(&format!("  error: {err}\n"));
    }
    out
}

/// `omniget agent diff` in plain lines: one line per file (status, `+/-` or
/// `binary`, path) and the totals; `Err` with the reason when the job has
/// no diff (never an empty list standing for "unknown").
fn diff_text(v: &Value) -> Result<String, String> {
    if !v["available"].as_bool().unwrap_or(false) {
        return Err(format!(
            "no diff for job {}: {} ({})",
            v["job_id"].as_str().unwrap_or(""),
            v["detail"].as_str().unwrap_or("unavailable"),
            v["reason"].as_str().unwrap_or("unavailable")
        ));
    }
    let files: Vec<&Value> = v["files"].as_array().into_iter().flatten().collect();
    let mut out = String::new();
    for f in &files {
        let counts = if f["binary"].as_bool().unwrap_or(false) {
            "binary".to_string()
        } else {
            format!(
                "+{} -{}",
                f["additions"].as_u64().unwrap_or(0),
                f["deletions"].as_u64().unwrap_or(0)
            )
        };
        out.push_str(&format!(
            "{}  {:<12} {}\n",
            f["status"].as_str().unwrap_or("?"),
            counts,
            f["path"].as_str().unwrap_or("")
        ));
    }
    let count = v["file_count"].as_u64().unwrap_or(files.len() as u64);
    out.push_str(&format!(
        "{count} file{} changed, +{} -{}",
        if count == 1 { "" } else { "s" },
        v["additions"].as_u64().unwrap_or(0),
        v["deletions"].as_u64().unwrap_or(0)
    ));
    if v["clipped"].as_bool().unwrap_or(false) {
        out.push_str(" (the patch or the list was cut; see --patch)");
    }
    if !v["complete"].as_bool().unwrap_or(true) {
        out.push_str(" (still running: against the folder as it is now)");
    }
    out.push('\n');
    Ok(out)
}

/// `omniget agent status` in plain lines: what runs, what waits for a
/// person, and every queued job with its place and the reason it waits, so
/// "is it stuck?" has an answer (W7).
fn status_text(v: &Value) -> String {
    let l = &v["limits"];
    let mut out = format!(
        "{} of {} slots in use",
        l["in_use"].as_u64().unwrap_or(0),
        l["max_parallel"].as_u64().unwrap_or(0)
    );
    if let Some(per) = l["max_per_agent"].as_u64() {
        out.push_str(&format!(", {per} per agent"));
    }
    out.push('\n');
    let line = |j: &Value| {
        format!(
            "{:<12} {:<8} {:<16} {}",
            j["id"].as_str().unwrap_or(""),
            j["kind"].as_str().unwrap_or(""),
            j["agent_id"].as_str().unwrap_or(""),
            j["workspace"].as_str().unwrap_or("")
        )
    };
    for j in v["running"].as_array().into_iter().flatten() {
        out.push_str(&format!("  running  {}\n", line(j).trim_end()));
    }
    for j in v["blocked"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "  blocked  {}  (waiting for you: answer in the app)\n",
            line(j).trim_end()
        ));
    }
    let queued: Vec<&Value> = v["queued"].as_array().into_iter().flatten().collect();
    if queued.is_empty() {
        out.push_str("  nothing queued\n");
    }
    for q in queued {
        out.push_str(&format!(
            "  {}. {:<12} {:<8} {:<16} {}\n",
            q["position"].as_u64().unwrap_or(0),
            q["id"].as_str().unwrap_or(""),
            q["kind"].as_str().unwrap_or(""),
            q["agent_id"].as_str().unwrap_or(""),
            q["detail"].as_str().unwrap_or(q["reason"].as_str().unwrap_or(""))
        ));
    }
    out
}

#[derive(clap::Subcommand)]
pub enum AgentCommand {
    /// Run one task with an agent of the roster (in the current folder)
    Run {
        prompt: String,
        #[arg(short, long, default_value = "omni")]
        agent: String,
        #[arg(short, long, help = "Workspace folder (default: current dir)")]
        workspace: Option<String>,
        #[arg(long, help = "Run without a workspace (no file or shell tools)")]
        no_workspace: bool,
        #[arg(long, help = "Return the job id at once instead of following it")]
        detach: bool,
        #[arg(
            long,
            value_name = "MIN",
            help = "Stop the job after this many minutes (default: the app setting, 60)"
        )]
        deadline_min: Option<u32>,
    },
    /// Keep an agent working until a check passes or the budget runs out
    Loop {
        prompt: String,
        #[arg(short, long, default_value = "omni")]
        agent: String,
        #[arg(short, long)]
        workspace: Option<String>,
        #[arg(long, help = "Stop after this many rounds")]
        rounds: Option<u32>,
        #[arg(long, help = "Stop after this many minutes")]
        minutes: Option<u32>,
        #[arg(
            long,
            help = "Shell command; the loop ends when it exits 0 (e.g. \"npm test\")"
        )]
        check: Option<String>,
    },
    /// List recent jobs, show one, or cancel one
    Jobs {
        id: Option<String>,
        #[arg(long)]
        cancel: bool,
    },
    /// Why a job is in its state: last event age, pending question, deadline left
    Explain { id: String },
    /// What runs now, what waits (place in the queue and why) and the limits
    Status,
    /// What a job changed in its folder: files and +/-, or the patch
    Diff {
        id: String,
        #[arg(long, help = "Print the unified diff instead of the file list")]
        patch: bool,
    },
    /// List the running and finished Loops
    Loops,
    /// List the agents of the roster
    Agents,
}

pub async fn execute(cmd: AgentCommand, json_out: bool) -> anyhow::Result<()> {
    let bridge = Bridge::open()?;
    match cmd {
        AgentCommand::Run {
            prompt,
            agent,
            workspace: ws,
            no_workspace,
            detach,
            deadline_min,
        } => {
            let job = bridge
                .post("/v1/agent/run", json!({ "agent_id": agent, "prompt": prompt, "workspace": workspace(ws, no_workspace), "deadline_min": deadline_min }))
                .await?;
            let id = job["id"].as_str().unwrap_or("").to_string();
            if detach {
                println!("{id}");
                return Ok(());
            }
            follow(&bridge, &id, json_out).await
        }
        AgentCommand::Loop {
            prompt,
            agent,
            workspace: ws,
            rounds,
            minutes,
            check,
        } => {
            let l = bridge
                .post(
                    "/v1/agent/loop",
                    json!({ "agent_id": agent, "prompt": prompt, "workspace": workspace(ws, false), "max_rounds": rounds, "max_minutes": minutes, "check_command": check }),
                )
                .await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&l)?);
            } else {
                println!("loop {} started; it keeps running inside the app. Follow it with `omniget agent loops` or in LLM → Loops.", l["id"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        AgentCommand::Jobs {
            id: Some(id),
            cancel: true,
        } => {
            let job = bridge
                .post(&format!("/v1/jobs/{id}/cancel"), json!({}))
                .await?;
            print_job_line(&job);
            Ok(())
        }
        AgentCommand::Jobs { id: Some(id), .. } => follow(&bridge, &id, json_out).await,
        AgentCommand::Jobs { id: None, .. } => {
            let jobs = bridge.get("/v1/jobs").await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&jobs)?);
            } else {
                jobs.as_array()
                    .into_iter()
                    .flatten()
                    .for_each(print_job_line);
            }
            Ok(())
        }
        AgentCommand::Explain { id } => {
            let v = bridge.get(&format!("/v1/jobs/{id}/explain")).await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print!("{}", explain_text(&v));
            }
            Ok(())
        }
        AgentCommand::Diff { id, patch: true } => {
            print!("{}", bridge.get_text(&format!("/v1/jobs/{id}/diff?format=patch")).await?);
            Ok(())
        }
        AgentCommand::Diff { id, patch: false } => {
            let v = bridge.get(&format!("/v1/jobs/{id}/diff?format=files")).await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v)?);
                return Ok(());
            }
            match diff_text(&v) {
                Ok(text) => {
                    print!("{text}");
                    Ok(())
                }
                Err(why) => bail!(why),
            }
        }
        AgentCommand::Status => {
            let v = bridge.get("/v1/state").await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print!("{}", status_text(&v));
            }
            Ok(())
        }
        AgentCommand::Loops => {
            let loops = bridge.get("/v1/loops").await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&loops)?);
            } else {
                for l in loops.as_array().into_iter().flatten() {
                    println!(
                        "{:<12} {:<10} rounds {:<3} {}  {}",
                        l["id"].as_str().unwrap_or(""),
                        l["state"].as_str().unwrap_or(""),
                        l["rounds_done"],
                        l["stop_reason"].as_str().unwrap_or(""),
                        l["name"].as_str().unwrap_or("")
                    );
                }
            }
            Ok(())
        }
        AgentCommand::Agents => {
            let agents = bridge.get("/v1/agents").await?;
            for a in agents.as_array().into_iter().flatten() {
                println!(
                    "{:<24} {}",
                    a["id"].as_str().unwrap_or(""),
                    a["name"].as_str().unwrap_or("")
                );
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_text_shows_position_and_reason() {
        let v = json!({
            "running": [{"id": "j1", "agent_id": "claude", "kind": "run", "state": "running", "workspace": "/w/repo"}],
            "blocked": [{"id": "j2", "agent_id": "codex", "kind": "loop", "state": "waiting_approval", "reason": "approval"}],
            "queued": [
                {"id": "j3", "position": 1, "reason": "workspace", "holder": "j1", "detail": "folder busy by j1 (/w/repo)", "agent_id": "claude", "kind": "run"},
                {"id": "j4", "position": 2, "reason": "slot", "detail": "all 2 job slots are busy", "agent_id": "omni", "kind": "mission"}
            ],
            "limits": {"max_parallel": 2, "max_per_agent": null, "in_use": 2, "available": 0, "stagger_ms": 500}
        });
        let text = status_text(&v);
        assert!(text.contains("2 of 2 slots in use"), "{text}");
        assert!(text.contains("j1"), "{text}");
        assert!(text.contains("j2") && text.contains("waiting for you"), "{text}");
        assert!(text.contains("1. j3") && text.contains("folder busy by j1 (/w/repo)"), "{text}");
        assert!(text.contains("2. j4") && text.contains("all 2 job slots are busy"), "{text}");
        let idle = status_text(&json!({"running": [], "blocked": [], "queued": [],
            "limits": {"max_parallel": 4, "max_per_agent": 2, "in_use": 0, "available": 4}}));
        assert!(idle.contains("0 of 4 slots in use"), "{idle}");
        assert!(idle.contains("2 per agent"), "{idle}");
        assert!(idle.contains("nothing queued"), "{idle}");
    }

    #[test]
    fn diff_text_lists_files_and_totals() {
        let v = json!({
            "job_id": "j1", "available": true, "file_count": 3, "additions": 7, "deletions": 2,
            "clipped": false, "complete": true,
            "files": [
                {"path": "src/a.rs", "status": "M", "additions": 5, "deletions": 2, "binary": false},
                {"path": "notes.txt", "status": "A", "additions": 2, "deletions": 0, "binary": false},
                {"path": "img.png", "status": "A", "additions": null, "deletions": null, "binary": true}
            ]
        });
        let text = diff_text(&v).unwrap();
        assert!(text.contains("M  +5 -2        src/a.rs"), "{text}");
        assert!(text.contains("A  binary       img.png"), "{text}");
        assert!(text.contains("3 files changed, +7 -2"), "{text}");
        let mut running = v.clone();
        running["complete"] = json!(false);
        running["clipped"] = json!(true);
        let text = diff_text(&running).unwrap();
        assert!(text.contains("still running") && text.contains("was cut"), "{text}");
    }

    #[test]
    fn diff_text_says_unavailable() {
        let v = json!({"job_id": "j2", "available": false, "reason": "no_snapshot",
            "detail": "no turn of this job took a snapshot", "files": [], "file_count": 0});
        let err = diff_text(&v).unwrap_err();
        assert!(err.contains("no diff for job j2") && err.contains("no turn of this job took a snapshot"), "{err}");
    }

    #[test]
    fn explain_text_shows_what_the_watchdog_sees() {
        let v = json!({
            "id": "j1", "state": "waiting_approval", "state_seq": 3, "kind": "run",
            "last_event_age_s": 42, "ask_pending": true,
            "asks": [{"tool": "cli:Bash", "age_s": 40, "expires_in_s": 3560, "preview": "npm test"}],
            "deadline_left_s": 3000, "pid": 4242, "slot": true
        });
        let text = explain_text(&v);
        assert!(text.contains("waiting_approval (seq 3)"), "{text}");
        assert!(text.contains("last_event_age: 42 s"));
        assert!(text.contains("ask_pending: true"));
        assert!(text.contains("cli:Bash  asked 40 s ago, expires in 3560 s  npm test"));
        assert!(text.contains("deadline_left: 3000 s"));
        assert!(text.contains("pid: 4242"));
        let failed = json!({"id": "j2", "state": "failed", "state_seq": 5, "kind": "run",
            "failure_reason": "stalled", "ask_pending": false, "asks": [], "slot": false,
            "error": "ERR_CLI_STALLED: the CLI printed nothing for 600 s"});
        let text = explain_text(&failed);
        assert!(text.contains("failure_reason: stalled"));
        assert!(text.contains("holds_slot: false"));
        assert!(!text.contains("pid:"));
    }
}
