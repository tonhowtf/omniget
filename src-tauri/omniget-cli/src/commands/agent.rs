//! `omniget agent run|loop|jobs|explain|status|diff|approvals|approve|agents`: talks to the running desktop app
//! through the local bridge (`127.0.0.1:<port>` from `settings.json`) with a
//! scoped token (2.1): `OMNIGET_TOKEN` if set, else the CLI token the app
//! keeps in `<app_data>/llm/cli-token`. The browser extension's key in
//! `settings.json` is the last resort, for one version, with a warning.
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
        let token = resolve_token(
            std::env::var("OMNIGET_TOKEN").ok(),
            std::fs::read_to_string(dir.join("llm").join("cli-token")).ok(),
            bridge["token"].as_str(),
        );
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

    pub(crate) async fn post(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        self.send(self.http.post(format!("{}{path}", self.base)).json(&body))
            .await
    }
}

/// Which bearer the CLI sends: `OMNIGET_TOKEN`, then the app's CLI token
/// file, then (deprecated) the extension key, which the agent routes stop
/// accepting in the next version.
fn resolve_token(env: Option<String>, file: Option<String>, extension: Option<&str>) -> String {
    if let Some(t) = env.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
        return t;
    }
    if let Some(t) = file.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
        return t;
    }
    let legacy = extension.unwrap_or("").trim().to_string();
    if !legacy.is_empty() {
        eprintln!(
            "warning: no OMNIGET_TOKEN and no CLI token file; using the browser extension key, \
             which agent routes stop accepting in the next version. Open OmniGet once to create \
             the CLI token, or set OMNIGET_TOKEN to a token from LLM → Jobs → Access tokens."
        );
    }
    legacy
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
                    eprintln!("  the agent is asking for permission: answer in the app (LLM → Jobs), on the pet,");
                    eprintln!(
                        "  or here: `omniget agent approvals --job {id}`, then `omniget agent approve <approval id>`"
                    );
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
            a["preview"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(60)
                .collect::<String>()
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
            "  blocked  {}  (waiting for you: `omniget agent approvals`)\n",
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
            q["detail"]
                .as_str()
                .unwrap_or(q["reason"].as_str().unwrap_or(""))
        ));
    }
    out
}

/// The decision `omniget agent approve` sends: accept by default, one flag
/// for anything else (clap refuses two of them together).
fn approve_decision(session: bool, always: bool, deny: bool, cancel: bool) -> &'static str {
    match (session, always, deny, cancel) {
        (true, ..) => "accept_session",
        (_, true, ..) => "accept_always",
        (_, _, true, _) => "decline",
        (.., true) => "cancel",
        _ => "accept",
    }
}

/// `omniget agent approvals` in plain lines: one per question, with the id
/// to pass to `approve`.
fn approvals_text(v: &Value) -> String {
    let rows: Vec<&Value> = v.as_array().into_iter().flatten().collect();
    if rows.is_empty() {
        return "nothing is waiting for you\n".to_string();
    }
    let mut out = String::new();
    for a in rows {
        out.push_str(&format!(
            "{}  {:<9} job {:<12} {:<14} {}\n",
            a["id"].as_str().unwrap_or(""),
            a["state"].as_str().unwrap_or(""),
            a["job_id"].as_str().unwrap_or("-"),
            a["tool"].as_str().unwrap_or("?"),
            a["preview"]
                .as_str()
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(60)
                .collect::<String>()
        ));
        if let Some(note) = a["note"].as_str() {
            out.push_str(&format!("    {note}\n"));
        }
    }
    out
}

#[derive(clap::Subcommand)]
pub enum AgentCommand {
    /// Run one task with an agent of the roster (in the current folder), or,
    /// with --tool, an agent of the catalog (or an .md file) as the system
    /// prompt of a coding CLI: `run <agent> --tool claude "<prompt>"`
    Run {
        /// `<prompt>`; with --tool: `[<agent: catalog id | file.md>] <prompt>`
        #[arg(required = true, num_args = 1..=2)]
        args: Vec<String>,
        #[arg(short, long, default_value = "omni")]
        agent: String,
        #[arg(
            long,
            help = "Coding CLI that runs it: claude, codex, gemini, opencode, qwen, cursor … or native"
        )]
        tool: Option<String>,
        #[arg(long, help = "Project folder for --tool (default: current dir)")]
        project: Option<String>,
        #[arg(long, help = "Model for --tool (e.g. haiku, sonnet, gpt-5)")]
        model: Option<String>,
        #[arg(
            long,
            value_parser = ["default", "plan", "accept-edits", "bypass"],
            help = "Permission level of the CLI (bypass prints the flag it maps to)"
        )]
        permission: Option<String>,
        #[arg(long, value_parser = ["docker", "e2b"], help = "Run the CLI in OmniGet's sandbox on a copy of the project")]
        sandbox: Option<String>,
        #[arg(
            long,
            help = "Vault key id (LLM → Keys) for the CLI inside the sandbox"
        )]
        key: Option<String>,
        #[arg(long, help = "Vault key id holding the E2B API key")]
        e2b_key: Option<String>,
        #[arg(long, help = "Append the detected stack of the project to the prompt")]
        context: bool,
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
    /// Keep an agent working until a check passes or the budget runs out;
    /// `--catalog <loop id>` runs a loop of the catalog instead of a prompt
    Loop {
        #[arg(required_unless_present = "catalog", default_value = "")]
        prompt: String,
        #[arg(
            long,
            help = "Catalog loop id (e.g. cct:loops/engineering/build-test-fix-loop)"
        )]
        catalog: Option<String>,
        #[arg(
            long,
            help = "Runner for --catalog: agent:<roster id>, account:<claude|codex>[:<account>], acp:<agent>, tool:<cli>"
        )]
        runner: Option<String>,
        #[arg(
            long,
            help = "With --catalog: install the loop's components into these tools (a,b)"
        )]
        targets: Option<String>,
        #[arg(long, help = "Interval (10m, daily) or cron line between rounds")]
        schedule: Option<String>,
        #[arg(long, help = "Stop when the rounds cost this much (USD)")]
        max_cost: Option<f64>,
        #[arg(long, help = "Model for tool/account runners")]
        model: Option<String>,
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
    /// Permission questions waiting for you (their ids answer them)
    Approvals {
        #[arg(long, help = "Only the questions of this job")]
        job: Option<String>,
        #[arg(
            long,
            default_value = "pending",
            value_parser = ["pending", "stale", "resolved", "cancelled", "expired", "all"],
            help = "Which questions to list"
        )]
        state: String,
    },
    /// Answer a permission question: allow once (default), or --session,
    /// --always, --deny, --cancel (deny and stop the turn)
    Approve {
        id: String,
        #[arg(
            long,
            help = "Allow this tool for the rest of the turn",
            group = "decision"
        )]
        session: bool,
        #[arg(
            long,
            help = "Allow and stop asking for this kind of call",
            group = "decision"
        )]
        always: bool,
        #[arg(long, help = "Refuse this call", group = "decision")]
        deny: bool,
        #[arg(long, help = "Refuse and stop the turn that asked", group = "decision")]
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
            args,
            tool: Some(tool),
            project,
            model,
            permission,
            sandbox,
            key,
            e2b_key,
            context,
            detach,
            ..
        } => {
            let (agent_ref, prompt) = match args.as_slice() {
                [p] => (None, p.clone()),
                [a, p] => (Some(a.clone()), p.clone()),
                _ => bail!("give `[<agent>] <prompt>`"),
            };
            let project = workspace(project, false);
            let mut body = json!({
                "tool": tool,
                "prompt": prompt,
                "project": project,
                "model": model,
                "permission": permission.map(|p| p.replace('-', "_")),
                "context": context,
            });
            if let Some(a) = agent_ref {
                let path = std::path::Path::new(&a);
                if a.ends_with(".md") && path.is_file() {
                    body["agent_markdown"] = json!(std::fs::read_to_string(path)?);
                    body["agent_name"] = json!(path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default());
                } else {
                    body["agent"] = json!(a);
                }
            }
            if let Some(p) = sandbox {
                body["sandbox"] =
                    json!({ "provider": p, "key_id": key, "e2b_key_id": e2b_key, "network": true });
            }
            if body["permission"] == "bypass" {
                let flags = omniget_core::core::agentkit_run::runner::runner_of(&tool)
                    .ok()
                    .and_then(|(_, r)| r.permission_flags.get("bypass").cloned())
                    .unwrap_or_default();
                eprintln!(
                    "warning: bypass runs {tool} with {} (no permission prompts)",
                    if flags.is_empty() {
                        "its most permissive mode".to_string()
                    } else {
                        flags.join(" ")
                    }
                );
            }
            let job = bridge.post("/v1/agent/run-tool", body).await?;
            let id = job["id"].as_str().unwrap_or("").to_string();
            if detach {
                println!("{id}");
                return Ok(());
            }
            follow(&bridge, &id, json_out).await
        }
        AgentCommand::Run {
            args,
            agent,
            workspace: ws,
            no_workspace,
            detach,
            deadline_min,
            ..
        } => {
            let prompt = match args.as_slice() {
                [p] => p.clone(),
                _ => bail!("two arguments (`<agent> <prompt>`) need --tool; without it, give only the prompt and pick the roster agent with --agent"),
            };
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
            catalog: Some(id),
            runner,
            targets,
            schedule,
            max_cost,
            model,
            workspace: ws,
            rounds,
            minutes,
            check,
            agent,
            ..
        } => {
            let spec = runner.unwrap_or_else(|| format!("agent:{agent}"));
            let mut parts = spec.splitn(3, ':');
            let kind = parts.next().unwrap_or("agent").to_string();
            let rid = parts.next().unwrap_or("").to_string();
            let account = parts.next().map(str::to_string);
            let body = json!({
                "id": id,
                "runner": { "kind": kind, "id": rid, "account": account, "model": model,
                            "permission": if kind == "tool" { Some("accept_edits") } else { None } },
                "workspace": workspace(ws, false),
                "targets": targets.map(|t| t.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect::<Vec<_>>()).unwrap_or_default(),
                "schedule": schedule,
                "check_command": check,
                "max_rounds": rounds,
                "max_minutes": minutes,
                "max_cost_usd": max_cost,
            });
            let l = bridge.post("/v1/agent/loop-catalog", body).await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&l)?);
            } else {
                println!(
                    "loop {} ({}) started with {}; check: {}. Follow it with `omniget agent loops` or in LLM → Loops.",
                    l["id"].as_str().unwrap_or(""),
                    l["name"].as_str().unwrap_or(""),
                    l["agent_id"].as_str().unwrap_or(""),
                    l["check_command"].as_str().unwrap_or("none (DONE or budget)")
                );
            }
            Ok(())
        }
        AgentCommand::Loop {
            prompt,
            agent,
            workspace: ws,
            rounds,
            minutes,
            check,
            ..
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
        AgentCommand::Approvals { job, state } => {
            let mut path = format!("/v1/approvals?state={state}");
            if let Some(j) = job {
                path.push_str(&format!("&job_id={j}"));
            }
            let v = bridge.get(&path).await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print!("{}", approvals_text(&v));
            }
            Ok(())
        }
        AgentCommand::Approve {
            id,
            session,
            always,
            deny,
            cancel,
        } => {
            let decision = approve_decision(session, always, deny, cancel);
            let v = bridge
                .post(
                    &format!("/v1/approvals/{id}"),
                    json!({ "decision": decision, "channel": "cli" }),
                )
                .await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!(
                    "{}: {} ({})",
                    v["id"].as_str().unwrap_or(&id),
                    v["state"].as_str().unwrap_or("?"),
                    v["decision"].as_str().unwrap_or(decision)
                );
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
            print!(
                "{}",
                bridge
                    .get_text(&format!("/v1/jobs/{id}/diff?format=patch"))
                    .await?
            );
            Ok(())
        }
        AgentCommand::Diff { id, patch: false } => {
            let v = bridge
                .get(&format!("/v1/jobs/{id}/diff?format=files"))
                .await?;
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
    fn approve_flags_pick_one_decision() {
        assert_eq!(approve_decision(false, false, false, false), "accept");
        assert_eq!(
            approve_decision(true, false, false, false),
            "accept_session"
        );
        assert_eq!(approve_decision(false, true, false, false), "accept_always");
        assert_eq!(approve_decision(false, false, true, false), "decline");
        assert_eq!(approve_decision(false, false, false, true), "cancel");
        // The flags are one clap group: two together never parse.
        use clap::Parser;
        #[derive(Parser)]
        struct T {
            #[command(subcommand)]
            c: AgentCommand,
        }
        assert!(T::try_parse_from(["t", "approve", "apr_1", "--deny", "--always"]).is_err());
        assert!(T::try_parse_from(["t", "approve", "apr_1", "--session"]).is_ok());
    }

    #[test]
    fn approvals_print_the_id_to_answer_with() {
        let v = json!([{"id": "apr_1", "state": "pending", "job_id": "j1", "tool": "shell_exec", "preview": "npm test"},
                       {"id": "apr_2", "state": "stale", "job_id": null, "tool": "cli:Bash", "preview": "ls", "note": "Restart the turn"}]);
        let text = approvals_text(&v);
        assert!(text.contains("apr_1  pending"), "{text}");
        assert!(text.contains("Restart the turn"));
        assert_eq!(approvals_text(&json!([])), "nothing is waiting for you\n");
    }

    #[test]
    fn the_token_comes_from_env_then_file_then_the_extension_key() {
        let f = || Some("ogt_file\n".to_string());
        assert_eq!(
            resolve_token(Some("ogt_env".into()), f(), Some("ext")),
            "ogt_env"
        );
        assert_eq!(
            resolve_token(Some("  ".into()), f(), Some("ext")),
            "ogt_file"
        );
        assert_eq!(resolve_token(None, None, Some("ext")), "ext");
        assert_eq!(resolve_token(None, None, None), "");
    }

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
        assert!(
            text.contains("j2") && text.contains("waiting for you"),
            "{text}"
        );
        assert!(
            text.contains("1. j3") && text.contains("folder busy by j1 (/w/repo)"),
            "{text}"
        );
        assert!(
            text.contains("2. j4") && text.contains("all 2 job slots are busy"),
            "{text}"
        );
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
        assert!(
            text.contains("still running") && text.contains("was cut"),
            "{text}"
        );
    }

    #[test]
    fn diff_text_says_unavailable() {
        let v = json!({"job_id": "j2", "available": false, "reason": "no_snapshot",
            "detail": "no turn of this job took a snapshot", "files": [], "file_count": 0});
        let err = diff_text(&v).unwrap_err();
        assert!(
            err.contains("no diff for job j2")
                && err.contains("no turn of this job took a snapshot"),
            "{err}"
        );
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
