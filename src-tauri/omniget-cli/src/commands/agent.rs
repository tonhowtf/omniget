//! `omniget agent run|loop|jobs|agents`: talks to the running desktop app
//! through the local bridge (`127.0.0.1:<port>`, bearer from `settings.json`).
//! The app owns the jobs, so a run started here survives this process.

use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

pub(crate) struct Bridge {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Bridge {
    pub(crate) fn open() -> anyhow::Result<Self> {
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

    pub(crate) async fn post(&self, path: &str, body: Value) -> anyhow::Result<Value> {
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
                if state == "waiting_approval" {
                    eprintln!("  the agent is asking for permission: answer in the app (LLM → Jobs) or on the pet");
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
        } => {
            let job = bridge
                .post("/v1/agent/run", json!({ "agent_id": agent, "prompt": prompt, "workspace": workspace(ws, no_workspace) }))
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
