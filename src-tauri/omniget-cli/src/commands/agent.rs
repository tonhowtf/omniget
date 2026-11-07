use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

pub(crate) struct Bridge {
    base: String,
    token: String,
    http: reqwest::Client,
    job: Option<String>,
}

pub(crate) fn job_context() -> Option<String> {
    std::env::var("OMNIGET_JOB_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl Bridge {
    pub(crate) fn open() -> anyhow::Result<Self> {
        let dir =
            omniget_core::core::paths::app_data_dir().ok_or_else(|| anyhow!("no app data dir"))?;
        let raw = std::fs::read_to_string(dir.join("settings.json"))
            .context("settings.json not found: open the OmniGet app once")?;
        let json: Value = serde_json::from_str(&raw)?;
        let bridge = &json["app_settings"]["bridge"];
        let job = job_context();
        let base = match std::env::var("OMNIGET_BRIDGE_URL")
            .ok()
            .filter(|u| u.starts_with("http://127.0.0.1:"))
        {
            Some(url) => url.trim_end_matches('/').to_string(),
            None => {
                let port = bridge["port"]
                    .as_u64()
                    .filter(|p| *p > 0)
                    .ok_or_else(|| anyhow!("the app has no bridge port yet: open OmniGet once"))?;
                format!("http://127.0.0.1:{port}")
            }
        };
        let token = match (&job, std::env::var("OMNIGET_JOB_TOKEN").ok()) {
            (Some(_), Some(t)) if !t.trim().is_empty() => t.trim().to_string(),
            _ => resolve_token(
                std::env::var("OMNIGET_TOKEN").ok(),
                std::fs::read_to_string(dir.join("llm").join("cli-token")).ok(),
                bridge["token"].as_str(),
            ),
        };
        Ok(Self {
            base,
            token,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(150))
                .build()?,
            job,
        })
    }

    pub(crate) async fn ready(&self) -> bool {
        self.authed(self.http.get(format!("{}/v1/state", self.base)))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = req.bearer_auth(&self.token);
        match &self.job {
            Some(j) => req.header("x-omniget-job", j),
            None => req,
        }
    }

    async fn wait(
        &self,
        id: &str,
        until: &str,
        after_seq: i64,
        timeout_ms: u64,
    ) -> anyhow::Result<Option<Value>> {
        let res = self
            .authed(self.http.get(format!(
                "{}/v1/jobs/{id}/wait?until={until}&after_seq={after_seq}&timeout_ms={timeout_ms}",
                self.base
            )))
            .send()
            .await
            .context("the OmniGet app is not running (the bridge did not answer)")?;
        let status = res.status();
        let body: Value = res.json().await.unwrap_or(Value::Null);
        if status.as_u16() == 408 {
            return Ok(None);
        }
        if !status.is_success() {
            bail!(
                "{}: {}",
                status,
                body["error"].as_str().unwrap_or("request failed")
            );
        }
        Ok(Some(body))
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        let res = self
            .authed(req)
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

    pub(crate) async fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.send(self.http.get(format!("{}{path}", self.base)))
            .await
    }

    async fn get_text(&self, path: &str) -> anyhow::Result<String> {
        let res = self
            .authed(self.http.get(format!("{}{path}", self.base)))
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

pub(crate) fn resolve_token(
    env: Option<String>,
    file: Option<String>,
    extension: Option<&str>,
) -> String {
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
             the CLI token, or set OMNIGET_TOKEN to a token from Settings → AI & agents → Connect to other apps."
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

fn auto_resume_flag(on: bool, off: bool) -> Option<bool> {
    match (on, off) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    }
}

const RUN_MODES: [&str; 4] = ["approval_required", "auto_edits", "auto", "full_access"];

fn with_run_mode(mut body: Value, mode: Option<String>) -> Value {
    if let Some(m) = mode.filter(|m| !m.trim().is_empty()) {
        body["run_mode"] = json!(m);
    }
    body
}

fn run_body(
    agent: Option<&str>,
    prompt: &str,
    workspace: Option<String>,
    deadline_min: Option<u32>,
    isolate: bool,
    shared: bool,
    base: Option<String>,
) -> Value {
    let mut body = json!({ "prompt": prompt, "workspace": workspace });
    if let Some(a) = agent.filter(|a| !a.trim().is_empty()) {
        body["agent_id"] = json!(a);
    }
    if let Some(d) = deadline_min {
        body["deadline_min"] = json!(d);
    }
    if isolate {
        body["isolation"] = json!("worktree");
    } else if shared {
        body["isolation"] = json!("shared");
    }
    if let Some(b) = base.filter(|b| !b.trim().is_empty()) {
        body["base"] = json!(b);
    }
    body
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

const TERMINAL: &[&str] = &[
    "done",
    "review",
    "done_unreported",
    "failed",
    "cancelled",
    "interrupted",
];

async fn follow(
    bridge: &Bridge,
    id: &str,
    json_out: bool,
    start_seq: Option<i64>,
) -> anyhow::Result<()> {
    let mut job = bridge.get(&format!("/v1/jobs/{id}")).await?;
    let mut last_state = String::new();
    let mut seen_log = 0usize;
    loop {
        let state = job["state"].as_str().unwrap_or("").to_string();
        let seq = job["state_seq"].as_i64().unwrap_or(0);
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
                    eprintln!(
                        "  the agent is asking for permission: answer in the app, on the pet,"
                    );
                    eprintln!(
                        "  or here: `omniget agent approvals --job {id}`, then `omniget agent approve <approval id>`"
                    );
                }
                last_state = state.clone();
            }
        }
        if is_end(&state, seq, start_seq) {
            return finish(id, &job, &state, json_out);
        }
        job = match bridge.wait(id, "", seq, 5_000).await? {
            Some(next) => next,
            None => bridge.get(&format!("/v1/jobs/{id}")).await?,
        };
    }
}

fn is_end(state: &str, seq: i64, start_seq: Option<i64>) -> bool {
    TERMINAL.contains(&state) && start_seq.is_none_or(|s| seq > s)
}

fn finish(id: &str, job: &Value, state: &str, json_out: bool) -> anyhow::Result<()> {
    if json_out {
        println!("{}", serde_json::to_string_pretty(job)?);
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
        if state == "review" {
            if let Some(stamp) = job["proof"]["stamp"].as_str() {
                eprintln!("proof: {stamp}");
            }
            eprintln!("  it changed files: `omniget agent diff {id}` shows them");
        }
        if state == "done_unreported" {
            eprintln!(
                "  the agent ended without its OUTCOME line: read the result before relying on it"
            );
        }
    }
    let said_failed = job["outcome"]["status"].as_str() == Some("failed");
    if !matches!(state, "done" | "review" | "done_unreported") || said_failed {
        std::process::exit(1);
    }
    Ok(())
}

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
    if let Some(session) = v["session_id"].as_str() {
        out.push_str(&format!("  session: {session}\n"));
    }
    if let Some(n) = v["attempt"].as_u64().filter(|n| *n > 0) {
        out.push_str(&format!("  resumed by itself: {n} time(s)\n"));
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

fn approve_decision(session: bool, always: bool, deny: bool, cancel: bool) -> &'static str {
    match (session, always, deny, cancel) {
        (true, ..) => "accept_session",
        (_, true, ..) => "accept_always",
        (_, _, true, _) => "decline",
        (.., true) => "cancel",
        _ => "accept",
    }
}

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

fn integrate_text(o: &Value) -> String {
    let target = o["target_branch"].as_str().unwrap_or("?");
    match o["status"].as_str().unwrap_or("") {
        "up_to_date" => format!("nothing to integrate: {target} already has it\n"),
        "merged" => format!(
            "integrated into {target} as {} ({})\n",
            o["commit"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(7)
                .collect::<String>(),
            if o["strategy"] == "squash" {
                "squash"
            } else {
                "merge --no-ff"
            }
        ),
        other => format!(
            "integration {other}: {}\nyour checkout was {}\n",
            o["conflicts"]
                .as_array()
                .map(|c| c
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "))
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| o["output"].as_str().unwrap_or("").to_string()),
            if o["clean"].as_bool() == Some(true) {
                "left as it was"
            } else {
                "NOT left clean: check git status"
            }
        ),
    }
}

fn discard_text(v: &Value) -> String {
    let branch = v["branch"].as_str().unwrap_or("");
    let mut out = String::new();
    if let Some(saved) = v["removed"]["saved"].as_str() {
        out.push_str(&format!(
            "uncommitted work saved on {branch} as {} before removing the worktree\n",
            saved.chars().take(7).collect::<String>()
        ));
    }
    match v["tip"].as_str() {
        Some(tip) => out.push_str(&format!(
            "discarded: worktree removed, branch {branch} deleted (it was {}; `git branch {branch} {tip}` brings it back until git collects it)\n",
            tip.chars().take(7).collect::<String>()
        )),
        None => out.push_str("discarded: worktree removed\n"),
    }
    out
}

#[derive(clap::Subcommand)]
pub enum AgentCommand {
    #[command(
        about = "Run one task with an agent of the roster (in the current folder), or, with --tool, an agent of the catalog (or an .md file) as the system prompt of a coding CLI: `run <agent> --tool claude \"<prompt>\"`"
    )]
    Run {
        #[arg(help = "`<prompt>`; with --tool: `[<agent: catalog id | file.md>] <prompt>`")]
        #[arg(required = true, num_args = 1..=2)]
        args: Vec<String>,
        #[arg(
            short,
            long,
            help = "Roster agent (default: the `agent` of the folder's .omniget/workflow.md, else omni)"
        )]
        agent: Option<String>,
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
        #[arg(long, help = "Vault key id for the CLI inside the sandbox")]
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
            conflicts_with = "detach",
            help = "Wait for the end (the default): only an end newer than the start counts; exit 1 if it failed"
        )]
        wait: bool,
        #[arg(
            long,
            value_name = "ID",
            help = "Idempotency key: the same id again returns the same job instead of starting another"
        )]
        command_id: Option<String>,
        #[arg(long, help = "A short name for the job in lists")]
        title: Option<String>,
        #[arg(
            long,
            value_name = "MIN",
            help = "Stop the job after this many minutes (default: the app setting, 60)"
        )]
        deadline_min: Option<u32>,
        #[arg(
            long,
            conflicts_with = "shared",
            help = "Run in a git worktree of its own (default: only when another job is writing in the folder)"
        )]
        isolate: bool,
        #[arg(
            long,
            help = "Run in the folder itself, waiting for it if another job is writing there"
        )]
        shared: bool,
        #[arg(
            long,
            value_name = "REF",
            help = "Start the worktree from this branch or commit (default: your checkout as it is, uncommitted work included)"
        )]
        base: Option<String>,
        #[arg(
            long,
            conflicts_with = "no_auto_resume",
            help = "Start again by itself (--resume, check before repeating) if the app closes or the agent goes silent; only when it ran in its own worktree or wrote nothing"
        )]
        auto_resume: bool,
        #[arg(long, help = "Never start again by itself, whatever the app's setting")]
        no_auto_resume: bool,
        #[arg(
            long,
            value_name = "MODE",
            value_parser = RUN_MODES,
            help = "What the agent may do without asking: approval_required (default: every edit and command asks), auto_edits, auto, full_access"
        )]
        mode: Option<String>,
        #[arg(
            long,
            help = "Ignore the repository's .omniget/workflow.md (its defaults and prompt template)"
        )]
        no_workflow: bool,
    },
    #[command(
        about = "Keep an agent working until a check passes or the budget runs out; `--catalog <loop id>` runs a loop of the catalog instead of a prompt"
    )]
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
            help = "Shell command; the loop ends when it exits 0 (e.g. \"npm test\"). Without it, an independent auditor on the agent's small model decides when the work is done"
        )]
        check: Option<String>,
        #[arg(
            long,
            conflicts_with = "no_auto_resume",
            help = "Start again by itself (--resume, check before repeating) if the app closes or the agent goes silent; only when it ran in its own worktree or wrote nothing"
        )]
        auto_resume: bool,
        #[arg(long, help = "Never start again by itself, whatever the app's setting")]
        no_auto_resume: bool,
        #[arg(
            long,
            help = "Stop when the rounds and their audits spend this many billable tokens"
        )]
        token_budget: Option<u64>,
        #[arg(
            long,
            value_name = "MODE",
            value_parser = RUN_MODES,
            help = "What the agent may do without asking: approval_required (default: every edit and command asks), auto_edits, auto, full_access"
        )]
        mode: Option<String>,
    },
    #[command(about = "List recent jobs, show one, or cancel one")]
    Jobs {
        id: Option<String>,
        #[arg(long)]
        cancel: bool,
    },
    #[command(
        about = "Why a job is in its state: last event age, pending question, deadline left"
    )]
    Explain { id: String },
    #[command(about = "What runs now, what waits (place in the queue and why) and the limits")]
    Status,
    #[command(about = "What a job changed in its folder: files and +/-, or the patch")]
    Diff {
        id: String,
        #[arg(long, help = "Print the unified diff instead of the file list")]
        patch: bool,
    },
    #[command(about = "Permission questions waiting for you (their ids answer them)")]
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
    #[command(
        about = "Answer a permission question: allow once (default), or --session, --always, --deny, --cancel (deny and stop the turn)"
    )]
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
    #[command(
        about = "Merge a worktree job's branch into your checkout (`merge --no-ff`, or `--squash`); a conflict is undone and your checkout left as it was"
    )]
    Integrate {
        id: String,
        #[arg(long, help = "One squashed commit instead of a merge commit")]
        squash: bool,
    },
    #[command(about = "Throw a worktree job's work away: its worktree and its branch")]
    Discard { id: String },
    #[command(about = "Budget ceilings (3.3): list them, set one, raise a paused one")]
    Budget {
        #[command(subcommand)]
        cmd: BudgetCommand,
    },
    #[command(
        about = "Everything to report a job back in one answer: verdict, the agent's OUTCOME, the host's proof (checks, base→head), the files it changed and what the repository's workflow did (3.10)"
    )]
    Report { id: String },
    #[command(
        about = "The repository's `.omniget/workflow.md` (3.10): what is in effect, trust its hooks and checks, or validate the file"
    )]
    Workflow {
        #[command(subcommand)]
        cmd: WorkflowCommand,
    },
    #[command(about = "List the running and finished Loops")]
    Loops,
    #[command(about = "List the agents of the roster")]
    Agents,
}

#[derive(clap::Subcommand)]
pub enum WorkflowCommand {
    #[command(
        about = "The version in effect (or the error and the last good version), its defaults, the commands it would run and whether they are trusted"
    )]
    Show {
        #[arg(short, long, help = "Folder in the repository (default: current dir)")]
        workspace: Option<String>,
    },
    #[command(
        about = "Approve the version in effect: its hooks and checks run from now on, until the file changes. Asks on a terminal; elsewhere give --sha256 (from `show`), so what is trusted is what was read"
    )]
    Trust {
        #[arg(short, long, help = "Folder in the repository (default: current dir)")]
        workspace: Option<String>,
        #[arg(
            long,
            value_name = "HASH",
            help = "The SHA-256 (or its first 12+ characters) `show` printed"
        )]
        sha256: Option<String>,
        #[arg(long, help = "Stop trusting this repository's workflow")]
        revoke: bool,
    },
    #[command(
        about = "Validate the file without the app: front matter, template variables (exit 1 when invalid)"
    )]
    Check {
        #[arg(help = "The file (default: .omniget/workflow.md of the current repository)")]
        file: Option<String>,
    },
}

fn query_value(v: &str) -> String {
    v.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn workflow_text(v: &Value) -> String {
    let mut out = String::new();
    let state = v["state"].as_str().unwrap_or("?");
    out.push_str(&format!("{}\n", v["path"].as_str().unwrap_or("")));
    if state == "none" {
        out.push_str("no workflow: jobs here use the flags and the app's defaults\n");
        return out;
    }
    if let Some(e) = v["error"].as_str() {
        out.push_str(&format!("invalid: {e}\n"));
        if v["from_last_good"] == json!(true) {
            out.push_str("the last good version stays in effect until the file is fixed\n");
        } else {
            out.push_str("no earlier valid version: jobs in this repository are refused until it is fixed (or --no-workflow)\n");
        }
    }
    let Some(a) = v.get("active").filter(|a| !a.is_null()) else {
        return out;
    };
    let sha = a["sha256"].as_str().unwrap_or("");
    out.push_str(&format!(
        "in effect: sha256 {sha} ({})\n",
        if v["trusted"] == json!(true) {
            "trusted"
        } else {
            "not trusted"
        }
    ));
    if let Some(c) = a["config"].as_object() {
        for (k, val) in c {
            if k == "hooks" || k == "checks" || val.is_null() {
                continue;
            }
            out.push_str(&format!("  {k}: {val}\n"));
        }
    }
    let cmds = v["commands"].as_array().cloned().unwrap_or_default();
    if !cmds.is_empty() {
        out.push_str("commands it runs (only while trusted):\n");
        for c in &cmds {
            out.push_str(&format!(
                "  {:<14} {}\n",
                c["kind"].as_str().unwrap_or(""),
                c["command"].as_str().unwrap_or("")
            ));
        }
    }
    if let Some(r) = v["skipped_reason"].as_str() {
        out.push_str(&format!("held until trusted ({}): {r}\n", v["gated"]));
    }
    out
}

fn report_text(v: &Value) -> String {
    let mut out = format!(
        "{} {} ({})\n",
        v["jobId"].as_str().unwrap_or(""),
        v["verdict"].as_str().unwrap_or("?"),
        v["state"].as_str().unwrap_or("?")
    );
    if let Some(o) = v["outcome"].as_object() {
        out.push_str(&format!(
            "OUTCOME: {} — {}\n",
            o.get("status").and_then(Value::as_str).unwrap_or("?"),
            o.get("summary").and_then(Value::as_str).unwrap_or("")
        ));
    }
    if let Some(e) = v["jobError"].as_str() {
        out.push_str(&format!("error: {e}\n"));
    }
    let d = &v["diff"];
    if let Some(n) = d["filesChanged"].as_u64() {
        out.push_str(&format!(
            "{n} file(s) changed, +{} -{}\n",
            d["additions"].as_u64().unwrap_or(0),
            d["deletions"].as_u64().unwrap_or(0)
        ));
        for f in d["files"].as_array().into_iter().flatten().take(20) {
            out.push_str(&format!(
                "  {} {}\n",
                f["status"].as_str().unwrap_or(""),
                f["path"].as_str().unwrap_or("")
            ));
        }
    }
    for c in v["proof"]["checks"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "check {} `{}`: exit {}\n",
            if c["ok"] == json!(true) { "✓" } else { "✗" },
            c["command"].as_str().unwrap_or(""),
            c["exit_code"]
        ));
    }
    if let Some(w) = v["proof"]["checksSkipped"].as_str() {
        out.push_str(&format!("checks skipped: {w}\n"));
    }
    if let Some(w) = v["workflow"].as_object() {
        out.push_str(&format!(
            "workflow sha256 {} ({}), filled: {}\n",
            w.get("sha256")
                .and_then(Value::as_str)
                .map(|s| &s[..12.min(s.len())])
                .unwrap_or("?"),
            if w.get("trusted") == Some(&json!(true)) {
                "trusted"
            } else {
                "not trusted"
            },
            w.get("filled").cloned().unwrap_or(json!([]))
        ));
    }
    out.push_str(&format!("next: {}\n", v["next"].as_str().unwrap_or("")));
    out
}

#[derive(clap::Subcommand)]
pub enum BudgetCommand {
    #[command(about = "Every ceiling with what it has used, and the open incidents")]
    List,
    #[command(
        about = "Set a ceiling: `omniget agent budget set loop:<id> 0.05` (US$ for the loop's whole run), `--metric quota_pct --window 5h`, `--metric weighted_tokens --window day`; the same scope, metric and window again replaces it"
    )]
    Set {
        #[arg(help = "global, agent:<id>, job:<id>, loop:<id>, mission:<id>, account:<id>")]
        scope: String,
        #[arg(help = "US$, weighted tokens, or percentage points of the window")]
        amount: f64,
        #[arg(long, default_value = "usd", value_parser = ["usd", "weighted_tokens", "quota_pct"])]
        metric: String,
        #[arg(long, default_value = "day", value_parser = ["turn", "day", "week", "5h", "7d", "run"])]
        window: String,
        #[arg(
            long,
            value_name = "PCT",
            help = "Warn when this share of the amount is used"
        )]
        warn: Option<f64>,
        #[arg(
            long,
            help = "Ask \"continue?\" at the ceiling instead of pausing the scope"
        )]
        soft: bool,
    },
    #[command(about = "Raise a paused scope's ceiling and resume its work (`bi_…` from `list`)")]
    Raise {
        incident: String,
        #[arg(help = "The new ceiling, above what is already used")]
        amount: f64,
    },
}

fn budget_text(v: &Value) -> String {
    let mut out = String::new();
    let policies = v["policies"].as_array().cloned().unwrap_or_default();
    if policies.is_empty() {
        out.push_str("no budget ceilings: nothing is limited\n");
    }
    for p in &policies {
        let observed = match p["observed"].as_f64() {
            Some(o) => format!("{o:.4}"),
            None => "no reading".into(),
        };
        out.push_str(&format!(
            "{}  {:<22} {} {} {} of {}{}{}\n",
            p["id"].as_str().unwrap_or("?"),
            p["scope"].as_str().unwrap_or("?"),
            p["metric"].as_str().unwrap_or("?"),
            p["window"].as_str().unwrap_or("?"),
            observed,
            p["amount"],
            if p["hard_stop"].as_bool() == Some(false) {
                "  (asks)"
            } else {
                ""
            },
            if p["incident"].is_object() {
                "  PAUSED"
            } else {
                ""
            },
        ));
    }
    for i in v["incidents"].as_array().into_iter().flatten() {
        if !matches!(i["state"].as_str(), Some("open" | "paused")) {
            continue;
        }
        out.push_str(&format!(
            "incident {}  {}  {} {:.4} of {}  → omniget agent budget raise {} <amount>\n",
            i["id"].as_str().unwrap_or("?"),
            i["state"].as_str().unwrap_or("?"),
            i["scope"].as_str().unwrap_or("?"),
            i["observed"].as_f64().unwrap_or(0.0),
            i["limit"],
            i["id"].as_str().unwrap_or("?"),
        ));
    }
    out
}

fn fresh_command_id() -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "cli-{nanos:x}-{:x}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn workflow_check(file: Option<&str>, json_out: bool) -> anyhow::Result<()> {
    use omniget_core::core::llm::workflow;
    let path = match file {
        Some(f) => std::path::PathBuf::from(f),
        None => workflow::root_of(&std::env::current_dir()?).join(workflow::FILE),
    };
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    match workflow::parse(&text) {
        Ok(w) => {
            if json_out {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"valid": true, "workflow": w, "commands": w.commands(), "gated": w.gated()})
                    )?
                );
            } else {
                println!(
                    "{}: valid (sha256 {})",
                    path.display(),
                    workflow::short(&w.sha256)
                );
                for c in w.commands() {
                    println!("  {:<14} {}", c.kind, c.command);
                }
                if !w.gated().is_empty() {
                    println!("held until trusted: {}", w.gated().join(", "));
                }
            }
            Ok(())
        }
        Err(e) => {
            if json_out {
                println!("{}", json!({"valid": false, "error": e}));
            }
            bail!("{}: {e}", path.display())
        }
    }
}

const INSIDE_A_JOB: &str = "OMNIGET_JOB_ID is set: this runs inside an OmniGet job, and a job never starts another job (delegation depth 1). Do the task yourself, in this turn.";

pub async fn execute(cmd: AgentCommand, json_out: bool) -> anyhow::Result<()> {
    if matches!(cmd, AgentCommand::Run { .. } | AgentCommand::Loop { .. })
        && job_context().is_some()
    {
        bail!(INSIDE_A_JOB);
    }
    if matches!(
        cmd,
        AgentCommand::Workflow {
            cmd: WorkflowCommand::Trust { .. }
        }
    ) && job_context().is_some()
    {
        bail!("OMNIGET_JOB_ID is set: a job never trusts a repository's workflow; the person does, in OmniGet or with this command outside a job.");
    }
    if let AgentCommand::Workflow {
        cmd: WorkflowCommand::Check { file },
    } = &cmd
    {
        return workflow_check(file.as_deref(), json_out);
    }
    let bridge = super::launch::connect().await?;
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
            follow(&bridge, &id, json_out, job["state_seq"].as_i64()).await
        }
        AgentCommand::Run {
            args,
            agent,
            workspace: ws,
            no_workspace,
            detach,
            deadline_min,
            isolate,
            shared,
            base,
            mode,
            command_id,
            title,
            auto_resume,
            no_auto_resume,
            no_workflow,
            ..
        } => {
            let prompt = match args.as_slice() {
                [p] => p.clone(),
                _ => bail!("two arguments (`<agent> <prompt>`) need --tool; without it, give only the prompt and pick the roster agent with --agent"),
            };
            let mut body = with_run_mode(
                run_body(
                    agent.as_deref(),
                    &prompt,
                    workspace(ws, no_workspace),
                    deadline_min,
                    isolate,
                    shared,
                    base,
                ),
                mode,
            );
            body["type"] = json!("job.start");
            body["command_id"] = json!(command_id.unwrap_or_else(fresh_command_id));
            body["channel"] = json!("cli");
            if let Some(t) = title {
                body["title"] = json!(t);
            }
            if let Some(on) = auto_resume_flag(auto_resume, no_auto_resume) {
                body["auto_resume"] = json!(on);
            }
            if no_workflow {
                body["workflow"] = json!(false);
            }
            let receipt = bridge.post("/v1/commands", body).await?;
            let id = receipt["job_id"].as_str().unwrap_or("").to_string();
            if detach {
                println!("{id}");
                return Ok(());
            }
            follow(&bridge, &id, json_out, receipt["job"]["state_seq"].as_i64()).await
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
                    "loop {} ({}) started with {}; check: {}. Follow it with `omniget agent loops`.",
                    l["id"].as_str().unwrap_or(""),
                    l["name"].as_str().unwrap_or(""),
                    l["agent_id"].as_str().unwrap_or(""),
                    l["check_command"].as_str().unwrap_or("none (the auditor or the budget)")
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
            schedule,
            auto_resume,
            no_auto_resume,
            token_budget,
            mode,
            ..
        } => {
            let l = bridge
                .post(
                    "/v1/agent/loop",
                    with_run_mode(
                        json!({ "agent_id": agent, "prompt": prompt, "workspace": workspace(ws, false), "max_rounds": rounds, "max_minutes": minutes, "check_command": check, "schedule": schedule, "auto_resume": auto_resume_flag(auto_resume, no_auto_resume), "token_budget": token_budget }),
                        mode,
                    ),
                )
                .await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&l)?);
            } else {
                println!("loop {} started; it keeps running inside the app. Follow it with `omniget agent loops`.", l["id"].as_str().unwrap_or(""));
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
        AgentCommand::Jobs { id: Some(id), .. } => follow(&bridge, &id, json_out, None).await,
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
        AgentCommand::Integrate { id, squash } => {
            let v = bridge
                .post(
                    &format!("/v1/jobs/{id}/integrate"),
                    json!({ "strategy": if squash { "squash" } else { "merge" } }),
                )
                .await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print!("{}", integrate_text(&v["outcome"]));
            }
            Ok(())
        }
        AgentCommand::Discard { id } => {
            let v = bridge
                .post(&format!("/v1/jobs/{id}/discard"), json!({}))
                .await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print!("{}", discard_text(&v));
            }
            Ok(())
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
        AgentCommand::Report { id } => {
            let v = bridge.get(&format!("/v1/jobs/{id}/report")).await?;
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print!("{}", report_text(&v));
            }
            Ok(())
        }
        AgentCommand::Workflow { cmd } => {
            let dir = |w: Option<String>| workspace(w, false).unwrap_or_default();
            let status = |ws: &str| format!("/v1/workflow?workspace={}", query_value(ws));
            let v = match cmd {
                WorkflowCommand::Show { workspace: w } => bridge.get(&status(&dir(w))).await?,
                WorkflowCommand::Trust {
                    workspace: w,
                    sha256,
                    revoke,
                } => {
                    let ws = dir(w);
                    let sha = if revoke {
                        String::new()
                    } else {
                        match sha256 {
                            Some(s) => s,
                            None => {
                                use std::io::IsTerminal;
                                let now = bridge.get(&status(&ws)).await?;
                                eprint!("{}", workflow_text(&now));
                                let Some(sha) = now["active"]["sha256"].as_str().map(String::from)
                                else {
                                    bail!("no valid workflow to trust here");
                                };
                                if !std::io::stdin().is_terminal() {
                                    bail!("stdin is not a terminal: read `omniget agent workflow show`, then pass --sha256 {}", &sha[..12]);
                                }
                                eprint!("Trust these commands of this repository? [y/N] ");
                                let mut line = String::new();
                                std::io::stdin().read_line(&mut line)?;
                                if !matches!(line.trim(), "y" | "Y" | "yes") {
                                    bail!("not trusted");
                                }
                                sha
                            }
                        }
                    };
                    bridge
                        .post(
                            "/v1/workflow/trust",
                            json!({ "workspace": ws, "sha256": sha, "revoke": revoke }),
                        )
                        .await?
                }
                WorkflowCommand::Check { .. } => unreachable!("handled before the bridge"),
            };
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print!("{}", workflow_text(&v));
            }
            Ok(())
        }
        AgentCommand::Budget { cmd } => {
            let v = match cmd {
                BudgetCommand::List => bridge.get("/v1/budget").await?,
                BudgetCommand::Set {
                    scope,
                    amount,
                    metric,
                    window,
                    warn,
                    soft,
                } => {
                    bridge
                        .post(
                            "/v1/budget/policies",
                            json!({ "scope": scope, "amount": amount, "metric": metric,
                                    "window": window, "warn_pct": warn, "hard_stop": !soft }),
                        )
                        .await?
                }
                BudgetCommand::Raise { incident, amount } => {
                    bridge
                        .post(
                            &format!("/v1/budget/incidents/{incident}"),
                            json!({ "action": "raise", "amount": amount }),
                        )
                        .await?
                }
            };
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else if v.get("policies").is_some() {
                print!("{}", budget_text(&v));
            } else if let Some(inc) = v.get("incident") {
                let loops = v["resumed_loops"].as_array().map(|a| a.len()).unwrap_or(0);
                println!(
                    "{}: {} ({}); {loops} loop(s) resumed",
                    inc["id"].as_str().unwrap_or("?"),
                    inc["state"].as_str().unwrap_or("?"),
                    inc["resolution"].as_str().unwrap_or("")
                );
            } else {
                println!(
                    "{}  {} {} {} = {}",
                    v["id"].as_str().unwrap_or("?"),
                    v["scope"].as_str().unwrap_or("?"),
                    v["metric"].as_str().unwrap_or("?"),
                    v["window"].as_str().unwrap_or("?"),
                    v["amount"]
                );
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
