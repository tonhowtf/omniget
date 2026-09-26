//! One table for every internal tool: the embedded MCP server
//! (`src-tauri/src/mcp.rs`), the LLM tool broker and the grant UI all read it,
//! so a tool is described (name, schema, category, cost) exactly once.
//! Owned by f3-tool-table.
//!
//! Two kinds of entry:
//!
//! - [`ToolImpl::Core`]: the whole call lives here, over `core::tools::*`. No
//!   `AppHandle`, so the core can run it and the tests can call it directly.
//! - [`ToolImpl::Host`]: the call needs the desktop app (the download queue,
//!   the torrent session, the extension cookie store). The table still owns the
//!   name and the schema; the body stays in `mcp.rs` behind [`HostTools`].
//!
//! The table is built once into a `OnceLock`, so `tools/list` and every
//! dispatch are a slice scan, not 37 `json!` allocations.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};

use super::types::ToolSpec;

/// The future a [`ToolImpl::Core`] call returns. Boxed so the table can hold
/// plain `fn` pointers instead of one type per tool.
pub type BoxFut = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;

/// One tool body that runs entirely inside the core.
pub type CoreCall = fn(Value) -> BoxFut;

/// The tools whose body needs the desktop app. `name` is the table name, so an
/// implementation is one `match` with no schema knowledge.
#[async_trait]
pub trait HostTools: Send + Sync {
    async fn call(&self, name: &str, input: Value) -> Result<Value, String>;
}

/// Where the body of a tool lives.
#[derive(Clone, Copy)]
pub enum ToolImpl {
    Core(CoreCall),
    Host,
}

impl std::fmt::Debug for ToolImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolImpl::Core(_) => f.write_str("Core"),
            ToolImpl::Host => f.write_str("Host"),
        }
    }
}

/// What one call is likely to cost the user, for the grant UI. Never a
/// measurement: it is the shape of the call, not its duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CostHint {
    /// Local, cheap, read-only (a listing, a lookup).
    Cheap,
    /// Local but heavy: CPU, disk or a spawned binary.
    Local,
    /// Talks to the network without an account.
    Network,
    /// Spends money or a quota (an AI key, a paid account).
    Paid,
}

/// One tool, described once for every consumer.
pub struct ToolEntry {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub call: ToolImpl,
    /// The group this tool belongs to (help, code, downloads).
    pub category: &'static str,
    pub cost_hint: CostHint,
}

impl ToolEntry {
    /// The broker/provider view of this tool.
    pub fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.to_string(),
            description: self.description.to_string(),
            input_schema: self.input_schema.clone(),
        }
    }

    pub fn needs_host(&self) -> bool {
        matches!(self.call, ToolImpl::Host)
    }
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

// ── Argument helpers (shared with the host half in `mcp.rs`) ───────────

pub fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

pub fn list(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

pub fn num(v: &Value, k: &str) -> Option<u64> {
    v.get(k).and_then(|x| x.as_u64())
}

/// A required string argument, with a readable error when it is missing.
pub fn need_str(v: &Value, k: &str) -> Result<String, String> {
    let value = s(v, k);
    if value.trim().is_empty() {
        return Err(format!("argument \"{}\" is required (string)", k));
    }
    Ok(value)
}

/// A download id: accepts a number or a numeric string.
pub fn need_id(v: &Value, k: &str) -> Result<u64, String> {
    v.get(k)
        .and_then(|x| {
            x.as_u64()
                .or_else(|| x.as_str().and_then(|t| t.trim().parse().ok()))
        })
        .ok_or_else(|| format!("argument \"{}\" is required (download id, integer)", k))
}

pub fn to_json<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// The queue status keys the downloads tools accept, and the ones
/// `QueueStatus` serialises to. The app asserts both sides still match.
pub const QUEUE_STATUS_KEYS: &[&str] =
    &["queued", "active", "paused", "seeding", "complete", "error"];

// ── The table ─────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn entry(
    name: &'static str,
    description: &'static str,
    input_schema: Value,
    call: ToolImpl,
    category: &'static str,
    cost_hint: CostHint,
) -> ToolEntry {
    ToolEntry {
        name,
        description,
        input_schema,
        call,
        category,
        cost_hint,
    }
}

fn core(f: CoreCall) -> ToolImpl {
    ToolImpl::Core(f)
}

/// Every internal tool. Built once; call [`table`] to read it.
fn build() -> Vec<ToolEntry> {
    use CostHint::{Cheap, Local, Network, Paid};
    vec![
        entry("help_connection_check", "Check local installation evidence without login or paid requests. Unknown authentication is not success.", obj(json!({"connectionId":{"type":"string"}}), &["connectionId"]), ToolImpl::Host, "help", Local),
        entry("help_diagnostic_run", "Run a selected, allowlisted local check. Never runs a paid model probe.", obj(json!({"checkId":{"type":"string","enum":["connection","documentation"]},"targetId":{"type":"string"},"locale":{"type":"string"}}), &["checkId"]), ToolImpl::Host, "help", Local),
        entry("help_docs_search", "Search bundled OmniGet help. Returns versioned citation sources.", obj(json!({"query":{"type":"string"},"locale":{"type":"string"},"limit":{"type":"integer","maximum":8}}), &["query"]), ToolImpl::Host, "help", Cheap),
        entry("help_docs_read", "Read a bundled OmniGet help article.", obj(json!({"articleId":{"type":"string"},"locale":{"type":"string"}}), &["articleId"]), ToolImpl::Host, "help", Cheap),
        entry("help_setup_inspect", "Inspect sanitized agent connections without secrets or network calls.", obj(json!({}), &[]), ToolImpl::Host, "help", Cheap),
        entry("help_agent_plan", "Plan a new agent or edit an existing agent name/connection. New agents have no tools; edits preserve permissions. Does not save.", obj(json!({"sourceAgentId":{"type":"string"},"name":{"type":"string"},"agentId":{"type":"string"}}), &["sourceAgentId", "name"]), ToolImpl::Host, "help", Cheap),
        entry("help_agent_apply", "Apply a previously reviewed agent plan idempotently. Requires the exact revision.", obj(json!({"planId":{"type":"string"},"expectedRevision":{"type":"string"},"idempotencyKey":{"type":"string"}}), &["planId", "expectedRevision", "idempotencyKey"]), ToolImpl::Host, "help", Cheap),
        // ── Coding harness (core/llm/code_tools.rs): confined to the workspace ──
        entry("fs_read", "Read a text file of the workspace with line numbers. A directory path lists it. Use offset/limit for long files.",
            obj(json!({ "path": { "type": "string" }, "offset": { "type": "integer", "minimum": 1 }, "limit": { "type": "integer", "minimum": 1 } }), &["path"]),
            core(|a| Box::pin(async move { super::code_tools::fs_read(&a) })), "code", Local),
        entry("fs_list", "List files and folders of the workspace (skips .git, node_modules, target).",
            obj(json!({ "path": { "type": "string" }, "depth": { "type": "integer", "minimum": 1, "maximum": 8 } }), &[]),
            core(|a| Box::pin(async move { super::code_tools::fs_list(&a) })), "code", Local),
        entry("fs_glob", "Find files by glob pattern (e.g. **/*.rs, src/**/*.{ts,svelte}).",
            obj(json!({ "pattern": { "type": "string" }, "path": { "type": "string" } }), &["pattern"]),
            core(|a| Box::pin(async move { super::code_tools::fs_glob(&a) })), "code", Local),
        entry("fs_grep", "Search file contents by regex; returns path:line: text.",
            obj(json!({ "pattern": { "type": "string" }, "path": { "type": "string" }, "include": { "type": "string" }, "literal_text": { "type": "boolean" } }), &["pattern"]),
            core(|a| Box::pin(async move { super::code_tools::fs_grep(&a) })), "code", Local),
        entry("fs_edit", "Replace old_string with new_string in a file. old_string must match exactly one place unless replace_all is true. Empty old_string creates a new file.",
            obj(json!({ "path": { "type": "string" }, "old_string": { "type": "string" }, "new_string": { "type": "string" }, "replace_all": { "type": "boolean" } }), &["path", "old_string", "new_string"]),
            core(|a| Box::pin(async move { super::code_tools::fs_edit(&a) })), "code", Local),
        entry("fs_write", "Create or overwrite a whole file.",
            obj(json!({ "path": { "type": "string" }, "content": { "type": "string" } }), &["path", "content"]),
            core(|a| Box::pin(async move { super::code_tools::fs_write(&a) })), "code", Local),
        entry("fs_apply_patch", "Apply a multi-file patch in the '*** Begin Patch' envelope (*** Add File / *** Update File with @@ hunks of ' ', '-', '+' lines / *** Delete File / *** End Patch).",
            obj(json!({ "patch": { "type": "string" } }), &["patch"]),
            core(|a| Box::pin(async move { super::code_tools::fs_apply_patch(&a) })), "code", Local),
        entry("shell_exec", "Run a shell command inside the workspace. Sandboxed on macOS: writes only inside the workspace, no network. Runs at the workspace root; paths are relative to it and workdir (optional) is a folder relative to it. Say what the command does in description.",
            obj(json!({ "command": { "type": "string" }, "description": { "type": "string" }, "workdir": { "type": "string" }, "timeout_ms": { "type": "integer", "minimum": 1000 } }), &["command", "description"]),
            core(|a| Box::pin(async move { super::code_tools::shell_exec(a).await })), "code", Local),
        entry("todo_write", "Publish or update the plan for the task: a list of steps with status pending, in_progress or completed.",
            obj(json!({ "plan": { "type": "array", "items": { "type": "object", "properties": { "step": { "type": "string" }, "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] } }, "required": ["step", "status"] } }, "explanation": { "type": "string" } }), &["plan"]),
            core(|a| Box::pin(async move { super::code_tools::todo_write(&a) })), "code", Local),
        entry("kb_search", "Search the project knowledge base (.omniget/kb, markdown notes written by the team of agents): decisions, conventions, how things are run, what was already tried. Search before you explore a codebase you were in before.",
            obj(json!({ "query": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 20 } }), &["query"]),
            core(|a| Box::pin(async move { super::kb::kb_search(a).await })), "code", Cheap),
        entry("kb_write", "Save a note to the project knowledge base so the next agent (or you, tomorrow) does not rediscover it: a decision and its reason, a command that works, a trap. One topic per note; `append` adds to an existing note with the same title.",
            obj(json!({ "title": { "type": "string" }, "content": { "type": "string", "description": "markdown" }, "append": { "type": "boolean" } }), &["title", "content"]),
            core(|a| Box::pin(async move { super::kb::kb_write(&a) })), "code", Cheap),
        entry("agent_delegate", "Hand a self-contained sub-task to another agent of the roster and get its final answer back. The other agent works in the same workspace, with its own tools and budget. Use it for work that fits another role better or can be isolated.",
            obj(json!({ "agent_id": { "type": "string", "description": "id of the roster agent" }, "task": { "type": "string", "description": "everything the other agent needs to know; it does not see this conversation" } }), &["agent_id", "task"]),
            ToolImpl::Host, "code", Paid),
        entry(
            "download_url",
            "Queue a URL (video, audio, playlist, course, image) in the OmniGet Downloads panel. Same as pasting it in the app.",
            obj(json!({ "url": { "type": "string" } }), &["url"]),
            ToolImpl::Host,
            "downloads",
            Network,
        ),
        entry(
            "download_enqueue",
            "Queue a URL in the Downloads panel using the app defaults, and return the queue item. mode audio downloads audio only. Always supply a stable idempotencyKey per user intent; reuse it on retries.",
            obj(json!({ "url": { "type": "string" }, "mode": { "type": "string", "enum": ["video", "audio"] }, "idempotencyKey": { "type": "string" } }), &["url"]),
            ToolImpl::Host,
            "downloads",
            Network,
        ),
        entry(
            "downloads_queue",
            "List the Downloads queue with per-item status, progress, speed, ETA and output path.",
            obj(json!({ "status": { "type": "string", "enum": QUEUE_STATUS_KEYS }, "limit": { "type": "integer" } }), &[]),
            ToolImpl::Host,
            "downloads",
            Cheap,
        ),
        entry(
            "download_status",
            "One download by id: status, percent, speed, ETA, file path and the last yt-dlp command.",
            obj(json!({ "download_id": { "type": "integer" } }), &["download_id"]),
            ToolImpl::Host,
            "downloads",
            Cheap,
        ),
        entry(
            "download_cancel",
            "Cancel a download by id (queued, active, paused or seeding).",
            obj(json!({ "download_id": { "type": "integer" } }), &["download_id"]),
            ToolImpl::Host,
            "downloads",
            Cheap,
        ),
        entry(
            "download_pause",
            "Pause an active download by id.",
            obj(json!({ "download_id": { "type": "integer" } }), &["download_id"]),
            ToolImpl::Host,
            "downloads",
            Cheap,
        ),
        entry(
            "download_resume",
            "Resume a paused download by id.",
            obj(json!({ "download_id": { "type": "integer" } }), &["download_id"]),
            ToolImpl::Host,
            "downloads",
            Cheap,
        ),
    ]
}

/// The table, built once.
pub fn table() -> &'static [ToolEntry] {
    static TABLE: OnceLock<Vec<ToolEntry>> = OnceLock::new();
    TABLE.get_or_init(build)
}

/// One entry by name.
pub fn find(name: &str) -> Option<&'static ToolEntry> {
    table().iter().find(|e| e.name == name)
}

/// Every tool as a `ToolSpec`, in table order: what the broker registers and
/// what a provider sees.
pub fn specs() -> Vec<ToolSpec> {
    table().iter().map(ToolEntry::spec).collect()
}

/// Tool names in table order.
pub fn names() -> Vec<&'static str> {
    table().iter().map(|e| e.name).collect()
}

/// Run one tool. `host` is the desktop half; without it, a
/// [`ToolImpl::Host`] tool fails instead of pretending.
pub async fn dispatch(
    name: &str,
    input: Value,
    host: Option<&dyn HostTools>,
) -> Result<Value, String> {
    let Some(entry) = find(name) else {
        return Err(format!("unknown tool: {}", name));
    };
    match entry.call {
        ToolImpl::Core(f) => f(input).await,
        ToolImpl::Host => match host {
            Some(host) => host.call(name, input).await,
            None => Err(format!(
                "tool \"{}\" needs the desktop app (no host available)",
                name
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_has_every_tool_once_with_an_object_schema() {
        let t = table();
        assert_eq!(t.len(), 26, "the table lost or gained a tool");
        let names: std::collections::HashSet<_> = t.iter().map(|e| e.name).collect();
        assert_eq!(names.len(), t.len(), "duplicate tool name");
        for e in t {
            assert_eq!(e.input_schema["type"], "object", "{}", e.name);
            assert!(!e.description.is_empty(), "{}", e.name);
            let props = e.input_schema["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("{} has no properties", e.name));
            let required = e.input_schema["required"]
                .as_array()
                .unwrap_or_else(|| panic!("{} has no required", e.name));
            for r in required {
                let key = r.as_str().unwrap_or_default();
                assert!(props.contains_key(key), "{}: required {:?}", e.name, key);
            }
        }
    }

    #[test]
    fn only_the_app_bound_tools_need_a_host() {
        let host: Vec<_> = table()
            .iter()
            .filter(|e| e.needs_host())
            .map(|e| e.name)
            .collect();
        assert_eq!(
            host,
            vec![
                "help_connection_check",
                "help_diagnostic_run",
                "help_docs_search",
                "help_docs_read",
                "help_setup_inspect",
                "help_agent_plan",
                "help_agent_apply",
                "agent_delegate",
                "download_url",
                "download_enqueue",
                "downloads_queue",
                "download_status",
                "download_cancel",
                "download_pause",
                "download_resume",
            ]
        );
    }

    #[test]
    fn specs_mirror_the_table_in_order() {
        let specs = specs();
        assert_eq!(specs.len(), table().len());
        for (spec, e) in specs.iter().zip(table()) {
            assert_eq!(spec.name, e.name);
            assert_eq!(spec.input_schema, e.input_schema);
        }
    }

    #[test]
    fn the_queue_filter_offers_exactly_the_status_keys() {
        let q = find("downloads_queue").expect("downloads_queue");
        assert_eq!(
            q.input_schema["properties"]["status"]["enum"],
            json!(QUEUE_STATUS_KEYS)
        );
        assert_eq!(q.input_schema["required"], json!([]));
    }

    #[tokio::test]
    async fn an_unknown_tool_says_so() {
        let e = dispatch("nope", json!({}), None).await.unwrap_err();
        assert_eq!(e, "unknown tool: nope");
    }

    #[tokio::test]
    async fn a_host_tool_without_a_host_fails_instead_of_pretending() {
        let e = dispatch("download_pause", json!({ "download_id": 1 }), None)
            .await
            .unwrap_err();
        assert!(e.contains("download_pause") && e.contains("host"), "{}", e);
    }

    #[tokio::test]
    async fn a_host_tool_reaches_the_host_with_its_own_name() {
        struct Spy;
        #[async_trait]
        impl HostTools for Spy {
            async fn call(&self, name: &str, input: Value) -> Result<Value, String> {
                Ok(json!({ "name": name, "input": input }))
            }
        }
        let out = dispatch("download_resume", json!({ "download_id": 7 }), Some(&Spy))
            .await
            .unwrap();
        assert_eq!(out["name"], "download_resume");
        assert_eq!(out["input"]["download_id"], 7);
    }

    #[tokio::test]
    async fn a_core_tool_runs_without_the_app() {
        // `todo_write` only records the plan: no network, no app handle.
        let plan = json!([{ "step": "a", "status": "pending" }]);
        let out = dispatch("todo_write", json!({ "plan": plan }), None)
            .await
            .unwrap();
        assert_eq!(out["plan"], plan);
    }

    #[test]
    fn arguments_fail_with_a_readable_message() {
        let a = json!({ "download_id": "42", "url": "  " });
        assert_eq!(need_id(&a, "download_id"), Ok(42));
        let Err(e) = need_id(&json!({}), "download_id") else {
            panic!("a missing id should fail");
        };
        assert!(e.contains("download_id") && e.contains("integer"), "{}", e);
        let Err(e) = need_str(&a, "url") else {
            panic!("a blank url should fail");
        };
        assert!(e.contains("url") && e.contains("required"), "{}", e);
    }
}
