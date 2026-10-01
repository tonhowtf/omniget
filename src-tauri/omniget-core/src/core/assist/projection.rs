//! Scoped MCP projection of the assistant tools for CLI/ACP runtimes
//! (spec 02 "Capacidade efetiva"). Owner: worker W4.
//!
//! A runtime that runs its own tool loop (Claude Code, an ACP agent) cannot
//! reach the broker, so the coordinator issues it a **per-session token**
//! before the turn: the token's hash is stored here with the bot, the
//! conversation, the scopes `ctx::resolve` gave that pair, the tools the
//! bot's effective grants allow in that turn and an expiry. The local bridge
//! serves `POST /mcp/assist` (bearer = the token) with `initialize`,
//! `tools/list` and `tools/call`, answering through
//! `assist::tools::call_with_ctx`. No valid token → 401 and nothing else.
//!
//! The scope set is fixed at issue time in the backend; nothing the model
//! sends can widen it (spec A20). The token itself is only ever in memory,
//! in the child's MCP config file (0600, deleted after the turn) or in the
//! ACP `session/new` request — never in logs, events or the database.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ctx::{AssistCtx, Scope};
use super::db::{AssistDb, Migration};
use super::tools::{AssistToolset, EmptyToolset};
use crate::core::llm::agent::GrantMode;
use crate::core::llm::broker::{Answer, AskEnd, ToolBroker};
use crate::core::llm::types::ToolSpec;

pub const ERR_PROJECTION: &str = "ERR_PROJECTION";
/// MCP server name the runtimes see (`mcp__omniget__<tool>` in Claude Code).
pub const SERVER_NAME: &str = "omniget";
/// Tool Claude Code calls to ask a permission (`--permission-prompt-tool`).
pub const PERMISSION_TOOL: &str = "omniget_permission";
/// Protocol version answered to `initialize` when the client sends none.
pub const PROTOCOL: &str = "2025-06-18";
/// Prefix of the bots' projected skill tools (`bots::skills::SKILL_SOURCE`).
pub const SKILL_PREFIX: &str = "skill__";
/// Default life of a token: one long turn.
pub const DEFAULT_TTL_MS: i64 = 6 * 60 * 60 * 1000;

pub const MIGRATIONS: &[Migration] = &[Migration {
    module: "projection",
    version: 1,
    sql: "
CREATE TABLE projection_tokens (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    bot_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    run_id TEXT,
    readable TEXT NOT NULL,
    writable TEXT NOT NULL,
    tools TEXT NOT NULL,
    permission_prompt INTEGER NOT NULL DEFAULT 0,
    created_ms INTEGER NOT NULL,
    expires_ms INTEGER NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX projection_tokens_expiry ON projection_tokens(expires_ms);
",
}];

pub fn toolset() -> Arc<dyn AssistToolset> {
    Arc::new(EmptyToolset("projection"))
}

/// What one valid token allows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    pub bot_id: String,
    pub conversation_id: String,
    pub run_id: Option<String>,
    pub readable: Vec<Scope>,
    pub writable: Vec<Scope>,
    /// Tool name → grant mode of the bot's effective agent (never `Deny`).
    pub tools: Vec<(String, GrantMode)>,
    pub permission_prompt: bool,
    pub expires_ms: i64,
}

impl Grant {
    pub fn ctx(&self) -> AssistCtx {
        if super::authority::external(&self.conversation_id) {
            let mut ctx = super::authority::context(&self.conversation_id, &self.bot_id);
            ctx.readable.retain(|s| self.readable.contains(s));
            ctx.writable.retain(|s| self.writable.contains(s));
            ctx.run_id = self.run_id.clone();
            return ctx;
        }
        AssistCtx {
            principal: super::ctx::LOCAL_USER.into(),
            bot_id: Some(self.bot_id.clone()),
            conversation_id: Some(self.conversation_id.clone()),
            run_id: self.run_id.clone(),
            readable: self.readable.clone(),
            writable: self.writable.clone(),
        }
    }

    pub fn mode(&self, tool: &str) -> Option<GrantMode> {
        self.tools.iter().find(|(n, _)| n == tool).map(|(_, m)| *m)
    }
}

/// A freshly issued token. `token` is the secret: hand it to the runtime,
/// never log it.
#[derive(Clone)]
pub struct Issued {
    pub id: String,
    pub token: String,
    pub expires_ms: i64,
}

impl std::fmt::Debug for Issued {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Issued")
            .field("id", &self.id)
            .field("token", &"<redacted>")
            .finish()
    }
}

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{ERR_PROJECTION}: {e}")
}

fn hash(token: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn new_secret() -> String {
    use base64::Engine;
    let mut buf = [0u8; 32];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

fn mode_str(m: GrantMode) -> &'static str {
    match m {
        GrantMode::Auto => "auto",
        GrantMode::Ask => "ask",
        GrantMode::Deny => "deny",
    }
}

fn mode_parse(s: &str) -> GrantMode {
    match s {
        "auto" => GrantMode::Auto,
        "ask" => GrantMode::Ask,
        _ => GrantMode::Deny,
    }
}

fn tools_json(tools: &[(String, GrantMode)]) -> String {
    let map: serde_json::Map<String, Value> = tools
        .iter()
        .filter(|(_, m)| !matches!(m, GrantMode::Deny))
        .map(|(n, m)| (n.clone(), Value::String(mode_str(*m).into())))
        .collect();
    Value::Object(map).to_string()
}

// ── In-memory side: the broker that asks, and where the bridge listens ────

struct Live {
    broker: Arc<ToolBroker>,
}

static LIVE: RwLock<Option<HashMap<String, Live>>> = RwLock::new(None);
static ENDPOINT: RwLock<Option<String>> = RwLock::new(None);

/// The bridge sets this when it binds (`http://127.0.0.1:<port>/mcp/assist`).
pub fn set_endpoint(url: Option<String>) {
    *ENDPOINT.write().unwrap_or_else(|e| e.into_inner()) = url;
}

/// Where runtimes reach the projection; `None` when the bridge is off.
pub fn endpoint() -> Option<String> {
    ENDPOINT.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Ties an issued token to the broker that asks the user for `Ask` grants
/// and permission prompts.
pub fn attach_broker(grant_id: &str, broker: Arc<ToolBroker>) {
    let mut g = LIVE.write().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new)
        .insert(grant_id.to_string(), Live { broker });
}

fn broker_of(grant_id: &str) -> Option<Arc<ToolBroker>> {
    LIVE.read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(grant_id).map(|l| l.broker.clone()))
}

/// The projection of the running turn, handed from the coordinator to the
/// CLI runtime (which has no broker of its own).
#[derive(Clone)]
pub struct TurnProjection {
    pub grant_id: String,
    pub url: String,
    pub token: String,
}

impl std::fmt::Debug for TurnProjection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnProjection")
            .field("grant_id", &self.grant_id)
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .finish()
    }
}

tokio::task_local! {
    static TURN_PROJECTION: TurnProjection;
}

pub async fn scope_turn<F: std::future::Future>(p: Option<TurnProjection>, fut: F) -> F::Output {
    match p {
        Some(p) => TURN_PROJECTION.scope(p, fut).await,
        None => fut.await,
    }
}

pub fn current_turn() -> Option<TurnProjection> {
    TURN_PROJECTION.try_with(Clone::clone).ok()
}

/// Tool name → mode for every assistant tool the agent's grants allow, plus
/// the bot's projected skills (`skill__<name>_<hash8>`, registered in the
/// broker by `bots::skills`).
pub fn granted_tools(
    broker: &ToolBroker,
    grants: &[crate::core::llm::agent::ToolGrant],
) -> Vec<(String, GrantMode)> {
    let mut names = super::tools::names();
    for spec in broker.specs() {
        if spec.name.starts_with(SKILL_PREFIX) && !names.contains(&spec.name) {
            names.push(spec.name);
        }
    }
    names
        .into_iter()
        .map(|n| {
            let m = broker.mode_for(grants, &n);
            (n, m)
        })
        .filter(|(_, m)| !matches!(m, GrantMode::Deny))
        .collect()
}

/// [`granted_tools`] for an external mission run on a CLI: also the file
/// tools of the external grant (served by the projection through
/// `authority::execute_files`, never by the CLI's own built-ins).
pub fn granted_tools_external(
    broker: &ToolBroker,
    grants: &[crate::core::llm::agent::ToolGrant],
) -> Vec<(String, GrantMode)> {
    let mut out = granted_tools(broker, grants);
    for g in grants {
        if let crate::core::llm::agent::ToolSource::Internal { name } = &g.source {
            if name.starts_with("fs_")
                && !matches!(g.mode, GrantMode::Deny)
                && !out.iter().any(|(n, _)| n == name)
            {
                out.push((name.clone(), g.mode));
            }
        }
    }
    out
}

fn external_fs_spec(grant: &Grant, name: &str) -> Option<ToolSpec> {
    if let Some((d, schema)) = super::external_files::external_spec(name) {
        return Some(ToolSpec {
            name: name.into(),
            description: d.into(),
            input_schema: schema,
        });
    }
    broker_of(&grant.id).and_then(|b| b.specs().into_iter().find(|s| s.name == name))
}

// ── Tokens ────────────────────────────────────────────────────────────────

/// Issues a token for `bot` in `conversation`. Scopes come from
/// `ctx::resolve` (never from the caller); `tools` are the effective grants
/// of this turn (`Deny` entries are dropped).
pub fn issue(
    db: &AssistDb,
    bot: &str,
    conversation: &str,
    run_id: Option<&str>,
    tools: &[(String, GrantMode)],
    permission_prompt: bool,
    ttl_ms: i64,
) -> Result<Issued, String> {
    let ctx = super::ctx::resolve(bot, Some(conversation));
    let token = new_secret();
    let id = format!("proj-{}", super::new_id());
    let now = super::now_ms();
    let expires = now + ttl_ms.max(1);
    db.with(|c| {
        // Expired tokens are useless: drop them while we are here.
        c.execute(
            "DELETE FROM projection_tokens WHERE expires_ms < ?1 OR revoked = 1",
            params![now - 60_000],
        )?;
        c.execute(
            "INSERT INTO projection_tokens (id, token_hash, bot_id, conversation_id, run_id, readable, writable, tools, permission_prompt, created_ms, expires_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                id,
                hash(&token),
                bot,
                conversation,
                run_id,
                serde_json::to_string(&ctx.readable).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&ctx.writable).unwrap_or_else(|_| "[]".into()),
                tools_json(tools),
                permission_prompt as i64,
                now,
                expires
            ],
        )
    })?;
    Ok(Issued {
        id,
        token,
        expires_ms: expires,
    })
}

/// A live ACP session keeps its token across turns; each turn refreshes the
/// tools (the effective grants of that turn) and the run.
pub fn refresh(
    db: &AssistDb,
    id: &str,
    run_id: Option<&str>,
    tools: &[(String, GrantMode)],
    ttl_ms: i64,
) -> Result<(), String> {
    let now = super::now_ms();
    db.with(|c| {
        c.execute(
            "UPDATE projection_tokens SET run_id=?2, tools=?3, expires_ms=?4 WHERE id=?1 AND revoked=0",
            params![id, run_id, tools_json(tools), now + ttl_ms.max(1)],
        )
        .map(|_| ())
    })
}

pub fn revoke(db: &AssistDb, id: &str) {
    let _ = db.with(|c| {
        c.execute(
            "UPDATE projection_tokens SET revoked=1 WHERE id=?1",
            params![id],
        )
    });
    if let Some(m) = LIVE.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(id);
    }
}

/// The grant behind a bearer token, or `None` (unknown, revoked, expired).
pub fn validate(db: &AssistDb, token: &str, now_ms: i64) -> Option<Grant> {
    if token.trim().is_empty() {
        return None;
    }
    let h = hash(token.trim());
    let row = db
        .with(|c| {
            c.query_row(
                "SELECT id, bot_id, conversation_id, run_id, readable, writable, tools, permission_prompt, expires_ms, revoked
                 FROM projection_tokens WHERE token_hash=?1",
                params![h],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, i64>(7)?,
                        r.get::<_, i64>(8)?,
                        r.get::<_, i64>(9)?,
                    ))
                },
            )
            .optional()
        })
        .ok()
        .flatten()?;
    let (id, bot, conv, run, readable, writable, tools, prompt, expires, revoked) = row;
    if revoked != 0 || now_ms >= expires {
        return None;
    }
    if super::authority::external(&conv) && super::authority::resolve(db, &conv, &bot).is_err() {
        return None;
    }
    let tools: serde_json::Map<String, Value> = serde_json::from_str(&tools).unwrap_or_default();
    Some(Grant {
        id,
        bot_id: bot.clone(),
        conversation_id: conv.clone(),
        run_id: run,
        readable: serde_json::from_str(&readable).unwrap_or_default(),
        writable: serde_json::from_str(&writable).unwrap_or_default(),
        tools: tools
            .into_iter()
            .map(|(k, v)| (k, mode_parse(v.as_str().unwrap_or(""))))
            .filter(|(_, m)| !matches!(m, GrantMode::Deny))
            .filter(|(name, _)| super::authority::check_tool(&conv, &bot, name).is_ok())
            .collect(),
        permission_prompt: prompt != 0,
        expires_ms: expires,
    })
}

// ── Runtime-side config ───────────────────────────────────────────────────

/// `--mcp-config` body for Claude Code.
pub fn claude_mcp_config(url: &str, token: &str) -> Value {
    json!({
        "mcpServers": {
            SERVER_NAME: {
                "type": "http",
                "url": url,
                "headers": { "Authorization": format!("Bearer {token}") }
            }
        }
    })
}

/// ACP `session/new` `mcpServers` entry (HTTP transport).
pub fn acp_mcp_server(url: &str, token: &str) -> Value {
    json!({
        "type": "http",
        "name": SERVER_NAME,
        "url": url,
        "headers": [{ "name": "Authorization", "value": format!("Bearer {token}") }]
    })
}

/// Writes the Claude Code MCP config to a private file (0600) under `dir`.
/// The caller deletes it when the turn ends.
pub fn write_mcp_config(dir: &Path, url: &str, token: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(err)?;
    let path = dir.join(format!("mcp-{}.json", super::new_id()));
    let body = claude_mcp_config(url, token).to_string();
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(err)?;
        f.write_all(body.as_bytes()).map_err(err)?;
    }
    #[cfg(not(unix))]
    std::fs::write(&path, body).map_err(err)?;
    Ok(path)
}

// ── JSON-RPC ──────────────────────────────────────────────────────────────

/// Why the bridge answers 401.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unauthorized;

fn rpc_ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_err(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn permission_spec() -> Value {
    json!({
        "name": PERMISSION_TOOL,
        "description": "Asks the OmniGet user to approve a tool call. Used by the runtime, not by the model.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool_name": { "type": "string" },
                "input": { "type": "object" },
                "tool_use_id": { "type": "string" }
            },
            "required": ["tool_name", "input"]
        }
    })
}

fn listed(grant: &Grant) -> Vec<Value> {
    let mut specs = super::tools::all_specs();
    // Skill tools live in the broker's `skill__` source.
    if grant.tools.iter().any(|(n, _)| n.starts_with(SKILL_PREFIX)) {
        if let Some(broker) = broker_of(&grant.id) {
            specs.extend(
                broker
                    .specs()
                    .into_iter()
                    .filter(|s| s.name.starts_with(SKILL_PREFIX)),
            );
        }
    }
    if super::authority::external(&grant.conversation_id) {
        for (name, _) in grant.tools.iter().filter(|(n, _)| n.starts_with("fs_")) {
            if let Some(spec) = external_fs_spec(grant, name) {
                specs.push(spec);
            }
        }
    }
    let mut out: Vec<Value> = specs
        .into_iter()
        .filter(|s| grant.mode(&s.name).is_some())
        .map(|s| json!({ "name": s.name, "description": s.description, "inputSchema": s.input_schema }))
        .collect();
    if grant.permission_prompt {
        out.push(permission_spec());
    }
    out
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// One JSON-RPC message (or a batch) for the projection. `Err(Unauthorized)`
/// → HTTP 401; `Ok(None)` → 202 (notification).
pub async fn handle(
    db: &AssistDb,
    bearer: Option<&str>,
    msg: &Value,
) -> Result<Option<Value>, Unauthorized> {
    let grant = bearer
        .and_then(|t| validate(db, t, super::now_ms()))
        .ok_or(Unauthorized)?;
    if let Some(batch) = msg.as_array() {
        let mut out = Vec::new();
        for m in batch {
            if let Some(r) = handle_one(&grant, m).await {
                out.push(r);
            }
        }
        return Ok((!out.is_empty()).then_some(Value::Array(out)));
    }
    Ok(handle_one(&grant, msg).await)
}

async fn handle_one(grant: &Grant, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned().filter(|v| !v.is_null());
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let Some(id) = id else {
        // Notifications (`notifications/initialized`, cancels): nothing to say.
        return None;
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => rpc_ok(
            &id,
            json!({
                "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL),
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "omniget-assist", "version": env!("CARGO_PKG_VERSION") },
            }),
        ),
        "ping" => rpc_ok(&id, json!({})),
        "tools/list" => rpc_ok(&id, json!({ "tools": listed(grant) })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            rpc_ok(&id, call(grant, name, args).await)
        }
        other => rpc_err(&id, -32601, &format!("method `{other}` not found")),
    })
}

/// Mission hooks see projected calls too (a hosted CLI's tools never pass
/// through the broker): policy veto before, and the round's call log after,
/// so the progress guard judges what the executor actually did.
async fn call(grant: &Grant, name: &str, args: Value) -> Value {
    if name == PERMISSION_TOOL && grant.permission_prompt {
        return permission_prompt(grant, args).await;
    }
    let conv = grant.conversation_id.as_str();
    if crate::core::assist::missions::hooks::mission_of(conv).is_none() {
        return call_tool(grant, name, args).await;
    }
    if let Some(reason) = crate::core::assist::missions::hooks::before_tool(conv, name, &args) {
        return text_result(reason, true);
    }
    let started = std::time::Instant::now();
    let out = call_tool(grant, name, args.clone()).await;
    let ok = !out.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
    crate::core::assist::missions::hooks::after_tool(conv, name, &args, ok, ms);
    out
}

async fn call_tool(grant: &Grant, name: &str, args: Value) -> Value {
    let Some(mode) = grant.mode(name) else {
        return text_result(
            format!(
                "{}: `{name}` is not granted to this bot here",
                super::tools::ERR_ASSIST_SCOPE
            ),
            true,
        );
    };
    if matches!(mode, GrantMode::Ask) {
        let Some(broker) = broker_of(&grant.id) else {
            return text_result(
                format!("{ERR_PROJECTION}: nobody can approve `{name}` now"),
                true,
            );
        };
        let call_id = format!("proj-{}", super::new_id());
        let end = broker
            .ask_user_ended(
                &grant.bot_id,
                grant.run_id.as_deref().unwrap_or(""),
                &call_id,
                name,
                &crate::core::llm::code_tools::preview(name, &args),
            )
            .await;
        if let AskEnd::Expired { waited } = end {
            let e = crate::core::llm::broker::timeout_error(name, waited);
            return text_result(format!("{}: {}", e.code, e.message), true);
        }
        if matches!(end.answer(), Answer::Deny) {
            return text_result(format!("`{name}` was refused"), true);
        }
    }
    // External file tools: the same checked transaction as a native run
    // (grant, workspace identity, protected names, revision-bound edits).
    if name.starts_with("fs_") && super::authority::external(&grant.conversation_id) {
        let (conv, bot, tool) = (
            grant.conversation_id.clone(),
            grant.bot_id.clone(),
            name.to_string(),
        );
        let out = tokio::task::spawn_blocking(move || {
            super::authority::execute_files(&conv, &bot, &tool, &args)
        })
        .await;
        return match out {
            Ok(Ok(text)) => text_result(text, false),
            Ok(Err(e)) => text_result(e, true),
            Err(_) => text_result("external file executor interrupted".into(), true),
        };
    }
    if name.starts_with(SKILL_PREFIX) {
        return match super::bots::skills::call_projected(&grant.ctx(), name, args).await {
            Ok(text) => text_result(text, false),
            Err(e) => text_result(e, true),
        };
    }
    match super::tools::call_with_ctx(&grant.ctx(), name, args).await {
        Ok(Value::String(s)) => text_result(s, false),
        Ok(v) => text_result(serde_json::to_string(&v).unwrap_or_default(), false),
        Err(e) => text_result(e, true),
    }
}

/// Claude Code's permission prompt, routed to the broker's ask (persisted as
/// a `PermissionRequest` of the run). Answers the documented shape:
/// `{"behavior":"allow","updatedInput":…}` or `{"behavior":"deny","message":…}`.
async fn permission_prompt(grant: &Grant, args: Value) -> Value {
    let tool = args
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("tool")
        .to_string();
    let input = args.get("input").cloned().unwrap_or(json!({}));
    let call_id = args
        .get("tool_use_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("perm-{}", super::new_id()));
    let label = format!("cli:{tool}");
    let preview = match input.get("command").and_then(Value::as_str) {
        Some(cmd) => cmd.to_string(),
        None => input
            .get("file_path")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| input.to_string().chars().take(600).collect()),
    };
    // The same gate as the native broker and the `PreToolUse` hook (3.4):
    // `Bash` is judged as `shell_exec`, `Write` as `fs_write`, so an
    // "Always" or a "Deny" given anywhere counts here.
    let workspace = crate::core::llm::code_tools::workspace_of(&grant.conversation_id);
    let (native, native_input) =
        crate::core::llm::perm::from_claude(&tool, &input, workspace.as_deref());
    let verdict = crate::core::llm::perm::gate(
        &crate::core::llm::perm::rules_of(&grant.bot_id),
        crate::core::llm::run_mode::of(&grant.conversation_id),
        crate::core::llm::perm::cli_grant(&native),
        &native,
        &native_input,
    );
    if matches!(verdict.mode, GrantMode::Deny) {
        let body = json!({ "behavior": "deny", "message": crate::core::llm::claude_hooks::deny_reason(&grant.bot_id, &native, &native_input) });
        return text_result(body.to_string(), false);
    }
    let end = match verdict.mode {
        GrantMode::Auto | GrantMode::Deny => AskEnd::Answered(Answer::Once),
        GrantMode::Ask => match broker_of(&grant.id) {
            Some(broker) => {
                broker
                    .ask_user_ended(
                        &grant.bot_id,
                        grant.run_id.as_deref().unwrap_or(""),
                        &call_id,
                        &label,
                        &preview,
                    )
                    .await
            }
            None => AskEnd::Answered(Answer::Deny),
        },
    };
    // Nobody answered: the agent must not read that as the user's "no"
    // (it would stop trying, or apologise for a refusal that never was).
    if let AskEnd::Expired { waited } = end {
        let e = crate::core::llm::broker::timeout_error(&tool, waited);
        let body = json!({ "behavior": "deny", "message": format!("{}: {}", e.code, e.message) });
        return text_result(body.to_string(), false);
    }
    let answer = end.answer();
    if matches!(answer, Answer::Always) {
        crate::core::llm::perm::add_rule(
            &grant.bot_id,
            crate::core::llm::perm::always_rule(&native, &native_input),
        );
    }
    let body = match answer {
        Answer::Deny => {
            json!({ "behavior": "deny", "message": "The OmniGet user refused this action." })
        }
        _ => json!({ "behavior": "allow", "updatedInput": input }),
    };
    text_result(body.to_string(), false)
}
