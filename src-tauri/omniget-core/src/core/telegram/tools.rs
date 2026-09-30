//! The `tg_*` MCP tools: names, schemas and bodies. The table in
//! `core::llm::tool_table` appends [`entries`]; the app's external MCP port
//! goes through [`call_as`] with the client's name. Every body ends in the
//! same engine the `/telegram` screen uses (`commands::dispatch` or the
//! client's functions), so an agent and the screen never diverge.
//!
//! Tools that write to the person's account (`tg_send_message`,
//! `tg_send_files`, `tg_forward`, `tg_delete_messages`) always ask the person
//! first through [`super::approvals`], unless the caller is the screen.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use grammers_client::grammers_tl_types as tl;
use serde::Serialize;
use serde_json::{json, Value};

use super::approvals;
use super::client::{self, api, auth::TelegramSessionHandle, history};
use crate::core::llm::tool_table::{BoxFut, CostHint, ToolEntry, ToolImpl, ToolRisk};

/// Who is calling. Only the screen writes without an approval.
#[derive(Debug, Clone)]
pub enum Caller {
    Screen,
    Agent,
    /// An external MCP client, by the name the person gave its grant.
    External(String),
    Cli,
}

impl Caller {
    fn label(&self) -> String {
        match self {
            Caller::Screen => "screen".into(),
            Caller::Agent => "agent".into(),
            Caller::External(name) => name.clone(),
            Caller::Cli => "cli".into(),
        }
    }
}

// ── Table rows ─────────────────────────────────────────────────────────

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

struct Def {
    name: &'static str,
    description: &'static str,
    schema: Value,
    call: fn(Value) -> BoxFut,
    cost: CostHint,
    risk: ToolRisk,
}

macro_rules! body {
    ($name:literal) => {
        |a: Value| -> BoxFut { Box::pin(async move { call($name, a).await }) }
    };
}

const CHAT_DOC: &str = "Chat: numeric id (Bot API -100... ids work too), @username, a t.me link, or the exact chat title.";

fn chat_prop() -> Value {
    json!({ "type": ["string", "integer"], "description": CHAT_DOC })
}

fn ids_prop() -> Value {
    json!({ "type": "array", "items": { "type": ["integer", "string"] }, "minItems": 1, "maxItems": 100, "description": "Message ids (from tg_read_messages, tg_search or tg_list_media)" })
}

fn media_types_prop() -> Value {
    json!({ "type": "array", "items": { "type": "string", "enum": history::MEDIA_TYPES } })
}

fn date_prop(what: &str) -> Value {
    json!({ "type": "integer", "description": format!("{what}, unix seconds") })
}

fn defs() -> Vec<Def> {
    use CostHint::{Cheap, Local, Network};
    use ToolRisk::{Destructive, Read, Reversible};
    let d = |name, description, schema, call, cost, risk| Def {
        name,
        description,
        schema,
        call,
        cost,
        risk,
    };
    let limit = |max: u64| json!({ "type": "integer", "minimum": 1, "maximum": max });
    vec![
        // ── Read ──────────────────────────────────────────────────────
        d("tg_status", "Telegram status in OmniGet: whether an account is logged in (and which), the connection phase and the active network route. Call it first; every other tg_* tool needs a logged-in account.",
            obj(json!({}), &[]), body!("tg_status"), Cheap, Read),
        d("tg_list_chats", "The person's Telegram chats (private, groups, channels), most recent first, with id, title, type, unread count and the last message. query filters by title.",
            obj(json!({ "query": { "type": "string" }, "limit": limit(500) }), &[]), body!("tg_list_chats"), Network, Read),
        d("tg_read_messages", "Read messages of one chat, oldest to newest in the returned page: id, date, author, text, a short media summary and the links in the text. Pages go back in time: pass nextOffsetId as offsetId for the older page. fromDate/toDate bound the page.",
            obj(json!({ "chat": chat_prop(), "limit": limit(500), "offsetId": { "type": "integer", "description": "Return messages older than this id" }, "fromDate": date_prop("Oldest message date"), "toDate": date_prop("Newest message date") }), &["chat"]), body!("tg_read_messages"), Network, Read),
        d("tg_search", "Search message text. With chat, inside that chat; without it, across every chat of the account. Each hit has the chat, the message id, date, author and text.",
            obj(json!({ "query": { "type": "string", "minLength": 1 }, "chat": chat_prop(), "limit": limit(100) }), &["query"]), body!("tg_search"), Network, Read),
        d("tg_chat_info", "One chat's details: id, type, title, unread count, muted/pinned, last message, and for groups and channels the description, public username and member count.",
            obj(json!({ "chat": chat_prop() }), &["chat"]), body!("tg_chat_info"), Network, Read),
        d("tg_list_media", "Files in a chat (videos, photos, documents, audio...), newest first, with message id, file name, size, type, date and caption. query searches captions and file names.",
            obj(json!({ "chat": chat_prop(), "mediaType": { "type": "string", "enum": ["video", "photo", "document", "audio", "gif", "round_video", "round_voice", "photo_video", "webpage"] }, "limit": limit(200), "query": { "type": "string" } }), &["chat"]), body!("tg_list_media"), Network, Read),
        d("tg_resolve_links", "Resolve t.me message links in a text (t.me/name/123, t.me/c/id/123, ranges like /10-20, albums) to the files they point at: chat, message id, file name, size and type. Downloads nothing.",
            obj(json!({ "text": { "type": "string", "minLength": 1 } }), &["text"]), body!("tg_resolve_links"), Network, Read),
        d("tg_stream_url", "A local http URL that streams one message's video or audio (Range supported) for a player on this computer, with mime type, size and duration. The URL only works on this machine.",
            obj(json!({ "chat": chat_prop(), "messageId": { "type": "integer", "minimum": 1 } }), &["chat", "messageId"]), body!("tg_stream_url"), Cheap, Read),
        d("tg_download_status", "Telegram downloads running right now (id and title).",
            obj(json!({}), &[]), body!("tg_download_status"), Cheap, Read),
        d("tg_watch_list", "Auto-save rules: chats whose new files OmniGet saves on its own, with filters, output folder, last message seen and how many files each saved.",
            obj(json!({}), &[]), body!("tg_watch_list"), Cheap, Read),
        d("tg_digest", "Compact text of what was said in the last hours, grouped by chat, ready to summarize: one line per message with id, time, author, text, media and links. Without chats, takes the chats with activity in the period (unread first).",
            obj(json!({ "chats": { "type": "array", "items": chat_prop(), "maxItems": 30 }, "sinceHours": { "type": "integer", "minimum": 1, "maximum": 720 }, "maxMessages": { "type": "integer", "minimum": 10, "maximum": 3000 } }), &[]), body!("tg_digest"), Network, Read),
        d("tg_export", "Export a chat's messages as compact markdown or json returned in the result (not a file), oldest first, cut to fit in context. For a file export with media use the /telegram screen.",
            obj(json!({ "chat": chat_prop(), "format": { "type": "string", "enum": ["markdown", "json"] }, "limit": limit(3000), "fromDate": date_prop("Oldest message date"), "toDate": date_prop("Newest message date") }), &["chat"]), body!("tg_export"), Network, Read),
        // ── Local actions ─────────────────────────────────────────────
        d("tg_download", "Download Telegram files to this computer. Either links (t.me message links) or chat plus filters (mediaTypes, dates, sizes, keyword, extensions; limit defaults to 200 files). Resumes interrupted files. template names files with {chat} {chat_id} {msg_id} {date} {time} {file_name} {stem} {ext} {type} {caption} {album}; slashes make folders.",
            obj(json!({
                "links": { "type": ["string", "array"], "items": { "type": "string" } },
                "chat": chat_prop(),
                "mediaTypes": media_types_prop(),
                "fromDate": date_prop("Oldest message date"), "toDate": date_prop("Newest message date"),
                "minSize": { "type": "integer", "minimum": 0 }, "maxSize": { "type": "integer", "minimum": 0 },
                "keyword": { "type": "string" },
                "extInclude": { "type": "array", "items": { "type": "string" } },
                "extExclude": { "type": "array", "items": { "type": "string" } },
                "limit": limit(5000),
                "outputDir": { "type": "string" }, "template": { "type": "string" },
            }), &[]), body!("tg_download"), Local, Reversible),
        d("tg_watch_add", "Create an auto-save rule: from now on, new files in the chat that pass the filters are saved on their own. backfill also saves the n newest matching files now.",
            obj(json!({
                "chat": chat_prop(),
                "mediaTypes": media_types_prop(),
                "minSize": { "type": "integer", "minimum": 0 }, "maxSize": { "type": "integer", "minimum": 0 },
                "keyword": { "type": "string" },
                "extInclude": { "type": "array", "items": { "type": "string" } },
                "extExclude": { "type": "array", "items": { "type": "string" } },
                "outputDir": { "type": "string" }, "template": { "type": "string" },
                "backfill": { "type": "integer", "minimum": 0, "maximum": 1000 },
            }), &["chat"]), body!("tg_watch_add"), Local, Reversible),
        d("tg_watch_update", "Change an auto-save rule. patch holds the fields to change (enabled, mediaTypes, minSize, maxSize, keyword, extInclude, extExclude, outputDir, template).",
            obj(json!({ "id": { "type": "string" }, "patch": { "type": "object" } }), &["id", "patch"]), body!("tg_watch_update"), Cheap, Reversible),
        d("tg_watch_remove", "Delete an auto-save rule. Files already saved stay.",
            obj(json!({ "id": { "type": "string" } }), &["id"]), body!("tg_watch_remove"), Cheap, Reversible),
        d("tg_watch_run", "Run an auto-save rule now (all enabled rules without id). Returns how many new files were found and queued.",
            obj(json!({ "id": { "type": "string" } }), &[]), body!("tg_watch_run"), Local, Reversible),
        // ── Writes to the account (always asks the person) ────────────
        d("tg_send_message", "Send a text message from the person's account. Always asks the person in the OmniGet window first and waits up to 5 minutes; a refusal fails the call.",
            obj(json!({ "chat": chat_prop(), "text": { "type": "string", "minLength": 1, "maxLength": 4096 } }), &["chat", "text"]), body!("tg_send_message"), Network, Destructive),
        d("tg_send_files", "Send local files or folders (recursive) from the person's account; photos and videos are grouped in albums. Always asks the person in the OmniGet window first.",
            obj(json!({ "chat": chat_prop(), "paths": { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": 50 }, "caption": { "type": "string", "maxLength": 1024 }, "asDocument": { "type": "boolean" } }), &["chat", "paths"]), body!("tg_send_files"), Network, Destructive),
        d("tg_forward", "Forward messages from one chat to another from the person's account. Always asks the person in the OmniGet window first.",
            obj(json!({ "fromChat": chat_prop(), "toChat": chat_prop(), "messageIds": ids_prop() }), &["fromChat", "toChat", "messageIds"]), body!("tg_forward"), Network, Destructive),
        d("tg_delete_messages", "Delete messages (for everyone when allowed). Cannot be undone. Always asks the person in the OmniGet window first.",
            obj(json!({ "chat": chat_prop(), "messageIds": ids_prop() }), &["chat", "messageIds"]), body!("tg_delete_messages"), Network, Destructive),
    ]
}

/// The `tg_*` rows of the tool table.
pub fn entries() -> Vec<ToolEntry> {
    defs()
        .into_iter()
        .map(|d| ToolEntry {
            name: d.name,
            description: d.description,
            input_schema: d.schema,
            call: ToolImpl::Core(d.call),
            category: "telegram",
            cost_hint: d.cost,
            platforms: &[],
            feature: Some("telegram"),
            risk: Some(d.risk),
            // The body asks the person itself (with the chat and the text),
            // so the broker's generic prompt would only ask twice.
            always_ask: false,
        })
        .collect()
}

/// Every `tg_*` name.
pub fn all_names() -> Vec<&'static str> {
    defs().into_iter().map(|d| d.name).collect()
}

/// Tools that write to the person's account and always need an approval.
pub const WRITE_TOOLS: &[&str] = &[
    "tg_send_message",
    "tg_send_files",
    "tg_forward",
    "tg_delete_messages",
];

/// Tools that change local state (downloads, auto-save rules).
pub const LOCAL_TOOLS: &[&str] = &[
    "tg_download",
    "tg_watch_add",
    "tg_watch_update",
    "tg_watch_remove",
    "tg_watch_run",
];

/// Run one `tg_*` tool as an agent of the /llm broker.
pub async fn call(name: &str, a: Value) -> Result<Value, String> {
    call_as(Caller::Agent, name, a).await
}

/// Run one `tg_*` tool for a known caller. The work runs on its own task,
/// boxed, so the large grammers futures never sit on the caller's stack.
pub async fn call_as(caller: Caller, name: &str, a: Value) -> Result<Value, String> {
    let name = name.to_string();
    let a = history::camelize(if a.is_object() { &a } else { &Value::Null });
    let a = if a.is_object() { a } else { json!({}) };
    let label = name.clone();
    tokio::spawn(Box::pin(async move { run(caller, &name, a).await }))
        .await
        .unwrap_or_else(|e| Err(format!("{label} failed unexpectedly: {e}")))
}

async fn run(caller: Caller, name: &str, a: Value) -> Result<Value, String> {
    match name {
        "tg_status" => status().await,
        "tg_list_chats" => list_chats(&a).await,
        "tg_read_messages" => read_messages(&a).await,
        "tg_search" => search(&a).await,
        "tg_chat_info" => chat_info(&a).await,
        "tg_list_media" => list_media(&a).await,
        "tg_resolve_links" => {
            let text = need_str(&a, "text")?;
            engine_call("telegram_resolve_links", json!({ "text": text })).await
        }
        "tg_stream_url" => {
            let t = target_arg(&a, "chat").await?;
            let mid = need_i64(&a, "messageId")?;
            engine_call(
                "telegram_stream_url",
                json!({ "chatId": t.id, "chatType": t.kind, "messageId": mid }),
            )
            .await
        }
        "tg_download_status" => download_status().await,
        "tg_watch_list" => wrap(
            "rules",
            engine_call("telegram_watch_list", json!({})).await?,
        ),
        "tg_digest" => digest(&a).await,
        "tg_export" => export(&a).await,
        "tg_download" => download(&a).await,
        "tg_watch_add" => watch_add(&a).await,
        "tg_watch_update" => {
            let id = need_str(&a, "id")?;
            let patch = a.get("patch").cloned().unwrap_or_else(|| json!({}));
            wrap(
                "rule",
                engine_call("telegram_watch_update", json!({ "id": id, "patch": patch })).await?,
            )
        }
        "tg_watch_remove" => {
            let id = need_str(&a, "id")?;
            engine_call("telegram_watch_remove", json!({ "id": id })).await
        }
        "tg_watch_run" => {
            let args = match opt_str(&a, "id") {
                Some(id) => json!({ "id": id }),
                None => json!({}),
            };
            wrap("result", engine_call("telegram_watch_run", args).await?)
        }
        "tg_send_message" => send_message(&caller, &a).await,
        "tg_send_files" => send_files(&caller, &a).await,
        "tg_forward" => forward(&caller, &a).await,
        "tg_delete_messages" => delete_messages(&caller, &a).await,
        _ => Err(format!("unknown tool: {name}")),
    }
}

// ── Arguments ──────────────────────────────────────────────────────────

fn opt_str(a: &Value, k: &str) -> Option<String> {
    a.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn need_str(a: &Value, k: &str) -> Result<String, String> {
    opt_str(a, k).ok_or_else(|| format!("INVALID_ARGUMENTS: {k} is required"))
}

fn opt_i64(a: &Value, k: &str) -> Option<i64> {
    match a.get(k)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn need_i64(a: &Value, k: &str) -> Result<i64, String> {
    opt_i64(a, k).ok_or_else(|| format!("INVALID_ARGUMENTS: {k} must be an integer"))
}

fn limit_arg(a: &Value, default: usize, max: usize) -> usize {
    opt_i64(a, "limit")
        .filter(|v| *v > 0)
        .map(|v| (v as usize).min(max))
        .unwrap_or(default)
}

fn ids_arg(a: &Value, k: &str) -> Result<Vec<i32>, String> {
    let list = a
        .get(k)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("INVALID_ARGUMENTS: {k} must be a list of message ids"))?;
    let mut out = Vec::new();
    for v in list {
        let id = match v {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.trim().trim_start_matches('#').parse().ok(),
            _ => None,
        }
        .filter(|id| *id > 0 && *id <= i32::MAX as i64)
        .ok_or_else(|| format!("INVALID_ARGUMENTS: {v} is not a message id"))?;
        if !out.contains(&(id as i32)) {
            out.push(id as i32);
        }
    }
    if out.is_empty() || out.len() > 100 {
        return Err(format!("INVALID_ARGUMENTS: {k} takes 1 to 100 ids"));
    }
    Ok(out)
}

fn wrap(key: &str, v: Value) -> Result<Value, String> {
    Ok(json!({ key: v }))
}

async fn engine_call(command: &str, args: Value) -> Result<Value, String> {
    super::commands::dispatch(command.to_string(), args).await
}

// ── Chats: parsing and resolution ──────────────────────────────────────

/// What the caller wrote for `chat`, before talking to Telegram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatRef {
    /// A numeric id, with its type when the form tells it (Bot API ids,
    /// `t.me/c/` links, `{chatId, chatType}` objects).
    Id {
        id: i64,
        kind: Option<String>,
    },
    Username(String),
    Title(String),
    /// The account's own chat ("Saved Messages").
    SelfChat,
}

fn valid_username(s: &str) -> bool {
    s.len() >= 3 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn id_ref(n: i64) -> ChatRef {
    // Bot API ids: -100<channel id> for channels and supergroups,
    // -<chat id> for basic groups.
    if n <= -1_000_000_000_000 {
        ChatRef::Id {
            id: -n - 1_000_000_000_000,
            kind: Some("channel".into()),
        }
    } else if n < 0 {
        ChatRef::Id {
            id: -n,
            kind: Some("group".into()),
        }
    } else {
        ChatRef::Id { id: n, kind: None }
    }
}

/// Parse the `chat` argument: a number, `@username`, a t.me link, a
/// `{chatId, chatType}` object or a chat title.
pub fn parse_chat(v: &Value) -> Result<ChatRef, String> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .map(id_ref)
            .ok_or_else(|| "INVALID_ARGUMENTS: chat id must be an integer".into()),
        Value::Object(o) => {
            let id = o
                .get("chatId")
                .or_else(|| o.get("chat_id"))
                .or_else(|| o.get("id"))
                .and_then(Value::as_i64)
                .ok_or("INVALID_ARGUMENTS: chat object needs chatId")?;
            let kind = o
                .get("chatType")
                .or_else(|| o.get("chat_type"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Ok(ChatRef::Id { id, kind })
        }
        Value::String(s) => parse_chat_str(s),
        _ => Err("INVALID_ARGUMENTS: chat is required".into()),
    }
}

fn parse_chat_str(raw: &str) -> Result<ChatRef, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("INVALID_ARGUMENTS: chat is required".into());
    }
    if let Ok(n) = s.parse::<i64>() {
        return Ok(id_ref(n));
    }
    if matches!(
        s.to_lowercase().as_str(),
        "me" | "self"
            | "@me"
            | "saved"
            | "saved messages"
            | "mensagens salvas"
            | "收藏夹"
            | "已保存的消息"
    ) {
        return Ok(ChatRef::SelfChat);
    }
    if let Some(name) = s.strip_prefix('@') {
        if valid_username(name) {
            return Ok(ChatRef::Username(name.to_string()));
        }
        return Err(format!("INVALID_ARGUMENTS: {s} is not a valid username"));
    }
    if let Some(rest) = s.strip_prefix("tg://resolve?") {
        if let Some(domain) = rest
            .split('&')
            .find_map(|kv| kv.strip_prefix("domain="))
            .filter(|d| valid_username(d))
        {
            return Ok(ChatRef::Username(domain.to_string()));
        }
    }
    let lower = s.to_ascii_lowercase();
    let no_scheme = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let no_www = no_scheme.strip_prefix("www.").unwrap_or(no_scheme);
    let hosts = ["t.me/", "telegram.me/", "telegram.dog/"];
    if let Some(host) = hosts.iter().find(|h| no_www.starts_with(**h)) {
        // Keep the original case of the path (usernames are case-insensitive,
        // but the title fallback is not used for links anyway).
        let start = s.len() - no_www.len() + host.len();
        let path = s[start..].split(['?', '#']).next().unwrap_or("");
        let mut parts = path.split('/').filter(|p| !p.is_empty());
        let first = parts.next().unwrap_or("");
        if first.starts_with('+') || first.eq_ignore_ascii_case("joinchat") {
            return Err("INVALID_ARGUMENTS: invite links are not supported; join the chat in Telegram first".into());
        }
        let first = if first == "s" {
            parts.next().unwrap_or("")
        } else {
            first
        };
        if first == "c" {
            let id = parts
                .next()
                .and_then(|p| p.parse::<i64>().ok())
                .ok_or_else(|| format!("INVALID_ARGUMENTS: {s} has no chat id"))?;
            return Ok(ChatRef::Id {
                id,
                kind: Some("channel".into()),
            });
        }
        if valid_username(first) {
            return Ok(ChatRef::Username(first.to_string()));
        }
        return Err(format!("INVALID_ARGUMENTS: {s} is not a chat link"));
    }
    Ok(ChatRef::Title(s.to_string()))
}

/// A chat the engine can talk to.
#[derive(Debug, Clone, Serialize)]
pub struct Target {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
}

fn session() -> TelegramSessionHandle {
    client::engine().session.clone()
}

async fn ready() -> Result<TelegramSessionHandle, String> {
    let h = session();
    client::restore_session(&h).await;
    if h.lock().await.client.is_none() {
        return Err("NOT_LOGGED_IN: no Telegram account is logged in. Ask the person to log in on the /telegram screen of OmniGet.".into());
    }
    Ok(h)
}

type ChatCache = Mutex<Option<(Instant, Vec<api::TelegramChat>)>>;

fn chat_cache() -> &'static ChatCache {
    static C: OnceLock<ChatCache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// The chat list, kept for a minute so resolving a title is not a full
/// dialog walk every call.
async fn chats(h: &TelegramSessionHandle) -> Result<Vec<api::TelegramChat>, String> {
    if let Ok(g) = chat_cache().lock() {
        if let Some((at, list)) = g.as_ref() {
            if at.elapsed() < Duration::from_secs(60) {
                return Ok(list.clone());
            }
        }
    }
    let list = api::list_chats(h).await.map_err(|e| e.to_string())?;
    if let Ok(mut g) = chat_cache().lock() {
        *g = Some((Instant::now(), list.clone()));
    }
    Ok(list)
}

fn user_name(u: &tl::types::User) -> String {
    let n = [u.first_name.clone(), u.last_name.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    if n.trim().is_empty() {
        u.username
            .clone()
            .map(|x| format!("@{x}"))
            .unwrap_or_else(|| format!("user {}", u.id))
    } else {
        n
    }
}

async fn resolve_username(h: &TelegramSessionHandle, name: &str) -> Result<Target, String> {
    let client = h
        .lock()
        .await
        .client
        .clone()
        .ok_or("NOT_LOGGED_IN: no Telegram account is logged in")?;
    let req = tl::functions::contacts::ResolveUsername {
        username: name.to_string(),
        referer: None,
    };
    let tl::enums::contacts::ResolvedPeer::Peer(res) = api::invoke_with_flood_wait(&client, &req)
        .await
        .map_err(|e| format!("CHAT_NOT_FOUND: cannot resolve @{name}: {e}"))?;
    remember_hashes(h, &res.users, &res.chats).await;
    match res.peer {
        tl::enums::Peer::User(p) => {
            let title = res
                .users
                .iter()
                .find_map(|u| match u {
                    tl::enums::User::User(u) if u.id == p.user_id => Some(user_name(u)),
                    _ => None,
                })
                .unwrap_or_else(|| format!("@{name}"));
            Ok(Target {
                id: p.user_id,
                kind: "private".into(),
                title,
            })
        }
        tl::enums::Peer::Channel(p) => {
            let (title, broadcast) = res
                .chats
                .iter()
                .find_map(|c| match c {
                    tl::enums::Chat::Channel(ch) if ch.id == p.channel_id => {
                        Some((ch.title.clone(), ch.broadcast))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| (format!("@{name}"), true));
            Ok(Target {
                id: p.channel_id,
                kind: if broadcast { "channel" } else { "group" }.into(),
                title,
            })
        }
        tl::enums::Peer::Chat(p) => Ok(Target {
            id: p.chat_id,
            kind: "group".into(),
            title: format!("@{name}"),
        }),
    }
}

/// Access hashes seen in a response go into the session, so the next call
/// for that user or channel works without walking the dialogs.
async fn remember_hashes(
    h: &TelegramSessionHandle,
    users: &[tl::enums::User],
    chats: &[tl::enums::Chat],
) {
    let mut g = h.lock().await;
    for u in users {
        if let tl::enums::User::User(u) = u {
            if let Some(hash) = u.access_hash {
                g.peer_hashes.entry(u.id).or_insert(hash);
            }
        }
    }
    for c in chats {
        if let tl::enums::Chat::Channel(ch) = c {
            if let Some(hash) = ch.access_hash {
                g.peer_hashes.entry(ch.id).or_insert(hash);
            }
        }
    }
}

fn target_of(c: &api::TelegramChat) -> Target {
    Target {
        id: c.id,
        kind: c.chat_type.clone(),
        title: c.title.clone(),
    }
}

async fn resolve(h: &TelegramSessionHandle, r: &ChatRef) -> Result<Target, String> {
    match r {
        ChatRef::SelfChat => {
            let me = super::commands::dispatch("telegram_get_self".into(), json!({})).await?;
            let id = me
                .get("user_id")
                .and_then(Value::as_i64)
                .ok_or("NOT_LOGGED_IN: no account")?;
            Ok(Target {
                id,
                kind: "private".into(),
                title: "Saved Messages".into(),
            })
        }
        ChatRef::Username(name) => resolve_username(h, name).await,
        ChatRef::Id { id, kind } => {
            let list = chats(h).await?;
            if let Some(c) = list
                .iter()
                .find(|c| c.id == *id && kind.as_ref().is_none_or(|k| same_kind(k, &c.chat_type)))
            {
                return Ok(target_of(c));
            }
            match kind {
                Some(k) => Ok(Target {
                    id: *id,
                    kind: k.clone(),
                    title: id.to_string(),
                }),
                None => Err(format!("CHAT_NOT_FOUND: no chat with id {id} in the account's chat list. Use tg_list_chats, an @username or a t.me link.")),
            }
        }
        ChatRef::Title(title) => {
            let list = chats(h).await?;
            let exact: Vec<_> = list.iter().filter(|c| c.title == *title).collect();
            let hits = if exact.is_empty() {
                list.iter()
                    .filter(|c| c.title.to_lowercase() == title.to_lowercase())
                    .collect()
            } else {
                exact
            };
            match hits.as_slice() {
                [one] => Ok(target_of(one)),
                [] => {
                    let needle = title.to_lowercase();
                    let close: Vec<String> = list
                        .iter()
                        .filter(|c| c.title.to_lowercase().contains(&needle))
                        .take(5)
                        .map(|c| format!("{} (id {})", c.title, c.id))
                        .collect();
                    if close.is_empty() {
                        Err(format!(
                            "CHAT_NOT_FOUND: no chat titled \"{title}\". Use tg_list_chats."
                        ))
                    } else {
                        Err(format!(
                            "CHAT_NOT_FOUND: no chat titled exactly \"{title}\". Close matches: {}",
                            close.join("; ")
                        ))
                    }
                }
                many => Err(format!(
                    "CHAT_AMBIGUOUS: several chats are titled \"{title}\": {}. Pass the id.",
                    many.iter()
                        .map(|c| format!("{} {}", c.chat_type, c.id))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
    }
}

/// A supergroup is `group` in the chat list and `channel` in a t.me/c link.
fn same_kind(asked: &str, listed: &str) -> bool {
    asked == listed
        || (asked == "channel" && listed == "group")
        || (asked == "group" && listed == "channel")
}

async fn target_arg(a: &Value, k: &str) -> Result<Target, String> {
    let r = parse_chat(a.get(k).unwrap_or(&Value::Null)).map_err(|e| e.replace("chat", k))?;
    let h = ready().await?;
    resolve(&h, &r).await
}

// ── Messages ───────────────────────────────────────────────────────────

/// One message, compact enough for a model's context.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Msg {
    pub id: i32,
    pub date: i64,
    pub author: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forwarded_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouped_id: Option<i64>,
}

type Names = HashMap<(u8, i64), String>;

fn peer_key(p: &tl::enums::Peer) -> (u8, i64) {
    match p {
        tl::enums::Peer::User(u) => (0, u.user_id),
        tl::enums::Peer::Chat(c) => (1, c.chat_id),
        tl::enums::Peer::Channel(c) => (2, c.channel_id),
    }
}

fn names_of(users: &[tl::enums::User], chats: &[tl::enums::Chat]) -> Names {
    let mut names = Names::new();
    for u in users {
        if let tl::enums::User::User(u) = u {
            names.insert((0, u.id), user_name(u));
        }
    }
    for c in chats {
        match c {
            tl::enums::Chat::Chat(c) => {
                names.insert((1, c.id), c.title.clone());
            }
            tl::enums::Chat::Channel(c) => {
                names.insert((2, c.id), c.title.clone());
            }
            _ => {}
        }
    }
    names
}

fn unpack(
    r: tl::enums::messages::Messages,
) -> (
    Vec<tl::enums::Message>,
    Vec<tl::enums::User>,
    Vec<tl::enums::Chat>,
) {
    match r {
        tl::enums::messages::Messages::Messages(m) => (m.messages, m.users, m.chats),
        tl::enums::messages::Messages::Slice(m) => (m.messages, m.users, m.chats),
        tl::enums::messages::Messages::ChannelMessages(m) => (m.messages, m.users, m.chats),
        tl::enums::messages::Messages::NotModified(_) => (vec![], vec![], vec![]),
    }
}

/// `offset` and `len` are UTF-16 code units, as Telegram counts them.
fn utf16_slice(s: &str, offset: i32, len: i32) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    let start = offset.max(0) as usize;
    let end = start.saturating_add(len.max(0) as usize).min(units.len());
    if start >= end {
        return String::new();
    }
    String::from_utf16_lossy(&units[start..end])
}

fn links_of(m: &tl::types::Message) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in m.entities.iter().flatten() {
        let url = match e {
            tl::enums::MessageEntity::Url(u) => utf16_slice(&m.message, u.offset, u.length),
            tl::enums::MessageEntity::TextUrl(u) => u.url.clone(),
            _ => continue,
        };
        if !url.is_empty() && !out.contains(&url) {
            out.push(url);
        }
    }
    if let Some(wp) = m.media.as_ref().and_then(api::extract_webpage_info_raw) {
        if !out.contains(&wp.url) {
            out.push(wp.url);
        }
    }
    out
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

fn media_summary(media: &tl::enums::MessageMedia) -> Option<String> {
    use tl::enums::MessageMedia as M;
    if let Some(wp) = api::extract_webpage_info_raw(media) {
        return Some(match wp.title.or(wp.site_name) {
            Some(t) => format!("link preview: {t}"),
            None => "link preview".into(),
        });
    }
    if let Some((name, size, kind)) = api::extract_raw_media_info(media, true) {
        return Some(format!("{kind} \"{name}\" {}", human_size(size)));
    }
    match media {
        M::Empty => None,
        M::Poll(_) => Some("poll".into()),
        M::Geo(_) | M::GeoLive(_) | M::Venue(_) => Some("location".into()),
        M::Contact(_) => Some("contact".into()),
        M::Dice(_) => Some("dice".into()),
        _ => Some("media".into()),
    }
}

const MAX_TEXT: usize = 4000;

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

fn to_msg(m: &tl::types::Message, names: &Names, chat_title: &str) -> Msg {
    let author = if m.out {
        "me".to_string()
    } else if let Some(p) = &m.from_id {
        names
            .get(&peer_key(p))
            .cloned()
            .unwrap_or_else(|| format!("id {}", peer_key(p).1))
    } else if let Some(a) = &m.post_author {
        a.clone()
    } else {
        names
            .get(&peer_key(&m.peer_id))
            .cloned()
            .unwrap_or_else(|| chat_title.to_string())
    };
    let reply_to = match &m.reply_to {
        Some(tl::enums::MessageReplyHeader::Header(h)) => h.reply_to_msg_id,
        _ => None,
    };
    let forwarded_from = match &m.fwd_from {
        Some(tl::enums::MessageFwdHeader::Header(f)) => f
            .from_name
            .clone()
            .or_else(|| {
                f.from_id
                    .as_ref()
                    .and_then(|p| names.get(&peer_key(p)).cloned())
            })
            .or_else(|| Some("someone".into())),
        None => None,
    };
    Msg {
        id: m.id,
        date: m.date as i64,
        author,
        text: clip(&m.message, MAX_TEXT),
        media: m.media.as_ref().and_then(media_summary),
        links: links_of(m),
        reply_to,
        forwarded_from,
        grouped_id: m.grouped_id,
    }
}

#[derive(Debug, Clone, Default)]
struct Range {
    limit: usize,
    offset_id: i32,
    from_date: Option<i64>,
    to_date: Option<i64>,
}

/// Messages of one chat in `range`, oldest first. Service messages (joins,
/// pins) are skipped.
async fn read_history(
    h: &TelegramSessionHandle,
    t: &Target,
    range: &Range,
) -> Result<Vec<Msg>, String> {
    let (client, peer, _) = history::peer_for(h, t.id, &t.kind)
        .await
        .map_err(|e| e.to_string())?;
    let mut out: Vec<Msg> = Vec::new();
    let mut offset_id = range.offset_id.max(0);
    let mut offset_date = if offset_id == 0 {
        range.to_date.map(|d| (d + 1) as i32).unwrap_or(0)
    } else {
        0
    };
    'pages: while out.len() < range.limit {
        let want = (range.limit - out.len()).min(100) as i32;
        let req = tl::functions::messages::GetHistory {
            peer: peer.clone(),
            offset_id,
            offset_date,
            add_offset: 0,
            limit: want,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let r = api::invoke_with_flood_wait(&client, &req)
            .await
            .map_err(|e| format!("messages.getHistory: {e}"))?;
        let (messages, users, chats) = unpack(r);
        if messages.is_empty() {
            break;
        }
        remember_hashes(h, &users, &chats).await;
        let names = names_of(&users, &chats);
        let mut lowest = i32::MAX;
        for raw in &messages {
            let (id, date) = match raw {
                tl::enums::Message::Message(m) => (m.id, m.date as i64),
                tl::enums::Message::Service(m) => (m.id, m.date as i64),
                tl::enums::Message::Empty(m) => (m.id, 0),
            };
            lowest = lowest.min(id);
            if range.from_date.is_some_and(|f| date > 0 && date < f) {
                break 'pages;
            }
            if let tl::enums::Message::Message(m) = raw {
                out.push(to_msg(m, &names, &t.title));
                if out.len() >= range.limit {
                    break 'pages;
                }
            }
        }
        if lowest <= 1 || lowest == i32::MAX || messages.len() < want as usize {
            break;
        }
        offset_id = lowest;
        offset_date = 0;
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    out.reverse();
    Ok(out)
}

/// Messages by id (for the approval preview of a forward or a delete).
async fn fetch_by_ids(
    h: &TelegramSessionHandle,
    t: &Target,
    ids: &[i32],
) -> Result<Vec<Msg>, String> {
    let (client, _, hash) = history::peer_for(h, t.id, &t.kind)
        .await
        .map_err(|e| e.to_string())?;
    let id: Vec<tl::enums::InputMessage> = ids
        .iter()
        .map(|i| tl::enums::InputMessage::Id(tl::types::InputMessageId { id: *i }))
        .collect();
    let is_channel = t.kind == "channel" || (t.kind == "group" && hash != 0);
    let r = if is_channel {
        let req = tl::functions::channels::GetMessages {
            channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
                channel_id: t.id,
                access_hash: hash,
            }),
            id,
        };
        api::invoke_with_flood_wait(&client, &req).await
    } else {
        let req = tl::functions::messages::GetMessages { id };
        api::invoke_with_flood_wait(&client, &req).await
    }
    .map_err(|e| e.to_string())?;
    let (messages, users, chats) = unpack(r);
    let names = names_of(&users, &chats);
    Ok(messages
        .iter()
        .filter_map(|m| match m {
            tl::enums::Message::Message(m) => Some(to_msg(m, &names, &t.title)),
            _ => None,
        })
        .collect())
}

// ── Read tools ─────────────────────────────────────────────────────────

async fn status() -> Result<Value, String> {
    let h = session();
    client::restore_session(&h).await;
    let logged_in = h.lock().await.client.is_some();
    let account = if logged_in {
        engine_call("telegram_get_self", json!({})).await.ok()
    } else {
        None
    };
    let net = serde_json::to_value(super::net::manager::get().status()).unwrap_or(Value::Null);
    let active = &net["active"];
    Ok(json!({
        "logged_in": logged_in,
        "account": account,
        "hint": if logged_in { Value::Null } else { json!("Ask the person to log in on the /telegram screen of OmniGet.") },
        "connection": {
            "phase": net["phase"],
            "mode": net["mode"],
            "route": if active.is_null() { Value::Null } else { json!({
                "name": active["route"]["name"],
                "kind": active["route"]["kind"],
                "rtt_ms": active["health"]["rtt_ms"],
            }) },
            "routes": net["routes"].as_array().map(|r| r.len()).unwrap_or(0),
        },
    }))
}

fn chat_row(c: &api::TelegramChat) -> Value {
    json!({
        "id": c.id,
        "title": c.title,
        "type": c.chat_type,
        "unread": c.unread_count,
        "muted": c.is_muted,
        "pinned": c.is_pinned,
        "last_message": c.last_message.as_deref().map(|m| clip(m, 120)),
        "last_message_date": c.last_message_date,
    })
}

async fn list_chats(a: &Value) -> Result<Value, String> {
    let h = ready().await?;
    let limit = limit_arg(a, 50, 500);
    let q = opt_str(a, "query").map(|q| q.to_lowercase());
    let list = chats(&h).await?;
    let matched: Vec<&api::TelegramChat> = list
        .iter()
        .filter(|c| {
            q.as_ref()
                .is_none_or(|q| c.title.to_lowercase().contains(q))
        })
        .collect();
    Ok(json!({
        "total": matched.len(),
        "chats": matched.iter().take(limit).map(|c| chat_row(c)).collect::<Vec<_>>(),
    }))
}

async fn read_messages(a: &Value) -> Result<Value, String> {
    let t = target_arg(a, "chat").await?;
    let h = session();
    let limit = limit_arg(a, 50, 500);
    let range = Range {
        limit,
        offset_id: opt_i64(a, "offsetId")
            .unwrap_or(0)
            .clamp(0, i32::MAX as i64) as i32,
        from_date: opt_i64(a, "fromDate"),
        to_date: opt_i64(a, "toDate"),
    };
    let msgs = read_history(&h, &t, &range).await?;
    let next = (msgs.len() >= limit)
        .then(|| msgs.first().map(|m| m.id))
        .flatten();
    Ok(json!({ "chat": t, "count": msgs.len(), "messages": msgs, "next_offset_id": next }))
}

fn kind_of_peer(p: &tl::enums::Peer, chats: &[tl::enums::Chat]) -> &'static str {
    match p {
        tl::enums::Peer::User(_) => "private",
        tl::enums::Peer::Chat(_) => "group",
        tl::enums::Peer::Channel(c) => {
            let broadcast = chats.iter().any(|x| {
                matches!(x, tl::enums::Chat::Channel(ch) if ch.id == c.channel_id && ch.broadcast)
            });
            if broadcast {
                "channel"
            } else {
                "group"
            }
        }
    }
}

async fn search(a: &Value) -> Result<Value, String> {
    let query = need_str(a, "query")?;
    let limit = limit_arg(a, 30, 100) as i32;
    let h = ready().await?;
    if a.get("chat").is_some_and(|c| !c.is_null()) {
        let t = target_arg(a, "chat").await?;
        let (client, peer, _) = history::peer_for(&h, t.id, &t.kind)
            .await
            .map_err(|e| e.to_string())?;
        let req = tl::functions::messages::Search {
            peer,
            q: query.clone(),
            from_id: None,
            saved_peer_id: None,
            saved_reaction: None,
            top_msg_id: None,
            filter: tl::enums::MessagesFilter::InputMessagesFilterEmpty,
            min_date: 0,
            max_date: 0,
            offset_id: 0,
            add_offset: 0,
            limit,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let r = api::invoke_with_flood_wait(&client, &req)
            .await
            .map_err(|e| format!("messages.search: {e}"))?;
        let (messages, users, chats) = unpack(r);
        let names = names_of(&users, &chats);
        let hits: Vec<Msg> = messages
            .iter()
            .filter_map(|m| match m {
                tl::enums::Message::Message(m) => Some(to_msg(m, &names, &t.title)),
                _ => None,
            })
            .collect();
        return Ok(json!({ "chat": t, "count": hits.len(), "hits": hits }));
    }
    let client = h
        .lock()
        .await
        .client
        .clone()
        .ok_or("NOT_LOGGED_IN: no Telegram account is logged in")?;
    let req = tl::functions::messages::SearchGlobal {
        broadcasts_only: false,
        groups_only: false,
        users_only: false,
        folder_id: None,
        q: query,
        filter: tl::enums::MessagesFilter::InputMessagesFilterEmpty,
        min_date: 0,
        max_date: 0,
        offset_rate: 0,
        offset_peer: tl::enums::InputPeer::Empty,
        offset_id: 0,
        limit,
    };
    let r = api::invoke_with_flood_wait(&client, &req)
        .await
        .map_err(|e| format!("messages.searchGlobal: {e}"))?;
    let (messages, users, chats) = unpack(r);
    remember_hashes(&h, &users, &chats).await;
    let names = names_of(&users, &chats);
    let hits: Vec<Value> = messages
        .iter()
        .filter_map(|m| match m {
            tl::enums::Message::Message(m) => {
                let key = peer_key(&m.peer_id);
                let title = names
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| key.1.to_string());
                let msg = to_msg(m, &names, &title);
                let mut v = serde_json::to_value(msg).ok()?;
                v["chat_id"] = json!(key.1);
                v["chat_type"] = json!(kind_of_peer(&m.peer_id, &chats));
                v["chat_title"] = json!(title);
                Some(v)
            }
            _ => None,
        })
        .collect();
    Ok(json!({ "count": hits.len(), "hits": hits }))
}

async fn chat_info(a: &Value) -> Result<Value, String> {
    let t = target_arg(a, "chat").await?;
    let h = session();
    let list = chats(&h).await.unwrap_or_default();
    let mut out = match list.iter().find(|c| c.id == t.id) {
        Some(c) => chat_row(c),
        None => json!({ "id": t.id, "title": t.title, "type": t.kind }),
    };
    if t.kind == "channel" || t.kind == "group" {
        if let Ok(full) = api::full_channel_info(&h, t.id, &t.kind).await {
            out["about"] = json!(full.about);
            out["username"] = json!(full.username);
            out["members"] = json!(full.participants_count);
            if out["title"]
                .as_str()
                .is_none_or(|s| s.is_empty() || s == t.id.to_string())
            {
                out["title"] = json!(full.title);
            }
        }
    }
    Ok(out)
}

async fn list_media(a: &Value) -> Result<Value, String> {
    let t = target_arg(a, "chat").await?;
    let limit = limit_arg(a, 50, 200);
    let media_type = opt_str(a, "mediaType");
    let items = match opt_str(a, "query") {
        Some(q) => {
            engine_call(
                "telegram_search_media",
                json!({ "chatId": t.id, "chatType": t.kind, "query": q, "mediaType": media_type, "limit": limit }),
            )
            .await?
        }
        None => {
            engine_call(
                "telegram_list_media",
                json!({ "chatId": t.id, "chatType": t.kind, "mediaType": media_type, "limit": limit }),
            )
            .await?
        }
    };
    let count = items.as_array().map(|v| v.len()).unwrap_or(0);
    Ok(json!({ "chat": t, "count": count, "items": items }))
}

async fn download_status() -> Result<Value, String> {
    let active = client::engine().active.clone();
    let list: Vec<Value> = active
        .lock()
        .await
        .iter()
        .map(|(id, (title, _))| json!({ "id": id, "title": title }))
        .collect();
    Ok(json!({ "count": list.len(), "active": list }))
}

// ── Digest and export ──────────────────────────────────────────────────

/// One chat's part of a digest.
#[derive(Debug, Clone)]
pub struct DigestChat {
    pub target: Target,
    pub unread: i32,
    pub messages: Vec<Msg>,
    /// More messages existed in the period than were read.
    pub truncated: bool,
}

/// Longest digest or export text returned to a model.
pub const MAX_CONTEXT_CHARS: usize = 100_000;
const DIGEST_TEXT: usize = 500;

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One message as a single line: `#id [time] author: text [media] <links>`.
pub fn message_line(m: &Msg, time: &str, max_text: usize) -> String {
    let mut line = format!("#{} [{}] {}", m.id, time, m.author);
    if let Some(f) = &m.forwarded_from {
        line.push_str(&format!(" (fwd from {f})"));
    }
    if let Some(r) = m.reply_to {
        line.push_str(&format!(" (reply to #{r})"));
    }
    line.push(':');
    let text = one_line(&m.text);
    if !text.is_empty() {
        line.push(' ');
        line.push_str(&clip(&text, max_text));
    }
    if let Some(media) = &m.media {
        line.push_str(&format!(" [{media}]"));
    }
    for l in &m.links {
        line.push_str(&format!(" <{l}>"));
    }
    line
}

fn local_time(ts: i64, fmt: &str) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format(fmt).to_string())
        .unwrap_or_else(|| ts.to_string())
}

/// The digest text: a header, then one block per chat with messages.
/// `time` formats a unix timestamp (local `MM-DD HH:MM` in the tool).
pub fn format_digest(
    chats: &[DigestChat],
    since_hours: u32,
    time: &dyn Fn(i64) -> String,
) -> String {
    let with: Vec<&DigestChat> = chats.iter().filter(|c| !c.messages.is_empty()).collect();
    let total: usize = with.iter().map(|c| c.messages.len()).sum();
    let mut out = format!(
        "Telegram digest, last {since_hours} h: {} chats, {total} messages.\n",
        with.len()
    );
    if with.is_empty() {
        out.push_str("Nothing new in this period.\n");
        return out;
    }
    for c in with {
        out.push_str(&format!(
            "\n## {} ({}, id {}) · {} messages",
            c.target.title,
            c.target.kind,
            c.target.id,
            c.messages.len()
        ));
        if c.unread > 0 {
            out.push_str(&format!(" · {} unread", c.unread));
        }
        if c.truncated {
            out.push_str(" · older messages of the period left out");
        }
        out.push('\n');
        for m in &c.messages {
            out.push_str(&message_line(m, &time(m.date), DIGEST_TEXT));
            out.push('\n');
        }
    }
    cut(out, MAX_CONTEXT_CHARS)
}

fn cut(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push_str("\n… (cut to fit; ask for fewer chats or a shorter period)\n");
    out
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn digest(a: &Value) -> Result<Value, String> {
    let h = ready().await?;
    let since_hours = opt_i64(a, "sinceHours").unwrap_or(24).clamp(1, 720) as u32;
    let max_messages = opt_i64(a, "maxMessages").unwrap_or(400).clamp(10, 3000) as usize;
    let since = now_secs() - since_hours as i64 * 3600;
    let list = chats(&h).await?;
    let unread_of = |id: i64| {
        list.iter()
            .find(|c| c.id == id)
            .map(|c| c.unread_count)
            .unwrap_or(0)
    };

    let mut targets: Vec<Target> = Vec::new();
    let mut errors: Vec<Value> = Vec::new();
    match a
        .get("chats")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
    {
        Some(asked) => {
            for c in asked.iter().take(30) {
                match parse_chat(c) {
                    Ok(r) => match resolve(&h, &r).await {
                        Ok(t) => targets.push(t),
                        Err(e) => errors.push(json!({ "chat": c, "error": e })),
                    },
                    Err(e) => errors.push(json!({ "chat": c, "error": e })),
                }
            }
        }
        None => {
            let mut active: Vec<&api::TelegramChat> = list
                .iter()
                .filter(|c| c.last_message_date.is_some_and(|d| d as i64 >= since))
                .collect();
            active.sort_by(|x, y| {
                (y.unread_count > 0)
                    .cmp(&(x.unread_count > 0))
                    .then(y.last_message_date.cmp(&x.last_message_date))
            });
            targets = active.into_iter().take(15).map(target_of).collect();
        }
    }

    let per_chat = (max_messages / targets.len().max(1)).max(20);
    let mut parts: Vec<DigestChat> = Vec::new();
    let mut left = max_messages;
    for t in targets {
        if left == 0 {
            break;
        }
        let want = per_chat.min(left);
        let range = Range {
            limit: want + 1,
            from_date: Some(since),
            ..Default::default()
        };
        match read_history(&h, &t, &range).await {
            Ok(mut msgs) => {
                let truncated = msgs.len() > want;
                if truncated {
                    msgs.remove(0);
                }
                left = left.saturating_sub(msgs.len());
                parts.push(DigestChat {
                    unread: unread_of(t.id),
                    target: t,
                    messages: msgs,
                    truncated,
                });
            }
            Err(e) => errors.push(json!({ "chat": t.title, "error": e })),
        }
    }
    let text = format_digest(&parts, since_hours, &|ts| local_time(ts, "%m-%d %H:%M"));
    let count: usize = parts.iter().map(|p| p.messages.len()).sum();
    Ok(json!({
        "since": since,
        "chats": parts.iter().filter(|p| !p.messages.is_empty()).count(),
        "messages": count,
        "digest": text,
        "errors": errors,
    }))
}

async fn export(a: &Value) -> Result<Value, String> {
    let t = target_arg(a, "chat").await?;
    let h = session();
    let format = opt_str(a, "format").unwrap_or_else(|| "markdown".into());
    if format != "markdown" && format != "json" {
        return Err("INVALID_ARGUMENTS: format is markdown or json".into());
    }
    let range = Range {
        limit: limit_arg(a, 500, 3000),
        from_date: opt_i64(a, "fromDate"),
        to_date: opt_i64(a, "toDate"),
        ..Default::default()
    };
    let msgs = read_history(&h, &t, &range).await?;
    let count = msgs.len();
    let (content, truncated) = if format == "json" {
        // Keep the newest messages that fit.
        let mut keep = msgs;
        let mut s = serde_json::to_string(&keep).unwrap_or_default();
        while s.len() > MAX_CONTEXT_CHARS && keep.len() > 1 {
            let drop = (keep.len() / 10).max(1);
            keep.drain(..drop);
            s = serde_json::to_string(&keep).unwrap_or_default();
        }
        let truncated = keep.len() < count;
        (
            Value::Array(
                keep.into_iter()
                    .filter_map(|m| serde_json::to_value(m).ok())
                    .collect(),
            ),
            truncated,
        )
    } else {
        let mut md = format!("# {} ({}, id {})\n\n", t.title, t.kind, t.id);
        for m in &msgs {
            md.push_str(&message_line(
                m,
                &local_time(m.date, "%Y-%m-%d %H:%M"),
                MAX_TEXT,
            ));
            md.push('\n');
        }
        let truncated = md.len() > MAX_CONTEXT_CHARS;
        (json!(cut(md, MAX_CONTEXT_CHARS)), truncated)
    };
    Ok(json!({
        "chat": t,
        "format": format,
        "count": count,
        "truncated": truncated,
        "content": content,
    }))
}

// ── Local actions ──────────────────────────────────────────────────────

fn copy_keys(a: &Value, keys: &[&str]) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    for k in keys {
        if let Some(v) = a.get(*k).filter(|v| !v.is_null()) {
            m.insert((*k).to_string(), v.clone());
        }
    }
    m
}

const FILTER_KEYS: &[&str] = &[
    "mediaTypes",
    "fromDate",
    "toDate",
    "minSize",
    "maxSize",
    "keyword",
    "extInclude",
    "extExclude",
];

async fn download(a: &Value) -> Result<Value, String> {
    let output_dir = a.get("outputDir").cloned().unwrap_or(Value::Null);
    let template = a.get("template").cloned().unwrap_or(Value::Null);
    let links = match a.get("links") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        Some(Value::Array(v)) if !v.is_empty() => Some(
            v.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    };
    if let Some(text) = links {
        return engine_call(
            "telegram_download_links",
            json!({ "text": text, "outputDir": output_dir, "template": template }),
        )
        .await;
    }
    if a.get("chat").is_none_or(Value::is_null) {
        return Err("INVALID_ARGUMENTS: pass links, or chat with filters".into());
    }
    let t = target_arg(a, "chat").await?;
    let mut filter = copy_keys(a, FILTER_KEYS);
    filter.insert("limit".into(), json!(limit_arg(a, 200, 5000)));
    let out = engine_call(
        "telegram_download_chat",
        json!({
            "chatId": t.id, "chatType": t.kind, "chatTitle": t.title,
            "filter": Value::Object(filter), "outputDir": output_dir, "template": template,
        }),
    )
    .await?;
    Ok(json!({ "chat": t, "batch_id": out["batch_id"], "count": out["count"] }))
}

async fn watch_add(a: &Value) -> Result<Value, String> {
    let t = target_arg(a, "chat").await?;
    let mut rule = copy_keys(
        a,
        &[
            "mediaTypes",
            "minSize",
            "maxSize",
            "keyword",
            "extInclude",
            "extExclude",
            "outputDir",
            "template",
        ],
    );
    rule.insert("chatId".into(), json!(t.id));
    rule.insert("chatType".into(), json!(t.kind));
    rule.insert("chatTitle".into(), json!(t.title));
    rule.insert("enabled".into(), json!(true));
    let mut args = json!({ "rule": Value::Object(rule) });
    if let Some(n) = opt_i64(a, "backfill").filter(|n| *n > 0) {
        args["backfill"] = json!(n.min(1000));
    }
    wrap("rule", engine_call("telegram_watch_add", args).await?)
}

// ── Writes (approval first) ────────────────────────────────────────────

async fn approve(
    caller: &Caller,
    tool: &str,
    summary: String,
    details: Value,
) -> Result<(), String> {
    if matches!(caller, Caller::Screen) {
        return Ok(());
    }
    approvals::request(tool, summary, details, &caller.label()).await
}

async fn preview(t: &Target, ids: &[i32]) -> Value {
    let h = session();
    match fetch_by_ids(&h, t, &ids[..ids.len().min(10)]).await {
        Ok(msgs) => json!(msgs
            .iter()
            .map(|m| message_line(m, &local_time(m.date, "%Y-%m-%d %H:%M"), 300))
            .collect::<Vec<_>>()),
        Err(_) => Value::Null,
    }
}

fn sent_id(u: &tl::enums::Updates) -> Option<i32> {
    match u {
        tl::enums::Updates::UpdateShortSentMessage(s) => Some(s.id),
        tl::enums::Updates::Updates(x) => x.updates.iter().find_map(|u| match u {
            tl::enums::Update::MessageId(m) => Some(m.id),
            _ => None,
        }),
        tl::enums::Updates::Combined(x) => x.updates.iter().find_map(|u| match u {
            tl::enums::Update::MessageId(m) => Some(m.id),
            _ => None,
        }),
        _ => None,
    }
}

async fn send_message(caller: &Caller, a: &Value) -> Result<Value, String> {
    let text = a
        .get("text")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or("INVALID_ARGUMENTS: text is required")?
        .to_string();
    if text.encode_utf16().count() > 4096 {
        return Err("INVALID_ARGUMENTS: text is longer than 4096 characters".into());
    }
    let t = target_arg(a, "chat").await?;
    approve(
        caller,
        "tg_send_message",
        format!("Send a message to {}", t.title),
        json!({ "chat": t, "text": text }),
    )
    .await?;
    let h = ready().await?;
    let (client, peer, _) = history::peer_for(&h, t.id, &t.kind)
        .await
        .map_err(|e| e.to_string())?;
    use rand::RngExt;
    let req = tl::functions::messages::SendMessage {
        no_webpage: false,
        silent: false,
        background: false,
        clear_draft: false,
        noforwards: false,
        update_stickersets_order: false,
        invert_media: false,
        allow_paid_floodskip: false,
        peer,
        reply_to: None,
        message: text,
        random_id: rand::rng().random::<i64>(),
        reply_markup: None,
        entities: None,
        schedule_date: None,
        send_as: None,
        quick_reply_shortcut: None,
        effect: None,
        allow_paid_stars: None,
        suggested_post: None,
    };
    let updates = client
        .invoke(&req)
        .await
        .map_err(|e| format!("messages.sendMessage: {e}"))?;
    Ok(json!({ "ok": true, "chat": t, "message_id": sent_id(&updates) }))
}

async fn send_files(caller: &Caller, a: &Value) -> Result<Value, String> {
    let paths: Vec<String> = a
        .get("paths")
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if paths.is_empty() || paths.len() > 50 {
        return Err("INVALID_ARGUMENTS: paths takes 1 to 50 files or folders".into());
    }
    let mut files = Vec::new();
    for p in &paths {
        let meta = std::fs::metadata(p).map_err(|e| format!("INVALID_ARGUMENTS: {p}: {e}"))?;
        files.push(json!({ "path": p, "folder": meta.is_dir(), "size": if meta.is_dir() { Value::Null } else { json!(human_size(meta.len())) } }));
    }
    let t = target_arg(a, "chat").await?;
    let caption = opt_str(a, "caption");
    let as_document = a
        .get("asDocument")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    approve(
        caller,
        "tg_send_files",
        format!("Send {} item(s) to {}", paths.len(), t.title),
        json!({ "chat": t, "files": files, "caption": caption, "as_document": as_document }),
    )
    .await?;
    let out = engine_call(
        "telegram_upload_paths",
        json!({
            "chatId": t.id, "chatType": t.kind, "paths": paths, "caption": caption,
            "asDocument": as_document, "groupAlbums": true,
        }),
    )
    .await?;
    Ok(json!({ "ok": true, "chat": t, "batch_id": out["batch_id"], "count": out["count"] }))
}

async fn forward(caller: &Caller, a: &Value) -> Result<Value, String> {
    let ids = ids_arg(a, "messageIds")?;
    let from = target_arg(a, "fromChat").await?;
    let to = target_arg(a, "toChat").await?;
    let messages = preview(&from, &ids).await;
    approve(
        caller,
        "tg_forward",
        format!(
            "Forward {} message(s) from {} to {}",
            ids.len(),
            from.title,
            to.title
        ),
        json!({ "from": from, "to": to, "message_ids": ids, "messages": messages }),
    )
    .await?;
    engine_call(
        "telegram_forward_messages",
        json!({
            "fromChatId": from.id, "fromChatType": from.kind,
            "toChatId": to.id, "toChatType": to.kind, "messageIds": ids,
        }),
    )
    .await?;
    Ok(json!({ "ok": true, "from": from, "to": to, "count": ids.len() }))
}

async fn delete_messages(caller: &Caller, a: &Value) -> Result<Value, String> {
    let ids = ids_arg(a, "messageIds")?;
    let t = target_arg(a, "chat").await?;
    let messages = preview(&t, &ids).await;
    approve(
        caller,
        "tg_delete_messages",
        format!("Delete {} message(s) in {}", ids.len(), t.title),
        json!({ "chat": t, "message_ids": ids, "messages": messages }),
    )
    .await?;
    let out = engine_call(
        "telegram_delete_messages",
        json!({ "chatId": t.id, "chatType": t.kind, "messageIds": ids, "revoke": true }),
    )
    .await?;
    Ok(json!({ "ok": true, "chat": t, "count": ids.len(), "result": out }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(id: i64, kind: Option<&str>) -> ChatRef {
        ChatRef::Id {
            id,
            kind: kind.map(str::to_string),
        }
    }

    #[test]
    fn chat_accepts_ids_usernames_links_and_titles() {
        assert_eq!(parse_chat(&json!(12345)).unwrap(), id(12345, None));
        assert_eq!(parse_chat(&json!("12345")).unwrap(), id(12345, None));
        assert_eq!(
            parse_chat(&json!(-1001234567890_i64)).unwrap(),
            id(1234567890, Some("channel"))
        );
        assert_eq!(
            parse_chat(&json!("-4567")).unwrap(),
            id(4567, Some("group"))
        );
        assert_eq!(
            parse_chat(&json!({ "chatId": 9, "chatType": "private" })).unwrap(),
            id(9, Some("private"))
        );
        assert_eq!(
            parse_chat(&json!("@durov")).unwrap(),
            ChatRef::Username("durov".into())
        );
        assert_eq!(
            parse_chat(&json!("https://t.me/TelegramTips/123")).unwrap(),
            ChatRef::Username("TelegramTips".into())
        );
        assert_eq!(
            parse_chat(&json!("t.me/s/durov")).unwrap(),
            ChatRef::Username("durov".into())
        );
        assert_eq!(
            parse_chat(&json!("https://t.me/c/1234567/89")).unwrap(),
            id(1234567, Some("channel"))
        );
        assert_eq!(
            parse_chat(&json!("tg://resolve?domain=durov&post=1")).unwrap(),
            ChatRef::Username("durov".into())
        );
        assert_eq!(
            parse_chat(&json!("  Family group ")).unwrap(),
            ChatRef::Title("Family group".into())
        );
        assert!(parse_chat(&json!("https://t.me/+AbCdEf")).is_err());
        assert!(parse_chat(&json!("@x")).is_err());
        assert!(parse_chat(&json!("")).is_err());
        assert!(parse_chat(&Value::Null).is_err());
    }

    #[test]
    fn message_ids_accept_numbers_and_strings_once() {
        let a = json!({ "ids": [5, "6", "#7", 5] });
        assert_eq!(ids_arg(&a, "ids").unwrap(), vec![5, 6, 7]);
        assert!(ids_arg(&json!({ "ids": [] }), "ids").is_err());
        assert!(ids_arg(&json!({ "ids": [0] }), "ids").is_err());
    }

    #[test]
    fn links_are_cut_in_utf16_units() {
        // "é" is one UTF-16 unit, "😀" is two.
        let s = "😀 é https://a.b/c";
        assert_eq!(utf16_slice(s, 5, 13), "https://a.b/c");
        assert_eq!(utf16_slice(s, 100, 3), "");
    }

    fn msg(id: i32, author: &str, text: &str) -> Msg {
        Msg {
            id,
            date: 0,
            author: author.into(),
            text: text.into(),
            media: None,
            links: vec![],
            reply_to: None,
            forwarded_from: None,
            grouped_id: None,
        }
    }

    #[test]
    fn digest_groups_by_chat_one_line_per_message() {
        let mut with_media = msg(11, "Bob", "look\nat this");
        with_media.media = Some("video \"a.mp4\" 1.0 MB".into());
        with_media.links = vec!["https://x.y".into()];
        with_media.reply_to = Some(10);
        let chats = vec![
            DigestChat {
                target: Target {
                    id: 1,
                    kind: "group".into(),
                    title: "Team".into(),
                },
                unread: 3,
                messages: vec![msg(10, "Ann", "hi"), with_media],
                truncated: true,
            },
            DigestChat {
                target: Target {
                    id: 2,
                    kind: "channel".into(),
                    title: "Quiet".into(),
                },
                unread: 0,
                messages: vec![],
                truncated: false,
            },
        ];
        let text = format_digest(&chats, 24, &|_| "09-28 14:05".into());
        assert!(text.starts_with("Telegram digest, last 24 h: 1 chats, 2 messages.\n"));
        assert!(text.contains(
            "## Team (group, id 1) · 2 messages · 3 unread · older messages of the period left out\n"
        ));
        assert!(text.contains("#10 [09-28 14:05] Ann: hi\n"));
        assert!(text.contains(
            "#11 [09-28 14:05] Bob (reply to #10): look at this [video \"a.mp4\" 1.0 MB] <https://x.y>\n"
        ));
        assert!(
            !text.contains("Quiet"),
            "chats without messages are left out"
        );
        let empty = format_digest(&[], 6, &|_| String::new());
        assert!(empty.contains("Nothing new"));
    }

    #[test]
    fn a_long_digest_is_cut_on_a_char_boundary() {
        let s = "é".repeat(MAX_CONTEXT_CHARS);
        let out = cut(s, MAX_CONTEXT_CHARS);
        assert!(out.ends_with("shorter period)\n"));
    }

    #[test]
    fn every_tool_has_an_arm_and_writes_are_destructive() {
        let names = all_names();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
        let src = include_str!("tools.rs");
        let body = &src[src.find("async fn run(").unwrap()..];
        for e in entries() {
            assert!(e.name.starts_with("tg_"));
            assert_eq!(e.input_schema["type"], "object", "{}", e.name);
            assert!(
                body.contains(&format!("\"{}\" =>", e.name)),
                "{} has no arm",
                e.name
            );
            let write = WRITE_TOOLS.contains(&e.name);
            assert_eq!(e.risk == Some(ToolRisk::Destructive), write, "{}", e.name);
        }
        for n in WRITE_TOOLS.iter().chain(LOCAL_TOOLS) {
            assert!(names.contains(n), "{n}");
        }
    }
}
