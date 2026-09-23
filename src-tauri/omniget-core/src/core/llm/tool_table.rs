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

use crate::core::tools::{
    self as tools, ai_keys, disk, dupes, edge_tts, file_search, humanize, image_resize, ocr, pdf,
    pricing, ryd, sponsorblock, startup, sysclean, uninstall, whisper, x,
};

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
    /// The Tools section category this belongs to (`catalog.ts` category id).
    pub category: &'static str,
    pub cost_hint: CostHint,
    /// The `catalog.ts` entry the UI can link to, when the tool has a page.
    pub catalog_id: Option<&'static str>,
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

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn or_default(v: &Value, k: &str, default: &str) -> String {
    let value = s(v, k);
    if value.is_empty() {
        default.to_string()
    } else {
        value
    }
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
    catalog_id: Option<&'static str>,
) -> ToolEntry {
    ToolEntry {
        name,
        description,
        input_schema,
        call,
        category,
        cost_hint,
        catalog_id,
    }
}

fn core(f: CoreCall) -> ToolImpl {
    ToolImpl::Core(f)
}

/// Every internal tool. Built once; call [`table`] to read it.
fn build() -> Vec<ToolEntry> {
    use CostHint::{Cheap, Local, Network, Paid};
    vec![
        entry("help_connection_check", "Check local installation evidence without login or paid requests. Unknown authentication is not success.", obj(json!({"connectionId":{"type":"string"}}), &["connectionId"]), ToolImpl::Host, "help", Local, None),
        entry("help_diagnostic_run", "Run a selected, allowlisted local check. Never runs a paid model probe.", obj(json!({"checkId":{"type":"string","enum":["connection","documentation"]},"targetId":{"type":"string"},"locale":{"type":"string"}}), &["checkId"]), ToolImpl::Host, "help", Local, None),
        entry("help_docs_search", "Search bundled OmniGet help. Returns versioned citation sources.", obj(json!({"query":{"type":"string"},"locale":{"type":"string"},"limit":{"type":"integer","maximum":8}}), &["query"]), ToolImpl::Host, "help", Cheap, None),
        entry("help_docs_read", "Read a bundled OmniGet help article.", obj(json!({"articleId":{"type":"string"},"locale":{"type":"string"}}), &["articleId"]), ToolImpl::Host, "help", Cheap, None),
        entry("help_setup_inspect", "Inspect sanitized agent connections without secrets or network calls.", obj(json!({}), &[]), ToolImpl::Host, "help", Cheap, None),
        entry("help_agent_plan", "Plan a new agent or edit an existing agent name/connection. New agents have no tools; edits preserve permissions. Does not save.", obj(json!({"sourceAgentId":{"type":"string"},"name":{"type":"string"},"agentId":{"type":"string"}}), &["sourceAgentId", "name"]), ToolImpl::Host, "help", Cheap, None),
        entry("help_agent_apply", "Apply a previously reviewed agent plan idempotently. Requires the exact revision.", obj(json!({"planId":{"type":"string"},"expectedRevision":{"type":"string"},"idempotencyKey":{"type":"string"}}), &["planId", "expectedRevision", "idempotencyKey"]), ToolImpl::Host, "help", Cheap, None),
        // ── Coding harness (core/llm/code_tools.rs): confined to the workspace ──
        entry("fs_read", "Read a text file of the workspace with line numbers. A directory path lists it. Use offset/limit for long files.",
            obj(json!({ "path": { "type": "string" }, "offset": { "type": "integer", "minimum": 1 }, "limit": { "type": "integer", "minimum": 1 } }), &["path"]),
            core(|a| Box::pin(async move { super::code_tools::fs_read(&a) })), "code", Local, None),
        entry("fs_list", "List files and folders of the workspace (skips .git, node_modules, target).",
            obj(json!({ "path": { "type": "string" }, "depth": { "type": "integer", "minimum": 1, "maximum": 8 } }), &[]),
            core(|a| Box::pin(async move { super::code_tools::fs_list(&a) })), "code", Local, None),
        entry("fs_glob", "Find files by glob pattern (e.g. **/*.rs, src/**/*.{ts,svelte}).",
            obj(json!({ "pattern": { "type": "string" }, "path": { "type": "string" } }), &["pattern"]),
            core(|a| Box::pin(async move { super::code_tools::fs_glob(&a) })), "code", Local, None),
        entry("fs_grep", "Search file contents by regex; returns path:line: text.",
            obj(json!({ "pattern": { "type": "string" }, "path": { "type": "string" }, "include": { "type": "string" }, "literal_text": { "type": "boolean" } }), &["pattern"]),
            core(|a| Box::pin(async move { super::code_tools::fs_grep(&a) })), "code", Local, None),
        entry("fs_edit", "Replace old_string with new_string in a file. old_string must match exactly one place unless replace_all is true. Empty old_string creates a new file.",
            obj(json!({ "path": { "type": "string" }, "old_string": { "type": "string" }, "new_string": { "type": "string" }, "replace_all": { "type": "boolean" } }), &["path", "old_string", "new_string"]),
            core(|a| Box::pin(async move { super::code_tools::fs_edit(&a) })), "code", Local, None),
        entry("fs_write", "Create or overwrite a whole file.",
            obj(json!({ "path": { "type": "string" }, "content": { "type": "string" } }), &["path", "content"]),
            core(|a| Box::pin(async move { super::code_tools::fs_write(&a) })), "code", Local, None),
        entry("fs_apply_patch", "Apply a multi-file patch in the '*** Begin Patch' envelope (*** Add File / *** Update File with @@ hunks of ' ', '-', '+' lines / *** Delete File / *** End Patch).",
            obj(json!({ "patch": { "type": "string" } }), &["patch"]),
            core(|a| Box::pin(async move { super::code_tools::fs_apply_patch(&a) })), "code", Local, None),
        entry("shell_exec", "Run a shell command inside the workspace. Sandboxed on macOS: writes only inside the workspace, no network. Runs at the workspace root; paths are relative to it and workdir (optional) is a folder relative to it. Say what the command does in description.",
            obj(json!({ "command": { "type": "string" }, "description": { "type": "string" }, "workdir": { "type": "string" }, "timeout_ms": { "type": "integer", "minimum": 1000 } }), &["command", "description"]),
            core(|a| Box::pin(async move { super::code_tools::shell_exec(a).await })), "code", Local, None),
        entry("todo_write", "Publish or update the plan for the task: a list of steps with status pending, in_progress or completed.",
            obj(json!({ "plan": { "type": "array", "items": { "type": "object", "properties": { "step": { "type": "string" }, "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] } }, "required": ["step", "status"] } }, "explanation": { "type": "string" } }), &["plan"]),
            core(|a| Box::pin(async move { super::code_tools::todo_write(&a) })), "code", Local, None),
        entry("kb_search", "Search the project knowledge base (.omniget/kb, markdown notes written by the team of agents): decisions, conventions, how things are run, what was already tried. Search before you explore a codebase you were in before.",
            obj(json!({ "query": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 20 } }), &["query"]),
            core(|a| Box::pin(async move { super::kb::kb_search(a).await })), "code", Cheap, None),
        entry("kb_write", "Save a note to the project knowledge base so the next agent (or you, tomorrow) does not rediscover it: a decision and its reason, a command that works, a trap. One topic per note; `append` adds to an existing note with the same title.",
            obj(json!({ "title": { "type": "string" }, "content": { "type": "string", "description": "markdown" }, "append": { "type": "boolean" } }), &["title", "content"]),
            core(|a| Box::pin(async move { super::kb::kb_write(&a) })), "code", Cheap, None),
        entry("agent_delegate", "Hand a self-contained sub-task to another agent of the roster and get its final answer back. The other agent works in the same workspace, with its own tools and budget. Use it for work that fits another role better or can be isolated.",
            obj(json!({ "agent_id": { "type": "string", "description": "id of the roster agent" }, "task": { "type": "string", "description": "everything the other agent needs to know; it does not see this conversation" } }), &["agent_id", "task"]),
            ToolImpl::Host, "code", Paid, None),
        entry(
            "download_url",
            "Queue a URL (video, audio, playlist, course, image) in the OmniGet Downloads panel. Same as pasting it in the app.",
            obj(json!({ "url": { "type": "string" } }), &["url"]),
            ToolImpl::Host,
            "downloads",
            Network,
            None,
        ),
        entry(
            "youtube_sponsorblock",
            "SponsorBlock segments (sponsor, intro, outro, selfpromo…) of a YouTube video.",
            obj(json!({ "url": { "type": "string" }, "categories": { "type": "array", "items": { "type": "string" } } }), &["url"]),
            core(|a| Box::pin(async move {
                to_json(sponsorblock::segments(&s(&a, "url"), &list(&a, "categories")).await.map_err(err)?)
            })),
            "youtube",
            Network,
            Some("yt-sponsorblock"),
        ),
        entry(
            "youtube_dislikes",
            "Return YouTube Dislike estimates for a video.",
            obj(json!({ "url": { "type": "string" } }), &["url"]),
            core(|a| Box::pin(async move {
                to_json(ryd::votes(&s(&a, "url")).await.map_err(err)?)
            })),
            "youtube",
            Network,
            Some("yt-dislikes"),
        ),
        entry(
            "pdf_info",
            "Pages, size, title, author and whether the PDF has a text layer.",
            obj(json!({ "path": { "type": "string" } }), &["path"]),
            core(|a| Box::pin(async move {
                let path = s(&a, "path");
                to_json(
                    tokio::task::spawn_blocking(move || pdf::info(&path, None))
                        .await
                        .map_err(err)?
                        .map_err(err)?,
                )
            })),
            "pdf",
            Cheap,
            None,
        ),
        entry(
            "pdf_merge",
            "Merge PDFs into one file, in the given order.",
            obj(json!({ "inputs": { "type": "array", "items": { "type": "string" } }, "output": { "type": "string" } }), &["inputs", "output"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                let opts = pdf::MergeOptions {
                    inputs: list(&a, "inputs"),
                    output: s(&a, "output"),
                };
                to_json(
                    tokio::task::spawn_blocking(move || pdf::merge(&opts, &p))
                        .await
                        .map_err(err)?
                        .map_err(err)?,
                )
            })),
            "pdf",
            Local,
            Some("pdf-merge"),
        ),
        entry(
            "pdf_split",
            "Split a PDF: mode each | every | ranges (\"1-3; 4-10\") | extract (\"1,3,5-7\").",
            obj(json!({ "input": { "type": "string" }, "mode": { "type": "string" }, "every": { "type": "integer" }, "ranges": { "type": "string" }, "output_dir": { "type": "string" } }), &["input", "mode"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                let opts = pdf::SplitOptions {
                    input: s(&a, "input"),
                    mode: s(&a, "mode"),
                    every: num(&a, "every").unwrap_or(0) as usize,
                    ranges: s(&a, "ranges"),
                    output_dir: s(&a, "output_dir"),
                };
                to_json(
                    tokio::task::spawn_blocking(move || pdf::split(&opts, &p))
                        .await
                        .map_err(err)?
                        .map_err(err)?,
                )
            })),
            "pdf",
            Local,
            Some("pdf-split"),
        ),
        entry(
            "pdf_text",
            "Extract the text of a PDF (optionally a page range like \"1-3, 5\").",
            obj(json!({ "path": { "type": "string" }, "pages": { "type": "string" } }), &["path"]),
            core(|a| Box::pin(async move {
                let (path, pages) = (s(&a, "path"), s(&a, "pages"));
                to_json(
                    tokio::task::spawn_blocking(move || pdf::to_text(&path, &pages, false, ""))
                        .await
                        .map_err(err)?
                        .map_err(err)?,
                )
            })),
            "pdf",
            Local,
            None,
        ),
        entry(
            "pdf_render",
            "Render PDF pages to PNG or JPG files.",
            obj(json!({ "input": { "type": "string" }, "pages": { "type": "string" }, "dpi": { "type": "integer" }, "format": { "type": "string", "enum": ["png", "jpg"] }, "output_dir": { "type": "string" } }), &["input"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                let opts = pdf::RenderOptions {
                    input: s(&a, "input"),
                    pages: s(&a, "pages"),
                    dpi: num(&a, "dpi").unwrap_or(0) as u32,
                    format: s(&a, "format"),
                    quality: 0,
                    output_dir: s(&a, "output_dir"),
                };
                to_json(
                    tokio::task::spawn_blocking(move || pdf::render(&opts, &p))
                        .await
                        .map_err(err)?
                        .map_err(err)?,
                )
            })),
            "pdf",
            Local,
            None,
        ),
        entry(
            "pdf_sanitize",
            "Rebuild a PDF from pixels (Dangerzone-style) so scripts, forms and attachments are dropped.",
            obj(json!({ "input": { "type": "string" }, "output_dir": { "type": "string" } }), &["input"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                let (input, dir) = (s(&a, "input"), s(&a, "output_dir"));
                to_json(
                    tokio::task::spawn_blocking(move || pdf::sanitize(&input, &dir, 0, 0, &p))
                        .await
                        .map_err(err)?
                        .map_err(err)?,
                )
            })),
            "pdf",
            Local,
            Some("pdf-sanitize"),
        ),
        entry(
            "tts_speak",
            "Text to speech with Microsoft Edge neural voices; writes an MP3.",
            obj(json!({ "text": { "type": "string" }, "voice": { "type": "string", "description": "e.g. pt-BR-AntonioNeural, en-US-AriaNeural" }, "output": { "type": "string" } }), &["text", "output"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                let voice = or_default(&a, "voice", "pt-BR-AntonioNeural");
                let opts: edge_tts::TtsOptions =
                    serde_json::from_value(json!({ "text": s(&a, "text"), "voice": voice }))
                        .map_err(err)?;
                to_json(
                    edge_tts::synthesize(opts, std::path::Path::new(&s(&a, "output")), p)
                        .await
                        .map_err(err)?,
                )
            })),
            "speech",
            Network,
            Some("speech-tts"),
        ),
        entry(
            "transcribe",
            "Transcribe audio or video locally with whisper.cpp; returns text and SRT path.",
            obj(json!({ "input": { "type": "string" }, "model": { "type": "string", "description": "GGML model id, default base" }, "language": { "type": "string", "description": "auto | pt | en …" } }), &["input"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                let model = or_default(&a, "model", "base");
                let language = or_default(&a, "language", "auto");
                let opts: whisper::TranscribeOptions = serde_json::from_value(
                    json!({ "input": s(&a, "input"), "model": model, "language": language }),
                )
                .map_err(err)?;
                let r = whisper::transcribe(opts, p).await.map_err(err)?;
                Ok(json!({ "language": r.language, "text": r.text, "srt": r.srt_path, "vtt": r.vtt_path, "txt": r.txt_path, "seconds": r.seconds }))
            })),
            "speech",
            Local,
            Some("speech-transcribe"),
        ),
        entry(
            "image_resize",
            "Resize images in batch. mode width | height | fit | percent.",
            obj(json!({ "inputs": { "type": "array", "items": { "type": "string" } }, "mode": { "type": "string" }, "value": { "type": "integer" }, "value2": { "type": "integer" }, "format": { "type": "string" }, "output_dir": { "type": "string" } }), &["inputs", "mode", "value"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                let opts: image_resize::ResizeOptions = serde_json::from_value(json!({ "inputs": list(&a, "inputs"), "mode": s(&a, "mode"), "value": num(&a, "value").unwrap_or(1024), "value2": num(&a, "value2").unwrap_or(0), "format": s(&a, "format"), "output_dir": s(&a, "output_dir") })).map_err(err)?;
                to_json(image_resize::run(opts, p).await.map_err(err)?)
            })),
            "images",
            Local,
            Some("img-resize"),
        ),
        entry(
            "ocr",
            "Extract text from images with Tesseract.",
            obj(json!({ "inputs": { "type": "array", "items": { "type": "string" } }, "langs": { "type": "string", "description": "por+eng" } }), &["inputs"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                to_json(ocr::run(&list(&a, "inputs"), &s(&a, "langs"), p).await.map_err(err)?)
            })),
            "images",
            Local,
            Some("img-ocr"),
        ),
        entry(
            "find_duplicates",
            "Find duplicate files (same content) under folders.",
            obj(json!({ "dirs": { "type": "array", "items": { "type": "string" } }, "min_size": { "type": "integer" } }), &["dirs"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                let opts: dupes::DupesOptions = serde_json::from_value(json!({ "dirs": list(&a, "dirs"), "min_size": num(&a, "min_size").unwrap_or(1024) })).map_err(err)?;
                to_json(
                    tokio::task::spawn_blocking(move || dupes::scan(&opts, &p))
                        .await
                        .map_err(err)?,
                )
            })),
            "files",
            Local,
            Some("files-dupes"),
        ),
        entry(
            "file_search",
            "Search files by name (Everything, Spotlight or locate/find).",
            obj(json!({ "query": { "type": "string" }, "folder": { "type": "string" }, "limit": { "type": "integer" } }), &["query"]),
            core(|a| Box::pin(async move {
                to_json(
                    file_search::search(
                        &s(&a, "query"),
                        &s(&a, "folder"),
                        num(&a, "limit").unwrap_or(100) as usize,
                    )
                    .await
                    .map_err(err)?,
                )
            })),
            "files",
            Local,
            Some("files-search"),
        ),
        entry(
            "ai_prices",
            "Search LLM prices per million tokens (LiteLLM + models.dev).",
            obj(json!({ "query": { "type": "string" }, "limit": { "type": "integer" } }), &["query"]),
            core(|a| Box::pin(async move {
                to_json(
                    pricing::search(&s(&a, "query"), "", num(&a, "limit").unwrap_or(30) as usize)
                        .await
                        .map_err(err)?,
                )
            })),
            "ai",
            Network,
            Some("ai-prices"),
        ),
        entry(
            "humanize",
            "Rewrite AI-sounding text so it reads like a person wrote it, using the app's configured AI.",
            obj(json!({ "text": { "type": "string" } }), &["text"]),
            core(|a| Box::pin(async move {
                Ok(json!({ "text": humanize::humanize(&s(&a, "text"), None).await? }))
            })),
            "ai",
            Paid,
            Some("ai-humanize"),
        ),
        entry(
            "x_post",
            "Fetch an X/Twitter post (text, author, media) by URL or id.",
            obj(json!({ "url": { "type": "string" } }), &["url"]),
            core(|a| Box::pin(async move {
                let input = s(&a, "url");
                let id = x::post_id_from(&input).ok_or_else(|| format!("not an X post: {}", input))?;
                to_json(x::fx::status(&id).await.map_err(err)?)
            })),
            "x",
            Network,
            Some("x-download"),
        ),
        entry(
            "x_thread",
            "Unroll an X/Twitter thread from any post in it.",
            obj(json!({ "url": { "type": "string" } }), &["url"]),
            core(|a| Box::pin(async move {
                to_json(x::thread::unroll(&s(&a, "url")).await.map_err(err)?)
            })),
            "x",
            Network,
            Some("x-thread"),
        ),
        entry(
            "x_profile",
            "Profile analytics for an X/Twitter user (engagement, best hours, top posts).",
            obj(json!({ "handle": { "type": "string" }, "limit": { "type": "integer" } }), &["handle"]),
            core(|a| Box::pin(async move {
                to_json(
                    x::profile::analyze(&s(&a, "handle"), num(&a, "limit").unwrap_or(100) as usize, false)
                        .await
                        .map_err(err)?,
                )
            })),
            "x",
            Network,
            Some("x-profile"),
        ),
        entry(
            "x_search",
            "Search X/Twitter posts (advanced operators supported).",
            obj(json!({ "query": { "type": "string" }, "feed": { "type": "string", "enum": ["latest", "top"] } }), &["query"]),
            core(|a| Box::pin(async move {
                let feed = or_default(&a, "feed", "latest");
                to_json(x::search::search(&s(&a, "query"), &feed, None).await.map_err(err)?)
            })),
            "x",
            Network,
            Some("x-search"),
        ),
        entry(
            "x_trends",
            "Current X/Twitter trends.",
            obj(json!({}), &[]),
            core(|_a| Box::pin(async move {
                to_json(x::search::trends().await.map_err(err)?)
            })),
            "x",
            Network,
            None,
        ),
        entry(
            "instagram_profile",
            "Public info of an Instagram profile, using the cookies captured by the OmniGet extension.",
            obj(json!({ "username": { "type": "string" }, "account": { "type": "string", "description": "cookie slot, default _default" } }), &["username"]),
            ToolImpl::Host,
            "instagram",
            Network,
            Some("ig-viewer"),
        ),
        entry(
            "gallery_download",
            "Download a gallery/profile with gallery-dl (Pinterest, ArtStation, DeviantArt, Reddit…).",
            obj(json!({ "url": { "type": "string" }, "dest": { "type": "string" } }), &["url", "dest"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                to_json(tools::gallery::download(&s(&a, "url"), &s(&a, "dest"), None, p).await.map_err(err)?)
            })),
            "documents",
            Network,
            Some("doc-gallery"),
        ),
        entry(
            "aria2_download",
            "Download a large file with aria2 (multi-connection).",
            obj(json!({ "url": { "type": "string" }, "dest_dir": { "type": "string" }, "connections": { "type": "integer" } }), &["url", "dest_dir"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                let opts: tools::aria2::Aria2Options = serde_json::from_value(json!({ "url": s(&a, "url"), "dest_dir": s(&a, "dest_dir"), "connections": num(&a, "connections").unwrap_or(16) })).map_err(err)?;
                to_json(tools::aria2::download(opts, p).await.map_err(err)?)
            })),
            "downloads",
            Network,
            Some("dl-aria2"),
        ),
        entry(
            "disk_volumes",
            "Mounted volumes with total and free space.",
            obj(json!({}), &[]),
            core(|_a| Box::pin(async move { to_json(disk::volumes()) })),
            "system",
            Cheap,
            Some("sys-disk"),
        ),
        entry(
            "disk_scan",
            "Folder sizes tree and largest files under a path.",
            obj(json!({ "path": { "type": "string" }, "depth": { "type": "integer" } }), &["path"]),
            core(|a| Box::pin(async move {
                let p = tools::noop_progress();
                let (path, depth) = (s(&a, "path"), num(&a, "depth").unwrap_or(2) as usize);
                to_json(
                    tokio::task::spawn_blocking(move || disk::scan(&path, depth, 25, &p))
                        .await
                        .map_err(err)?
                        .map_err(err)?,
                )
            })),
            "system",
            Local,
            Some("sys-disk"),
        ),
        entry(
            "clean_scan",
            "What the cache cleaner would remove (rule, size, files). Does not delete anything.",
            obj(json!({}), &[]),
            core(|_a| Box::pin(async move {
                let p = tools::noop_progress();
                to_json(
                    tokio::task::spawn_blocking(move || sysclean::scan(&p))
                        .await
                        .map_err(err)?,
                )
            })),
            "system",
            Local,
            Some("sys-clean"),
        ),
        entry(
            "startup_items",
            "Programs that start with the system.",
            obj(json!({}), &[]),
            core(|_a| Box::pin(async move { to_json(startup::list().await) })),
            "system",
            Cheap,
            Some("sys-startup"),
        ),
        entry(
            "installed_apps",
            "Installed applications with version and size.",
            obj(json!({}), &[]),
            core(|_a| Box::pin(async move {
                let p = tools::noop_progress();
                to_json(uninstall::list(p).await)
            })),
            "system",
            Local,
            Some("sys-uninstall"),
        ),
        entry(
            "ai_keys",
            "Saved AI API accounts (names, providers, balances). Keys are never returned.",
            obj(json!({}), &[]),
            core(|_a| Box::pin(async move { to_json(ai_keys::list()) })),
            "ai",
            Cheap,
            Some("ai-keys"),
        ),
        entry(
            "download_enqueue",
            "Queue a URL in the Downloads panel using the app defaults, and return the queue item. mode audio downloads audio only. Always supply a stable idempotencyKey per user intent; reuse it on retries.",
            obj(json!({ "url": { "type": "string" }, "mode": { "type": "string", "enum": ["video", "audio"] }, "idempotencyKey": { "type": "string" } }), &["url"]),
            ToolImpl::Host,
            "downloads",
            Network,
            None,
        ),
        entry(
            "downloads_queue",
            "List the Downloads queue with per-item status, progress, speed, ETA and output path.",
            obj(json!({ "status": { "type": "string", "enum": QUEUE_STATUS_KEYS }, "limit": { "type": "integer" } }), &[]),
            ToolImpl::Host,
            "downloads",
            Cheap,
            None,
        ),
        entry(
            "download_status",
            "One download by id: status, percent, speed, ETA, file path and the last yt-dlp command.",
            obj(json!({ "download_id": { "type": "integer" } }), &["download_id"]),
            ToolImpl::Host,
            "downloads",
            Cheap,
            None,
        ),
        entry(
            "download_cancel",
            "Cancel a download by id (queued, active, paused or seeding).",
            obj(json!({ "download_id": { "type": "integer" } }), &["download_id"]),
            ToolImpl::Host,
            "downloads",
            Cheap,
            None,
        ),
        entry(
            "download_pause",
            "Pause an active download by id.",
            obj(json!({ "download_id": { "type": "integer" } }), &["download_id"]),
            ToolImpl::Host,
            "downloads",
            Cheap,
            None,
        ),
        entry(
            "download_resume",
            "Resume a paused download by id.",
            obj(json!({ "download_id": { "type": "integer" } }), &["download_id"]),
            ToolImpl::Host,
            "downloads",
            Cheap,
            None,
        ),
        // ── Catalog (Central): an agent can equip itself, the owner approves ──
        entry(
            "catalog_search",
            "Search the OmniGet catalog of agent components (agents, commands, skills, MCP servers, hooks, settings, statuslines, plugins) that can be installed into Claude Code, Codex, Gemini CLI, Cursor, OpenCode and other coding tools. Returns ids for catalog_get / catalog_plan_install.",
            obj(json!({ "query": { "type": "string" }, "kinds": { "type": "array", "items": { "type": "string", "enum": ["agent", "command", "skill", "mcp", "hook", "setting", "statusline", "plugin", "rule", "loop", "mod", "template", "sandbox"] } }, "categories": { "type": "array", "items": { "type": "string" } }, "limit": { "type": "integer", "minimum": 1, "maximum": 50 }, "offset": { "type": "integer", "minimum": 0 } }), &[]),
            ToolImpl::Host,
            "catalog",
            Cheap,
            None,
        ),
        entry(
            "catalog_get",
            "One catalog item by id: description, files, license, source, and on which installed coding tools it works (and what is lost in the conversion).",
            obj(json!({ "id": { "type": "string" }, "project": { "type": "string", "description": "absolute project folder, for project-scoped compatibility" } }), &["id"]),
            ToolImpl::Host,
            "catalog",
            Cheap,
            None,
        ),
        entry(
            "catalog_plan_install",
            "Plan installing catalog items: every file each target tool would get, with a unified diff, conflicts and the commands the install makes a tool run. Writes nothing. Returns a plan_id for catalog_install.",
            obj(json!({ "ids": { "type": "array", "items": { "type": "string" }, "minItems": 1 }, "targets": { "type": "array", "items": { "type": "string" }, "description": "tool ids (claude, codex, gemini, cursor, opencode…); empty = the installed tools enabled by default" }, "scope": { "type": "string", "enum": ["project", "global", "local"] }, "project": { "type": "string", "description": "absolute project folder (required for scope project)" }, "policy": { "type": "string", "enum": ["rename", "skip", "overwrite"] } }), &["ids"]),
            ToolImpl::Host,
            "catalog",
            Network,
            None,
        ),
        entry(
            "catalog_install",
            "Apply a plan from catalog_plan_install. The owner of this machine is asked first and sees the whole diff; a refusal or no answer fails the call and writes nothing.",
            obj(json!({ "plan_id": { "type": "string" }, "reason": { "type": "string", "description": "one sentence for the owner: why you need this" } }), &["plan_id"]),
            ToolImpl::Host,
            "catalog",
            Local,
            None,
        ),
        // ── Preview: the thread Browser tab (host body: crate::preview::tools) ──
        entry("preview_servers", "List dev servers listening on local TCP ports from inside a folder (lsof/netstat + HTTP probe). Returns port, url, pid, command, cwd, inWorkspace, html, title. Without cwd, every local listener.", obj(json!({"cwd":{"type":"string","description":"Project or worktree folder"},"all":{"type":"boolean","description":"Also list local listeners outside the folder"}}), &[]), ToolImpl::Host, "code", Cheap, None),
        entry("preview_navigate", "Open a URL in the thread's Browser preview (opens a hidden preview when none is open) or go back/forward/reload. Waits for the page load and returns {url, title, loading, visible}.", obj(json!({"url":{"type":"string"},"action":{"type":"string","enum":["back","forward","reload"]},"threadId":{"type":"string"}}), &[]), ToolImpl::Host, "code", Local, None),
        entry("preview_snapshot", "Accessibility-style snapshot of the preview page: an indented role/name tree whose refs (e12) work as selectors in preview_click/preview_type, plus the visible text (and the HTML with includeHtml).", obj(json!({"threadId":{"type":"string"},"maxNodes":{"type":"integer","maximum":10000},"maxChars":{"type":"integer","maximum":200000},"includeHtml":{"type":"boolean"}}), &[]), ToolImpl::Host, "code", Cheap, None),
        entry("preview_click", "Click an element of the preview page. selector: a CSS selector, a ref from preview_snapshot (e12), or text=Label.", obj(json!({"selector":{"type":"string"},"double":{"type":"boolean"},"threadId":{"type":"string"}}), &["selector"]), ToolImpl::Host, "code", Local, None),
        entry("preview_type", "Type into an input, textarea, select or contenteditable of the preview page (React-safe value setter + input/change events). clear defaults to true; submit presses Enter and submits the form.", obj(json!({"selector":{"type":"string"},"text":{"type":"string"},"submit":{"type":"boolean"},"clear":{"type":"boolean"},"threadId":{"type":"string"}}), &["selector", "text"]), ToolImpl::Host, "code", Local, None),
        entry("preview_console", "Console messages, page errors and failed resource loads captured in the preview since the last page load (drained by default).", obj(json!({"level":{"type":"string","enum":["error","warn","log","info","debug"]},"limit":{"type":"integer","maximum":500},"clear":{"type":"boolean"},"threadId":{"type":"string"}}), &[]), ToolImpl::Host, "code", Cheap, None),
        entry("preview_screenshot", "PNG screenshot of the preview page; returns {path, width, height}. Shows the preview for a moment when it is hidden.", obj(json!({"threadId":{"type":"string"}}), &[]), ToolImpl::Host, "code", Local, None),
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
        assert_eq!(t.len(), 67, "the table lost or gained a tool");
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
                "instagram_profile",
                "download_enqueue",
                "downloads_queue",
                "download_status",
                "download_cancel",
                "download_pause",
                "download_resume",
                "catalog_search",
                "catalog_get",
                "catalog_plan_install",
                "catalog_install",
                "preview_servers",
                "preview_navigate",
                "preview_snapshot",
                "preview_click",
                "preview_type",
                "preview_console",
                "preview_screenshot",
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
    fn every_catalog_id_looks_like_a_catalog_entry() {
        // `catalog.ts` ids are `<category-prefix>-<slug>`; a typo like
        // `pdf_merge` would silently break the UI link.
        for e in table() {
            let Some(id) = e.catalog_id else { continue };
            assert!(
                id.contains('-') && !id.contains('_') && id == id.to_lowercase(),
                "{}: bad catalog id {:?}",
                e.name,
                id
            );
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
        // `disk_volumes` is local and read-only: no network, no app handle.
        let out = dispatch("disk_volumes", json!({}), None).await.unwrap();
        assert!(out.is_array(), "{}", out);
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
