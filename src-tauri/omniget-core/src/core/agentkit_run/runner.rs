//! Headless run of a coding CLI (`claude -p`, `codex exec`, `gemini -p`,
//! `opencode run`, `qwen -p`, `agent -p` …) with an agent as the system prompt.
//! The argv comes from the `[runner]` table of each tool manifest
//! (`agentkit/targets/data/<id>.toml`), never from code per tool; the output
//! parser reads the JSON line dialects the manifests ask for (Claude/Qwen/Cursor
//! stream-json, Codex `--json`, Gemini stream-json, OpenCode `--format json`)
//! and falls back to plain text.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio_util::sync::CancellationToken;

use crate::core::agentkit::targets::{self, TargetAdapter};
use crate::core::llm::drivers::acp::proc;

pub const ERR_RUN: &str = "ERR_AGENTKIT_RUN";

/// Permission level of a headless run. `Default` adds no flag (the CLI's own
/// defaults); `Bypass` is only ever set by an explicit user choice and the UI
/// shows the flag it maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    #[default]
    Default,
    Plan,
    AcceptEdits,
    Bypass,
}

impl Permission {
    pub fn parse(s: &str) -> Permission {
        match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "plan" | "read_only" | "readonly" => Permission::Plan,
            "accept_edits" | "acceptedits" | "edits" => Permission::AcceptEdits,
            "bypass" | "yolo" | "full" | "full_access" => Permission::Bypass,
            _ => Permission::Default,
        }
    }

    pub fn key(self) -> Option<&'static str> {
        match self {
            Permission::Default => None,
            Permission::Plan => Some("plan"),
            Permission::AcceptEdits => Some("accept_edits"),
            Permission::Bypass => Some("bypass"),
        }
    }
}

/// One headless run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolRun {
    /// Tool id of the briefing table (`claude`, `codex` …).
    pub tool: String,
    pub prompt: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission: Permission,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

/// The launch of a [`ToolRun`]: program, argv, and what could not be mapped.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    /// Flags the permission level added (shown to the user as they are).
    pub permission_flags: Vec<String>,
    /// `true` when the tool has no system-prompt flag and the agent went in
    /// front of the prompt instead.
    pub system_in_prompt: bool,
    pub notes: Vec<String>,
}

/// Tools whose JSON output this module parses; others run in plain text.
const JSON_TOOLS: &[&str] = &["claude", "codex", "gemini", "qwen", "cursor", "opencode"];

/// The runner config of `tool`, or an error naming the tools that have one.
pub fn runner_of(
    tool: &str,
) -> Result<(&'static TargetAdapter, &'static targets::RunnerConfig), String> {
    let t = targets::target(tool).ok_or_else(|| format!("{ERR_RUN}: unknown tool `{tool}`"))?;
    let r = t.runner.as_ref().ok_or_else(|| {
        let with: Vec<&str> = targets::all_targets()
            .iter()
            .filter(|t| t.runner.is_some())
            .map(|t| t.id.as_str())
            .collect();
        format!(
            "{ERR_RUN}: {} has no headless runner (tools with one: {})",
            t.name,
            with.join(", ")
        )
    })?;
    Ok((t, r))
}

/// Tools that can run headless, with the binary found on this machine.
pub fn available_tools() -> Vec<Value> {
    targets::all_targets()
        .iter()
        .filter_map(|t| {
            let r = t.runner.as_ref()?;
            let bin = proc::which(&r.cmd);
            Some(serde_json::json!({
                "id": t.id,
                "name": t.name,
                "cmd": r.cmd,
                "installed": bin.is_some(),
                "path": bin.map(|p| p.to_string_lossy().to_string()),
                "system_prompt_flag": r.system_prompt_flag,
                "model_flag": r.model_flag,
                "permission_flags": r.permission_flags,
            }))
        })
        .collect()
}

/// The prompt with the agent in front, for runners without a system flag and
/// for roster agents (the job prompt carries the role).
pub fn prompt_with_role(system: Option<&str>, prompt: &str) -> String {
    match system.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => format!(
            "<agent_instructions>\n{s}\n</agent_instructions>\n\nFollow the instructions above as your role for this task.\n\nTask:\n{prompt}"
        ),
        None => prompt.to_string(),
    }
}

/// Pure argv builder. Subcommand words of `headless` (`exec`, `run`) go first,
/// flags next, the headless flags (`-p`) right before the prompt, which is the
/// last argument (Gemini/Qwen read `-p <prompt>`).
pub fn launch(target: &TargetAdapter, r: &targets::RunnerConfig, run: &ToolRun) -> Launch {
    let mut args: Vec<String> = Vec::new();
    let mut notes = Vec::new();
    let (sub, head_flags): (Vec<&String>, Vec<&String>) =
        r.headless.iter().partition(|a| !a.starts_with('-'));
    args.extend(sub.into_iter().cloned());
    if JSON_TOOLS.contains(&target.id.as_str()) {
        args.extend(r.json_flag.iter().cloned());
    }
    let mut permission_flags = Vec::new();
    if let Some(key) = run.permission.key() {
        match r.permission_flags.get(key) {
            Some(f) => {
                permission_flags = f.clone();
                args.extend(f.iter().cloned());
            }
            None => notes.push(format!(
                "{} has no flag for the `{key}` permission level; its defaults apply",
                target.name
            )),
        }
    }
    if let Some(m) = run
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        match &r.model_flag {
            Some(flag) => {
                args.push(flag.clone());
                args.push(m.to_string());
            }
            None => notes.push(format!(
                "{} takes no model flag; `{m}` ignored",
                target.name
            )),
        }
    }
    let system = run
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut system_in_prompt = false;
    let prompt = match (system, &r.system_prompt_flag) {
        (Some(s), Some(flag)) => {
            args.push(flag.clone());
            args.push(s.to_string());
            run.prompt.clone()
        }
        (Some(s), None) => {
            system_in_prompt = true;
            notes.push(format!(
                "{} has no system-prompt flag: the agent goes in front of the prompt",
                target.name
            ));
            prompt_with_role(Some(s), &run.prompt)
        }
        _ => run.prompt.clone(),
    };
    args.extend(head_flags.into_iter().cloned());
    args.push(prompt);
    Launch {
        program: r.cmd.clone(),
        args,
        permission_flags,
        system_in_prompt,
        notes,
    }
}

/// Summed usage of a run, same keys as a job's `usage`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunUsage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: Option<f64>,
    pub calls: u64,
}

/// What one output line meant.
#[derive(Debug, Clone, PartialEq)]
pub enum LineEvent {
    /// Assistant text (a delta or a whole message).
    Text(String),
    /// A tool started: its name and a short argument.
    Tool(String),
    /// The final answer the CLI reported (replaces the collected text).
    Final(String),
    Usage(RunUsage),
    Session(String),
    Error(String),
    Nothing,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn u(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}

fn short_arg(input: &Value) -> String {
    for k in [
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
    ] {
        if let Some(v) = input.get(k).and_then(|v| v.as_str()) {
            let one: String = v.lines().next().unwrap_or("").chars().take(80).collect();
            return one;
        }
    }
    String::new()
}

/// Reads one stdout line of any supported dialect. Non-JSON lines are text.
pub fn parse_line(line: &str) -> Vec<LineEvent> {
    let t = line.trim();
    if t.is_empty() {
        return vec![LineEvent::Nothing];
    }
    let Ok(v) = serde_json::from_str::<Value>(t) else {
        return vec![LineEvent::Text(format!("{line}\n"))];
    };
    if !v.is_object() {
        return vec![LineEvent::Text(format!("{line}\n"))];
    }
    let ty = s(&v["type"]);
    let mut out = Vec::new();
    match ty {
        // Claude / Qwen / Cursor stream-json
        "system" if !s(&v["session_id"]).is_empty() => {
            out.push(LineEvent::Session(s(&v["session_id"]).to_string()))
        }
        "assistant" => {
            for block in v["message"]["content"].as_array().into_iter().flatten() {
                match s(&block["type"]) {
                    "text" => out.push(LineEvent::Text(s(&block["text"]).to_string())),
                    "tool_use" => {
                        let arg = short_arg(&block["input"]);
                        out.push(LineEvent::Tool(
                            format!("{} {arg}", s(&block["name"])).trim().to_string(),
                        ))
                    }
                    _ => {}
                }
            }
            if let Some(text) = v["message"]["content"].as_str() {
                out.push(LineEvent::Text(text.to_string()));
            }
        }
        "result" => {
            let usage = &v["usage"];
            let stats = &v["stats"];
            let mut ru = RunUsage {
                model: String::new(),
                input_tokens: u(&usage["input_tokens"]).max(u(&stats["input_tokens"])),
                output_tokens: u(&usage["output_tokens"]).max(u(&stats["output_tokens"])),
                cache_read_tokens: u(&usage["cache_read_input_tokens"]),
                cost_usd: v["total_cost_usd"].as_f64(),
                calls: 1,
            };
            if let Some(models) = v["modelUsage"].as_object() {
                if let Some(m) = models.keys().next() {
                    ru.model = m.clone();
                }
            }
            out.push(LineEvent::Usage(ru));
            if let Some(r) = v["result"].as_str() {
                out.push(LineEvent::Final(r.to_string()));
            }
            if v["is_error"].as_bool() == Some(true) || s(&v["status"]) == "error" {
                let msg = v["result"]
                    .as_str()
                    .or(v["error"]["message"].as_str())
                    .or(v["error"].as_str())
                    .unwrap_or("the run failed");
                out.push(LineEvent::Error(msg.to_string()));
            }
            if !s(&v["session_id"]).is_empty() {
                out.push(LineEvent::Session(s(&v["session_id"]).to_string()));
            }
        }
        // Gemini stream-json
        "init" if !s(&v["session_id"]).is_empty() => {
            out.push(LineEvent::Session(s(&v["session_id"]).to_string()))
        }
        "message" if s(&v["role"]) == "assistant" => {
            out.push(LineEvent::Text(s(&v["content"]).to_string()))
        }
        "tool_use" => {
            // Gemini {tool_name, parameters}; OpenCode {part:{tool, state:{input}}}
            let name = v["tool_name"]
                .as_str()
                .or(v["part"]["tool"].as_str())
                .unwrap_or("tool");
            let input = if v["parameters"].is_object() {
                &v["parameters"]
            } else {
                &v["part"]["state"]["input"]
            };
            out.push(LineEvent::Tool(
                format!("{name} {}", short_arg(input)).trim().to_string(),
            ))
        }
        "error" => out.push(LineEvent::Error(
            v["message"]
                .as_str()
                .or(v["error"]["message"].as_str())
                .or(v["error"].as_str())
                .unwrap_or("error")
                .to_string(),
        )),
        // Codex --json
        "thread.started" => out.push(LineEvent::Session(s(&v["thread_id"]).to_string())),
        "item.started" | "item.completed" => {
            let item = &v["item"];
            match s(&item["type"]) {
                "agent_message" if ty == "item.completed" => {
                    out.push(LineEvent::Text(format!("{}\n", s(&item["text"]))))
                }
                "command_execution" if ty == "item.started" => {
                    out.push(LineEvent::Tool(format!("shell {}", s(&item["command"]))))
                }
                "file_change" if ty == "item.completed" => {
                    let paths: Vec<&str> = item["changes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|c| s(&c["path"]))
                        .collect();
                    out.push(LineEvent::Tool(format!("edit {}", paths.join(", "))))
                }
                "mcp_tool_call" if ty == "item.started" => out.push(LineEvent::Tool(format!(
                    "{} {}",
                    s(&item["server"]),
                    s(&item["tool"])
                ))),
                "error" => out.push(LineEvent::Error(s(&item["message"]).to_string())),
                _ => {}
            }
        }
        "turn.completed" => {
            let usage = &v["usage"];
            out.push(LineEvent::Usage(RunUsage {
                model: String::new(),
                input_tokens: u(&usage["input_tokens"]),
                output_tokens: u(&usage["output_tokens"]),
                cache_read_tokens: u(&usage["cached_input_tokens"]),
                cost_usd: None,
                calls: 1,
            }))
        }
        "turn.failed" => out.push(LineEvent::Error(s(&v["error"]["message"]).to_string())),
        // OpenCode --format json
        "text" => out.push(LineEvent::Text(s(&v["part"]["text"]).to_string())),
        "step_finish" => {
            let part = &v["part"];
            out.push(LineEvent::Usage(RunUsage {
                model: String::new(),
                input_tokens: u(&part["tokens"]["input"]),
                output_tokens: u(&part["tokens"]["output"]),
                cache_read_tokens: u(&part["tokens"]["cache"]["read"]),
                cost_usd: part["cost"].as_f64(),
                calls: 1,
            }))
        }
        _ => {}
    }
    if out.is_empty() {
        out.push(LineEvent::Nothing);
    }
    out
}

/// Folds one usage report into a total.
pub fn add_usage(total: &mut Option<RunUsage>, u: &RunUsage) {
    let t = total.get_or_insert_with(RunUsage::default);
    if t.model.is_empty() {
        t.model = u.model.clone();
    }
    t.input_tokens += u.input_tokens;
    t.output_tokens += u.output_tokens;
    t.cache_read_tokens += u.cache_read_tokens;
    t.calls += u.calls;
    if let Some(c) = u.cost_usd {
        t.cost_usd = Some(t.cost_usd.unwrap_or(0.0) + c);
    }
}

/// The end of a run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunOutput {
    pub text: String,
    pub exit_code: Option<i32>,
    pub usage: Option<RunUsage>,
    pub session_id: Option<String>,
    pub error: Option<String>,
    pub stderr_tail: String,
    pub cancelled: bool,
}

/// Accumulates the events of a run into its output.
#[derive(Default)]
pub struct Collector {
    text: String,
    final_text: Option<String>,
    pub usage: Option<RunUsage>,
    pub session_id: Option<String>,
    pub error: Option<String>,
}

impl Collector {
    /// Folds one line in; returns the log line to show, if any.
    pub fn feed(&mut self, line: &str) -> Vec<String> {
        let mut log = Vec::new();
        for ev in parse_line(line) {
            match ev {
                LineEvent::Text(t) => self.text.push_str(&t),
                LineEvent::Final(t) => self.final_text = Some(t),
                LineEvent::Tool(t) => log.push(format!("→ {t}")),
                LineEvent::Usage(u) => {
                    // Claude sends one `result` per run with the totals.
                    add_usage(&mut self.usage, &u)
                }
                LineEvent::Session(id) if !id.is_empty() => self.session_id = Some(id),
                LineEvent::Error(e) if !e.is_empty() => {
                    log.push(format!("! {e}"));
                    self.error = Some(e)
                }
                _ => {}
            }
        }
        log
    }

    pub fn text(&self) -> String {
        self.final_text
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.text.clone())
    }
}

/// Variables never inherited by a child CLI: nesting markers of a Claude Code
/// session that started OmniGet, which make a nested `claude` refuse or
/// change behaviour. API keys are the user's own environment and stay.
pub const NESTING_ENV: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SSE_PORT",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
];

/// Runs the tool on this machine. `on_log` gets each tool line as it happens.
pub async fn run_local(
    run: &ToolRun,
    cancel: CancellationToken,
    mut on_log: impl FnMut(&str) + Send,
) -> Result<RunOutput, String> {
    let (target, r) = runner_of(&run.tool)?;
    let l = launch(target, r, run);
    if proc::which(&l.program).is_none() {
        return Err(format!(
            "{ERR_RUN}: `{}` not found: install {} first (Central → Tools)",
            l.program, target.name
        ));
    }
    for n in &l.notes {
        on_log(&format!("· {n}"));
    }
    let spec = proc::Spawn {
        command: l.program.clone(),
        args: l.args.clone(),
        env: vec![],
        env_remove: NESTING_ENV.iter().map(|s| s.to_string()).collect(),
        cwd: run.cwd.clone(),
    };
    drive(spec, cancel, &mut on_log).await
}

/// Spawns `spec`, streams stdout through a [`Collector`], stops on cancel.
pub async fn drive(
    spec: proc::Spawn,
    cancel: CancellationToken,
    on_log: &mut (dyn FnMut(&str) + Send),
) -> Result<RunOutput, String> {
    let mut child =
        proc::spawn(&spec).map_err(|e| format!("{ERR_RUN}: spawn {}: {e}", spec.command))?;
    // Headless runs read the prompt from argv; an open stdin makes some CLIs wait.
    drop(child.stdin.take());
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let tail = proc::Tail::default();
    let tail2 = tail.clone();
    let err_task = tokio::spawn(async move {
        if let Some(mut e) = stderr {
            let mut buf = vec![0u8; 4096];
            loop {
                match e.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => tail2.push(&String::from_utf8_lossy(&buf[..n])),
                }
            }
        }
    });
    let mut col = Collector::default();
    let mut cancelled = false;
    if let Some(out) = stdout {
        let mut lines = BufReader::new(out).lines();
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    cancelled = true;
                    proc::kill_tree(pid);
                    break;
                }
                line = lines.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            for log in col.feed(&l) {
                                on_log(&log);
                            }
                        }
                        _ => break,
                    }
                }
            }
        }
    }
    let status = tokio::select! {
        s = child.wait() => s.ok(),
        _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
            proc::kill_tree(pid);
            None
        }
    };
    let _ = err_task.await;
    let exit_code = status.and_then(|s| s.code());
    let stderr_tail = tail.text();
    let mut error = col.error.clone();
    if !cancelled
        && error.is_none()
        && exit_code.map(|c| c != 0).unwrap_or(true)
        && col.text().trim().is_empty()
    {
        let last = stderr_tail
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .to_string();
        error = Some(format!(
            "exit {}{}",
            exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into()),
            if last.is_empty() {
                String::new()
            } else {
                format!(": {last}")
            }
        ));
    }
    Ok(RunOutput {
        text: col.text(),
        exit_code,
        usage: col.usage,
        session_id: col.session_id,
        error,
        stderr_tail,
        cancelled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(tool: &str) -> ToolRun {
        ToolRun {
            tool: tool.into(),
            prompt: "fix it".into(),
            system_prompt: Some("You are a tester.".into()),
            model: Some("haiku".into()),
            permission: Permission::AcceptEdits,
            cwd: None,
        }
    }

    #[test]
    fn argv_per_tool_follows_the_manifests() {
        let (t, r) = runner_of("claude").unwrap();
        let l = launch(t, r, &run("claude"));
        assert_eq!(l.program, "claude");
        assert_eq!(l.args.last().unwrap(), "fix it");
        assert_eq!(l.args[l.args.len() - 2], "-p");
        assert!(l
            .args
            .windows(2)
            .any(|w| w == ["--append-system-prompt", "You are a tester."]));
        assert!(l
            .args
            .windows(2)
            .any(|w| w == ["--permission-mode", "acceptEdits"]));
        assert!(l.args.windows(2).any(|w| w == ["--model", "haiku"]));
        assert!(!l.args.iter().any(|a| a.contains("dangerously")));

        let (t, r) = runner_of("codex").unwrap();
        let l = launch(t, r, &run("codex"));
        assert_eq!(l.args[0], "exec");
        assert!(l.system_in_prompt);
        assert!(l.args.last().unwrap().contains("You are a tester."));
        assert!(l
            .args
            .windows(2)
            .any(|w| w == ["--sandbox", "workspace-write"]));

        let (t, r) = runner_of("gemini").unwrap();
        let l = launch(t, r, &run("gemini"));
        let n = l.args.len();
        assert_eq!(l.args[n - 2], "-p");

        let mut b = run("claude");
        b.permission = Permission::Bypass;
        let (t, r) = runner_of("claude").unwrap();
        assert_eq!(
            launch(t, r, &b).permission_flags,
            vec!["--dangerously-skip-permissions"]
        );
    }

    #[test]
    fn output_dialects_are_read() {
        let mut c = Collector::default();
        c.feed(r#"{"type":"system","subtype":"init","session_id":"s1"}"#);
        let log = c.feed(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi "},{"type":"tool_use","name":"Bash","input":{"command":"npm test"}}]}}"#);
        assert_eq!(log, vec!["→ Bash npm test"]);
        c.feed(r#"{"type":"result","result":"All green.","total_cost_usd":0.012,"usage":{"input_tokens":10,"output_tokens":5}}"#);
        assert_eq!(c.text(), "All green.");
        assert_eq!(c.session_id.as_deref(), Some("s1"));
        assert_eq!(c.usage.as_ref().unwrap().cost_usd, Some(0.012));

        let mut c = Collector::default();
        c.feed(
            r#"{"type":"item.started","item":{"type":"command_execution","command":"cargo test"}}"#,
        );
        c.feed(r#"{"type":"item.completed","item":{"type":"agent_message","text":"Done."}}"#);
        c.feed(r#"{"type":"turn.completed","usage":{"input_tokens":7,"output_tokens":3}}"#);
        assert_eq!(c.text().trim(), "Done.");
        assert_eq!(c.usage.as_ref().unwrap().input_tokens, 7);

        let mut c = Collector::default();
        c.feed("plain answer");
        assert_eq!(c.text().trim(), "plain answer");
    }
}
