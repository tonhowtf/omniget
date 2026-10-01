//! Codex turns over `codex app-server` (JSON-RPC over stdio), item 3.6 (S21,
//! `82#P7`, `81#P7`). The same Codex account, the same `CODEX_HOME`, the same
//! process tree, watchdog and cancel as every CLI turn (`session.rs`), but a
//! conversation instead of a one-shot `exec`:
//!
//! - `thread/resume` with the thread id the run registry kept for the
//!   conversation, so only the **new message** travels (the flattened
//!   transcript of `exec` grew with every turn). Codex no longer knowing the
//!   thread (`no rollout found`) falls back to `thread/start` with the replayed
//!   transcript; any other error fails the turn, never a silent fresh start;
//! - `turn/start` carries the agent's reasoning `effort` and the run mode's
//!   approval policy, sandbox and reviewer (the T3 table of
//!   `drivers::codex::launch::mode_config`, under the account's ceiling);
//! - every `item/*/requestApproval` becomes one durable ask of the broker
//!   (a `runs_permissions` row, 2.2), answerable from the window, the bridge,
//!   the CLI, MCP or the phone; the answer goes back as the JSON-RPC response;
//! - cancel sends `turn/interrupt` and kills the tree after
//!   [`session::INTERRUPT_GRACE`](super::session::INTERRUPT_GRACE).
//!
//! One process per turn, not per conversation: the process is created inside
//! the turn and is dead before its stream ends, exactly like `exec`, so the
//! watchdog, the reaper of `runs` and the job slots need nothing new. What a
//! long-lived process would buy (no resume cost) is a few hundred ms per turn;
//! what it would cost is a second process lifecycle to supervise.
//!
//! `exec --json` stays the fallback, picked by version: the app-server is
//! used only when the probe saw Codex ≥ [`MIN_VERSION`] answer
//! `codex app-server --help` (the t3code floor, `82#P7`), and never when
//! `OMNIGET_CODEX_TRANSPORT=exec` is set.
//!
//! Protocol reference: the JSON Schema of codex 0.158
//! (`codex app-server generate-json-schema`, kept in
//! `estudos/91-onda3/codex-app-server-schema/`), the Central's driver
//! (`drivers::codex`, same wire) and t3code (MIT) `CodexSessionRuntime.ts`.
//! This file never opens `auth.json`: the CLI logs itself in.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};

use serde_json::{json, Value};

use super::super::broker::{Answer, AskEnd, ToolBroker};
use super::super::drivers::codex::launch::{initialize_params, mode_config, ModeConfig};
use super::super::drivers::codex::protocol as p;
use super::super::drivers::codex::translate::{approval_result, PendingKind};
use super::super::drivers::ApprovalDecision;
use super::super::error::LlmError;
use super::super::run_mode::{Effective, RunMode};
use super::super::types::{FinishReason, TurnEvent, Usage};
use super::parse::{classify_error_text, CliSignal};
use super::session::{Duplex, LineParser};
use super::{ERR_CLI_EXIT, ERR_CLI_RATE};

/// The first Codex whose app-server speaks the v2 protocol this file uses
/// (t3code requires the same, `82#P7`).
pub const MIN_VERSION: (u32, u32) = (0, 156);

/// `exec` forces the one-shot fallback (escape hatch, and the "before" side
/// of BN10).
pub const ENV_TRANSPORT: &str = "OMNIGET_CODEX_TRANSPORT";

const ID_INIT: i64 = 1;
const ID_RESUME: i64 = 2;
const ID_START: i64 = 3;
const ID_TURN: i64 = 4;
const ID_INTERRUPT: i64 = 5;

/// Longest tool output handed back as a `ToolResult`.
const OUTPUT_CLIP: usize = 16 * 1024;

/// `0.158.0` → true. Anything unparsable is false: the fallback is safe.
pub fn version_supports(version: &str) -> bool {
    let mut parts = version
        .trim()
        .trim_start_matches('v')
        .split(['.', '-', ' '])
        .map(|p| p.parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(major), Some(minor)) => (major, minor) >= MIN_VERSION,
        _ => false,
    }
}

/// Whether this turn goes over the app-server (the caps say it exists and
/// nobody forced `exec`).
pub fn wanted(app_server: bool) -> bool {
    app_server
        && !std::env::var(ENV_TRANSPORT)
            .map(|v| v.trim().eq_ignore_ascii_case("exec"))
            .unwrap_or(false)
}

/// The approval policy, sandbox and reviewer of a run mode under the
/// account's ceiling. A read-only account never asks: Codex's "accept" would
/// run the command outside the sandbox, which the ceiling forbids.
pub fn mode_of(effective: Effective) -> ModeConfig {
    match effective {
        Effective::ReadOnly => {
            let mut c = mode_config(RunMode::ApprovalRequired.access());
            c.approval_policy = p::AskForApproval::Known(p::AskForApprovalKnown::Never);
            c
        }
        Effective::Mode(m) => mode_config(m.access()),
    }
}

/// A Codex effort word from the agent's `reasoning_effort` (Codex lists
/// `low|medium|high|xhigh|max|ultra`; `none|minimal` are older names).
pub fn effort_of(raw: &str) -> Option<String> {
    let w = raw.trim().to_ascii_lowercase();
    match w.as_str() {
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra" => Some(w),
        _ => None,
    }
}

/// Who answers Codex's questions: the broker's durable ask, on behalf of this
/// agent and this turn (the run id is the turn's request id, so the watchdog
/// pauses while a person decides and `approvals.rs` can cancel the job).
#[derive(Clone)]
pub struct Asker {
    pub broker: Weak<ToolBroker>,
    pub agent_id: String,
    pub request_id: String,
    pub conversation: String,
    /// The run registry of the turn, captured where the turn was planned: the
    /// ask runs in a task of its own, outside the coordinator's task-local.
    pub registry: Option<crate::core::assist::runs::Registry>,
}

/// Everything one app-server turn needs, decided by `CliRuntime`.
#[derive(Debug, Clone, Default)]
pub struct AppServerPlan {
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The agent's system prompt plus this turn's system messages
    /// (`developerInstructions`, sent on start and on resume).
    pub instructions: Option<String>,
    /// The Codex thread to resume (the run registry's provider handle).
    pub resume: Option<String>,
    /// What a resumed turn sends: the new message alone.
    pub resume_text: String,
    /// What a fresh thread gets: the new message, or the replayed transcript
    /// when the conversation has history Codex does not hold.
    pub fresh_text: String,
    pub effective: Option<Effective>,
}

impl AppServerPlan {
    fn mode(&self) -> ModeConfig {
        mode_of(
            self.effective
                .unwrap_or(Effective::Mode(RunMode::default())),
        )
    }

    fn thread_params(&self) -> serde_json::Map<String, Value> {
        let mode = self.mode();
        let mut m = serde_json::Map::new();
        if let Some(cwd) = &self.cwd {
            m.insert("cwd".into(), json!(cwd));
        }
        if let Some(model) = &self.model {
            m.insert("model".into(), json!(model));
        }
        m.insert("approvalPolicy".into(), json!(mode.approval_policy));
        m.insert("sandbox".into(), json!(mode.sandbox));
        m.insert("approvalsReviewer".into(), json!(mode.reviewer));
        if let Some(i) = self.instructions.as_ref().filter(|i| !i.trim().is_empty()) {
            m.insert("developerInstructions".into(), json!(i));
        }
        m
    }

    /// The lines written before the first answer: `initialize`,
    /// `initialized`, then `thread/resume` or `thread/start` (pipelined; the
    /// server handles them in order).
    pub fn prelude(&self) -> String {
        let mut lines = vec![
            json!({ "id": ID_INIT, "method": "initialize", "params": initialize_params() }),
            json!({ "method": "initialized" }),
        ];
        lines.push(match &self.resume {
            Some(thread) => self.resume_request(thread),
            None => self.start_request(),
        });
        let mut out = String::new();
        for l in lines {
            out.push_str(&l.to_string());
            out.push('\n');
        }
        out
    }

    fn resume_request(&self, thread: &str) -> Value {
        let mut m = self.thread_params();
        m.insert("threadId".into(), json!(thread));
        // History items of any version must never break a resume.
        m.insert("excludeTurns".into(), json!(true));
        json!({ "id": ID_RESUME, "method": "thread/resume", "params": m })
    }

    fn start_request(&self) -> Value {
        json!({ "id": ID_START, "method": "thread/start", "params": self.thread_params() })
    }

    /// `turn/start`: the text, the effort, and the mode again (Codex keeps
    /// turn overrides for later turns, so every turn states its own).
    pub fn turn_request(&self, thread: &str, text: &str) -> Value {
        let mode = self.mode();
        let mut m = serde_json::Map::new();
        m.insert("threadId".into(), json!(thread));
        m.insert(
            "input".into(),
            json!([{ "type": "text", "text": text, "text_elements": [] }]),
        );
        m.insert("approvalPolicy".into(), json!(mode.approval_policy));
        m.insert("approvalsReviewer".into(), json!(mode.reviewer));
        m.insert("sandboxPolicy".into(), json!(mode.sandbox_policy));
        if let Some(cwd) = &self.cwd {
            m.insert("cwd".into(), json!(cwd));
        }
        if let Some(model) = &self.model {
            m.insert("model".into(), json!(model));
        }
        if let Some(effort) = &self.effort {
            m.insert("effort".into(), json!(effort));
        }
        json!({ "id": ID_TURN, "method": "turn/start", "params": m })
    }
}

/// `thread/resume` failed because Codex does not have that thread (the
/// driver's rule, same wording): start a fresh one with the replay.
fn missing_thread(message: &str) -> bool {
    let msg = message.to_lowercase();
    (msg.contains("thread")
        && [
            "not found",
            "missing thread",
            "no such thread",
            "unknown thread",
            "does not exist",
        ]
        .iter()
        .any(|w| msg.contains(w)))
        || msg.contains("no rollout found")
}

#[derive(Default)]
struct State {
    thread: Option<String>,
    turn: Option<String>,
    resumed: bool,
    started: bool,
    errored: bool,
    finished: bool,
    /// Agent messages that streamed deltas (their `completed` text is not
    /// repeated).
    streamed: HashSet<String>,
    /// Tool items opened with `ToolCallStart` and not yet answered.
    open: HashSet<String>,
    /// File change item → its paths (the approval request only names the item).
    paths: HashMap<String, Vec<String>>,
    /// Command item → its command line.
    commands: HashMap<String, String>,
    /// Items OmniGet declined for a person (the note says "the answer was no").
    declined_by_person: HashSet<String>,
    usage: Usage,
    saw_usage: bool,
}

/// Reads the app-server's stdout and writes its follow-ups (see module doc).
pub struct AppServerParser {
    plan: AppServerPlan,
    state: Arc<Mutex<State>>,
    out: tokio::sync::mpsc::UnboundedSender<String>,
    outbox: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<String>>>,
    asker: Option<Asker>,
    /// Said next to a call nobody allowed (2.7, S5).
    no_surface_note: String,
    refused_note: String,
}

impl AppServerParser {
    pub fn new(plan: AppServerPlan, asker: Option<Asker>) -> Self {
        let effective = plan
            .effective
            .unwrap_or(Effective::Mode(RunMode::default()));
        let (out, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            no_surface_note: super::super::run_mode::denial_note(effective, false),
            refused_note: super::super::run_mode::denial_note(effective, true),
            plan,
            state: Arc::new(Mutex::new(State::default())),
            out,
            outbox: Mutex::new(Some(rx)),
            asker,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, v: Value) {
        let _ = self.out.send(v.to_string());
    }

    fn fail(&self, message: &str, info: Option<&Value>, out: &mut Vec<CliSignal>) {
        let mut st = self.state();
        if !st.errored {
            st.errored = true;
            let rate = info
                .and_then(Value::as_str)
                .is_some_and(|i| i == "usageLimitExceeded")
                || info.and_then(|i| i.get("usageLimitExceeded")).is_some();
            let code = if rate {
                ERR_CLI_RATE
            } else {
                classify_error_text(message).unwrap_or(ERR_CLI_EXIT)
            };
            let mut error = LlmError::new(code, message.to_string());
            error.retryable = code == ERR_CLI_RATE;
            out.push(CliSignal::Event(TurnEvent::Error { error }));
        }
    }

    fn finish(&self, reason: FinishReason, out: &mut Vec<CliSignal>) {
        let mut st = self.state();
        if st.finished {
            return;
        }
        st.finished = true;
        if st.saw_usage {
            out.push(CliSignal::Event(TurnEvent::Usage {
                usage: st.usage.clone(),
            }));
        }
        out.push(CliSignal::Event(TurnEvent::Finished { reason }));
    }

    fn on_response(&self, id: i64, value: &Value, out: &mut Vec<CliSignal>) {
        let error = value.get("error").map(|e| {
            e.get("message")
                .and_then(Value::as_str)
                .unwrap_or("app-server error")
                .to_string()
        });
        match id {
            ID_INIT => {
                if let Some(e) = error {
                    self.fail(&format!("Codex app-server did not start: {e}"), None, out);
                    self.finish(FinishReason::Other, out);
                }
            }
            ID_RESUME | ID_START => {
                if let Some(e) = error {
                    if id == ID_RESUME && missing_thread(&e) {
                        tracing::warn!("[codex] thread is gone ({e}); starting a new one");
                        self.send(self.plan.start_request());
                        return;
                    }
                    let what = if id == ID_RESUME {
                        "thread/resume"
                    } else {
                        "thread/start"
                    };
                    self.fail(&format!("{what} failed: {e}"), None, out);
                    self.finish(FinishReason::Other, out);
                    return;
                }
                let Some(thread) = value
                    .get("result")
                    .and_then(|r| r.get("thread"))
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                else {
                    self.fail("the app-server opened no thread", None, out);
                    self.finish(FinishReason::Other, out);
                    return;
                };
                let text = {
                    let mut st = self.state();
                    st.thread = Some(thread.clone());
                    st.resumed = id == ID_RESUME;
                    if st.resumed {
                        self.plan.resume_text.clone()
                    } else {
                        self.plan.fresh_text.clone()
                    }
                };
                out.push(CliSignal::Session(thread.clone()));
                self.send(self.plan.turn_request(&thread, &text));
            }
            ID_TURN => {
                if let Some(e) = error {
                    self.fail(&format!("turn/start failed: {e}"), None, out);
                    self.finish(FinishReason::Other, out);
                    return;
                }
                if let Some(turn) = value
                    .get("result")
                    .and_then(|r| r.get("turn"))
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                {
                    self.state().turn = Some(turn.to_string());
                }
            }
            _ => {}
        }
    }

    fn on_notification(&self, method: &str, params: &Value, out: &mut Vec<CliSignal>) {
        match method {
            "turn/started" => {
                let id = params
                    .get("turn")
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("codex-turn")
                    .to_string();
                let mut st = self.state();
                st.turn.get_or_insert(id.clone());
                if !st.started {
                    st.started = true;
                    out.push(CliSignal::Event(TurnEvent::Started { request_id: id }));
                }
            }
            "item/agentMessage/delta" => {
                if let Some(d) = params.get("delta").and_then(Value::as_str) {
                    if let Some(item) = params.get("itemId").and_then(Value::as_str) {
                        self.state().streamed.insert(item.to_string());
                    }
                    out.push(CliSignal::Event(TurnEvent::TextDelta {
                        text: d.to_string(),
                    }));
                }
            }
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                if let Some(d) = params.get("delta").and_then(Value::as_str) {
                    out.push(CliSignal::Event(TurnEvent::ThinkingDelta {
                        text: d.to_string(),
                    }));
                }
            }
            "item/started" => {
                if let Some(item) = params.get("item") {
                    self.item_started(item, out);
                }
            }
            "item/completed" => {
                if let Some(item) = params.get("item") {
                    self.item_completed(item, out);
                }
            }
            "thread/tokenUsage/updated" => {
                // `last` is one model request; a turn is the sum of its
                // requests. `inputTokens` holds the cache (OpenAI convention,
                // the same as `Usage`).
                if let Some(last) = params.get("tokenUsage").and_then(|u| u.get("last")) {
                    let n = |k: &str| last.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
                    let mut st = self.state();
                    st.saw_usage = true;
                    let u = &mut st.usage;
                    u.input_tokens = u.input_tokens.saturating_add(n("inputTokens"));
                    u.cache_read_tokens =
                        u.cache_read_tokens.saturating_add(n("cachedInputTokens"));
                    u.cache_write_tokens = u
                        .cache_write_tokens
                        .saturating_add(n("cacheWriteInputTokens"));
                    u.output_tokens = u.output_tokens.saturating_add(n("outputTokens"));
                }
            }
            "error" => {
                if params.get("willRetry").and_then(Value::as_bool) == Some(true) {
                    return;
                }
                let e = params.get("error").cloned().unwrap_or(Value::Null);
                let message = e
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex reported an error");
                self.fail(message, e.get("codexErrorInfo"), out);
            }
            "turn/completed" => {
                let turn = params.get("turn").cloned().unwrap_or(Value::Null);
                match turn.get("status").and_then(Value::as_str).unwrap_or("") {
                    "completed" => self.finish(FinishReason::Stop, out),
                    "interrupted" => self.finish(FinishReason::Cancelled, out),
                    _ => {
                        let e = turn.get("error").cloned().unwrap_or(Value::Null);
                        let message = e
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("the Codex turn failed");
                        self.fail(message, e.get("codexErrorInfo"), out);
                        self.finish(FinishReason::Other, out);
                    }
                }
            }
            _ => {}
        }
    }

    fn item_started(&self, item: &Value, out: &mut Vec<CliSignal>) {
        let Some((id, name, input)) = tool_of(item) else {
            return;
        };
        {
            let mut st = self.state();
            if !st.open.insert(id.clone()) {
                return;
            }
            if let Some(cmd) = input.get("command").and_then(Value::as_str) {
                st.commands.insert(id.clone(), cmd.to_string());
            }
            if let Some(paths) = input.get("paths").and_then(Value::as_array) {
                st.paths.insert(
                    id.clone(),
                    paths
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                );
            }
        }
        out.push(CliSignal::Event(TurnEvent::ToolCallStart {
            id: id.clone(),
            name,
        }));
        out.push(CliSignal::Event(TurnEvent::ToolCallDelta {
            id: id.clone(),
            input_json_delta: input.to_string(),
        }));
        out.push(CliSignal::Event(TurnEvent::ToolCallEnd { id }));
    }

    fn item_completed(&self, item: &Value, out: &mut Vec<CliSignal>) {
        let id = item.get("id").and_then(Value::as_str).unwrap_or("item");
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "agentMessage" => {
                if self.state().streamed.contains(id) {
                    return;
                }
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    out.push(CliSignal::Event(TurnEvent::TextDelta {
                        text: text.to_string(),
                    }));
                }
            }
            _ => {
                // A tool that only ever completes (a file change with no
                // `started`) still opens before it closes.
                if !self.state().open.contains(id) {
                    self.item_started(item, out);
                }
                if tool_of(item).is_none() {
                    return;
                }
                let status = item.get("status").and_then(Value::as_str).unwrap_or("");
                let (content, is_error) = match status {
                    "declined" => {
                        let by_person = self.state().declined_by_person.contains(id);
                        (
                            if by_person {
                                self.refused_note.clone()
                            } else {
                                self.no_surface_note.clone()
                            },
                            true,
                        )
                    }
                    "failed" => (output_of(item).unwrap_or_else(|| "failed".into()), true),
                    _ => (output_of(item).unwrap_or_default(), false),
                };
                self.state().open.remove(id);
                out.push(CliSignal::Event(TurnEvent::ToolResult {
                    id: id.to_string(),
                    content,
                    is_error,
                }));
            }
        }
    }

    /// A question of Codex. Approvals go to the broker (durable, answerable
    /// from anywhere) in a task of their own: the turn keeps streaming while
    /// a person decides. Everything OmniGet cannot answer is refused at once,
    /// so Codex never waits on a question nobody sees.
    fn on_request(&self, id: Value, method: &str, params: &Value) {
        let kind = match method {
            "item/commandExecution/requestApproval" => PendingKind::Command,
            "item/fileChange/requestApproval" => PendingKind::FileChange,
            "item/permissions/requestApproval" => PendingKind::Permissions,
            "execCommandApproval" => PendingKind::LegacyExec,
            "applyPatchApproval" => PendingKind::LegacyPatch,
            "mcpServer/elicitation/request" => {
                self.send(json!({ "id": id, "result": { "action": "decline" } }));
                return;
            }
            // Credentials are the CLI's own business; OmniGet never holds them.
            _ => {
                self.send(json!({ "id": id, "error": {
                    "code": -32601,
                    "message": format!("OmniGet does not answer `{method}` in a background turn"),
                }}));
                return;
            }
        };
        let item = params
            .get("itemId")
            .or_else(|| params.get("callId"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let (label, input, preview) = self.question(kind, &item, params);
        let Some(asker) = self.asker.clone() else {
            self.send(json!({ "id": id, "result": approval_result(kind, ApprovalDecision::Decline, params) }));
            return;
        };
        let out = self.out.clone();
        let state = self.state.clone();
        let params = params.clone();
        let call_id = format!(
            "codex-{}",
            if item.is_empty() {
                uuid::Uuid::new_v4().to_string()
            } else {
                item.clone()
            }
        );
        tokio::spawn(async move {
            let decision = decide(&asker, &call_id, &label, &input, &preview).await;
            if !decision.allows() {
                state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .declined_by_person
                    .insert(item);
            }
            let reply = json!({ "id": id, "result": approval_result(kind, decision, &params) });
            let _ = out.send(reply.to_string());
        });
    }

    /// (rule label, rule input, preview) of a question. Commands and file
    /// edits use the native tool names, so one "Always" rule of the agent
    /// (`perm.rs`) answers the same way for the native harness and Codex.
    fn question(&self, kind: PendingKind, item: &str, params: &Value) -> (String, Value, String) {
        let reason = params
            .get("reason")
            .and_then(Value::as_str)
            .map(|r| format!("\n{r}"))
            .unwrap_or_default();
        match kind {
            PendingKind::Command | PendingKind::LegacyExec => {
                let cmd = match params.get("command") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Array(a)) => a
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" "),
                    _ => self.state().commands.get(item).cloned().unwrap_or_default(),
                };
                (
                    "shell_exec".into(),
                    json!({ "command": cmd }),
                    format!("{cmd}{reason}"),
                )
            }
            PendingKind::FileChange | PendingKind::LegacyPatch => {
                let mut paths = self.state().paths.get(item).cloned().unwrap_or_default();
                if let Some(Value::Object(changes)) = params.get("fileChanges") {
                    paths.extend(changes.keys().cloned());
                }
                (
                    "fs_edit".into(),
                    json!({ "path": paths.first().cloned().unwrap_or_default() }),
                    format!("edit {}{reason}", paths.join(", ")),
                )
            }
            _ => (
                "codex:permissions".into(),
                json!({}),
                format!(
                    "more access: {}{reason}",
                    params
                        .get("permissions")
                        .map(Value::to_string)
                        .unwrap_or_default()
                ),
            ),
        }
    }
}

/// The agent's rules first (an "Always" or a deny of `perm.rs`), else a
/// person through the broker. Nobody answering is a decline, never a yes.
async fn decide(
    asker: &Asker,
    call_id: &str,
    label: &str,
    input: &Value,
    preview: &str,
) -> ApprovalDecision {
    use super::super::perm;
    let ruled = if label.starts_with("codex:") {
        None
    } else {
        perm::decide(&asker.agent_id, label, input)
    };
    match ruled {
        Some(perm::Action::Allow) => return ApprovalDecision::Accept,
        Some(perm::Action::Deny) => return ApprovalDecision::Decline,
        _ => {}
    }
    let Some(broker) = asker.broker.upgrade() else {
        return ApprovalDecision::Decline;
    };
    let end = crate::core::assist::runs::scope(
        asker.registry.clone(),
        super::super::code_tools::scope(
            &asker.conversation,
            &asker.agent_id,
            &asker.request_id,
            broker.ask_user_ended(&asker.agent_id, &asker.request_id, call_id, label, preview),
        ),
    )
    .await;
    match end {
        AskEnd::Answered(Answer::Once) => ApprovalDecision::Accept,
        AskEnd::Answered(Answer::Always) => {
            if !label.starts_with("codex:") {
                perm::add_rule(&asker.agent_id, perm::always_rule(label, input));
            }
            ApprovalDecision::AcceptForSession
        }
        _ => ApprovalDecision::Decline,
    }
}

/// `(id, display name, input)` of an item that is a tool call.
fn tool_of(item: &Value) -> Option<(String, String, Value)> {
    let id = item.get("id").and_then(Value::as_str)?.to_string();
    let s = |k: &str| {
        item.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    Some(match item.get("type").and_then(Value::as_str)? {
        "commandExecution" => {
            let cmd = s("command");
            let name = cmd
                .split_whitespace()
                .next()
                .unwrap_or("command")
                .to_string();
            (id, name, json!({ "command": cmd, "cwd": s("cwd") }))
        }
        "fileChange" => {
            let paths: Vec<String> = item
                .get("changes")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.get("path").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            (
                id,
                format!("apply_patch({})", paths.len()),
                json!({ "paths": paths }),
            )
        }
        "mcpToolCall" => (
            id,
            format!("{}/{}", s("server"), s("tool")),
            item.get("arguments").cloned().unwrap_or(json!({})),
        ),
        "dynamicToolCall" => (
            id,
            s("tool"),
            item.get("arguments").cloned().unwrap_or(json!({})),
        ),
        "webSearch" => (id, "web_search".into(), json!({ "query": s("query") })),
        "collabAgentToolCall" => (id, s("tool"), json!({})),
        _ => return None,
    })
}

fn output_of(item: &Value) -> Option<String> {
    let text = item
        .get("aggregatedOutput")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            item.get("error")
                .and_then(|e| e.get("message").or(Some(e)))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| item.get("result").map(Value::to_string))?;
    let mut text = text;
    if text.len() > OUTPUT_CLIP {
        let mut cut = text.len() - OUTPUT_CLIP;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        text = text.split_off(cut);
    }
    Some(text)
}

impl LineParser for AppServerParser {
    fn parse_line(&self, line: &str) -> Vec<CliSignal> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return vec![CliSignal::Ignored];
        };
        let mut out = Vec::new();
        let method = value.get("method").and_then(Value::as_str);
        match (value.get("id"), method) {
            (Some(id), Some(method)) => self.on_request(id.clone(), method, &value["params"]),
            (Some(id), None) => {
                if let Some(id) = id.as_i64() {
                    self.on_response(id, &value, &mut out);
                }
            }
            (None, Some(method)) => {
                let params = value.get("params").cloned().unwrap_or(Value::Null);
                self.on_notification(method, &params, &mut out);
            }
            _ => {}
        }
        if out.is_empty() {
            out.push(CliSignal::Ignored);
        }
        out
    }

    fn duplex(&self) -> Option<Duplex> {
        let outbox = self.outbox.lock().ok()?.take()?;
        let state = self.state.clone();
        Some(Duplex {
            outbox,
            interrupt: Some(Arc::new(move || {
                let st = state.lock().unwrap_or_else(|e| e.into_inner());
                let (Some(thread), Some(turn)) = (st.thread.clone(), st.turn.clone()) else {
                    return None;
                };
                if st.finished {
                    return None;
                }
                Some(
                    json!({
                        "id": ID_INTERRUPT,
                        "method": "turn/interrupt",
                        "params": { "threadId": thread, "turnId": turn },
                    })
                    .to_string(),
                )
            })),
        })
    }
}
