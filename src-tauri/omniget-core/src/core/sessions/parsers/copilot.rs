//! GitHub Copilot: CLI e chat do VS Code (estudo 06, Copilot §j; Parte 6 §8).
//!
//! CLI (`${COPILOT_HOME:-~/.copilot}`):
//! - `session-state/<id>/events.jsonl`: envelope `{id, timestamp, parentId,
//!   type, data}`. Tipos lidos: `session.start`, `session.model_change`,
//!   `user.message`, `assistant.message` (`content`, `toolRequests[]`),
//!   `tool.execution_start|complete`, `subagent.*`, `session.shutdown`
//!   (`modelMetrics.<modelo>.usage.{inputTokens, outputTokens,
//!   cacheReadTokens, cacheWriteTokens, reasoningTokens}`). O uso por chamada
//!   (`assistant.usage`) é efêmero e não vai para o disco: sem o
//!   `session-store.db`, o total do `session.shutdown` é lançado no último
//!   turno do assistente com aquele modelo (os totais batem; a distribuição
//!   por turno não).
//! - `session-store.db`: `sessions(id, cwd, repository, branch, summary,
//!   created_at, updated_at)` e `assistant_usage_events(session_id,
//!   turn_index, model, input_tokens, output_tokens, cache_read_tokens,
//!   cache_write_tokens, reasoning_tokens, created_at)`, uso por turno.
//!   `inputTokens` do Copilot inclui cache; aqui ele é normalizado.
//!
//! VS Code: `<User>/workspaceStorage/<hash>/chatSessions/<id>.json` (retrato)
//! ou `.jsonl` (log de mutações `{kind:0,v}` estado inicial, `{kind:1,k,v}`
//! set, `{kind:2,k,v,i?}` push/splice, `{kind:3,k}` delete), mais
//! `globalStorage/emptyWindowChatSessions/`. Verificado em arquivo real nesta
//! máquina só o formato de retrato inicial (sessões vazias); requisições e
//! tokens (`promptTokens`, `result.metadata.{outputTokens, resolvedModel,
//! toolCallRounds}`) seguem a fixture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::util::*;
use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus, Turn,
};

pub struct CopilotSource;

const TOOL: &str = "copilot";

fn copilot_home() -> Option<PathBuf> {
    env_dir("COPILOT_HOME").or_else(|| home_join(&[".copilot"]))
}

fn cli_session_files() -> Vec<PathBuf> {
    let Some(root) = copilot_home().map(|h| h.join("session-state")) else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    rd.flatten()
        .map(|e| e.path().join("events.jsonl"))
        .filter(|p| p.is_file())
        .collect()
}

/// Arquivos de chat do VS Code: (rótulo do editor, arquivo, pasta do projeto).
fn vscode_files() -> Vec<(String, PathBuf, Option<String>)> {
    let mut out = Vec::new();
    for (label, user) in vscode_user_dirs() {
        if label == "Cursor" || label == "cursor-server" {
            continue;
        }
        let ws = user.join("workspaceStorage");
        if let Ok(rd) = std::fs::read_dir(&ws) {
            for e in rd.flatten() {
                let dir = e.path().join("chatSessions");
                if !dir.is_dir() {
                    continue;
                }
                let folder = workspace_folder(&e.path());
                for f in find_files(&dir, 1, is_chat_file) {
                    out.push((label.clone(), f, folder.clone()));
                }
            }
        }
        for sub in ["emptyWindowChatSessions", "transferredChatSessions"] {
            let dir = user.join("globalStorage").join(sub);
            for f in find_files(&dir, 1, is_chat_file) {
                out.push((label.clone(), f, None));
            }
        }
    }
    out
}

fn is_chat_file(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()),
        Some("json" | "jsonl")
    )
}

// ------------------------------------------------------------------ CLI

struct StoreRow {
    turn_index: i64,
    model: Option<String>,
    usage: TokenUsage,
    ts: Option<String>,
}

#[derive(Default)]
struct StoreSession {
    cwd: Option<String>,
    branch: Option<String>,
    summary: Option<String>,
    rows: Vec<StoreRow>,
}

fn session_store(id: &str) -> Option<StoreSession> {
    let db = copilot_home()?.join("session-store.db");
    let conn = open_ro(&db)?;
    let mut out = StoreSession::default();
    if table_exists(&conn, "sessions") {
        let cols = table_columns(&conn, "sessions");
        let pick = |c: &str| {
            if cols.iter().any(|x| x == c) {
                c.to_string()
            } else {
                "NULL".into()
            }
        };
        let sql = format!(
            "SELECT {}, {}, {} FROM sessions WHERE id = ?1",
            pick("cwd"),
            pick("branch"),
            pick("summary")
        );
        if let Ok((cwd, branch, summary)) = conn.query_row(&sql, [id], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        }) {
            out.cwd = cwd;
            out.branch = branch;
            out.summary = summary;
        }
    }
    if table_exists(&conn, "assistant_usage_events") {
        let cols = table_columns(&conn, "assistant_usage_events");
        let pick = |c: &str| {
            if cols.iter().any(|x| x == c) {
                c.to_string()
            } else {
                "NULL".into()
            }
        };
        let sql = format!(
            "SELECT {}, COALESCE({}, {}), {}, {}, {}, {}, {}, {} FROM assistant_usage_events \
             WHERE session_id = ?1 ORDER BY rowid",
            pick("turn_index"),
            pick("copilot_usage_model"),
            pick("model"),
            pick("input_tokens"),
            pick("output_tokens"),
            pick("cache_read_tokens"),
            pick("cache_write_tokens"),
            pick("reasoning_tokens"),
            pick("created_at"),
        );
        if let Ok(mut st) = conn.prepare(&sql) {
            if let Ok(rows) = st.query_map([id], |r| {
                let n = |i: usize| -> u64 {
                    r.get::<_, Option<i64>>(i)
                        .ok()
                        .flatten()
                        .unwrap_or(0)
                        .max(0) as u64
                };
                let cache_read = n(5);
                let cache_write = n(6);
                Ok(StoreRow {
                    turn_index: r.get::<_, Option<i64>>(0).ok().flatten().unwrap_or(-1),
                    model: r.get::<_, Option<String>>(1).ok().flatten(),
                    usage: TokenUsage {
                        input: n(3).saturating_sub(cache_read + cache_write),
                        output: n(4),
                        cache_read,
                        cache_write,
                        reasoning: n(7),
                    },
                    ts: r
                        .get::<_, Option<String>>(8)
                        .ok()
                        .flatten()
                        .and_then(|s| str_to_ms(&s))
                        .map(ms_to_rfc3339),
                })
            }) {
                out.rows = rows.flatten().collect();
            }
        }
    }
    Some(out)
}

/// Parser puro do `events.jsonl` do Copilot CLI.
pub fn parse_cli(events: &[Value], id: &str, source: &Path) -> Session {
    let mut meta = new_meta(TOOL, id, source);
    meta.account = Some("cli".into());
    let mut turns: Vec<Turn> = Vec::new();
    let mut model: Option<String> = None;
    let mut results: Vec<(String, String, bool)> = Vec::new();
    let mut shutdown: Option<Value> = None;
    for ev in events {
        let ty = ev.get("type").and_then(Value::as_str).unwrap_or("");
        let data = ev.get("data").cloned().unwrap_or(Value::Null);
        let ts = ev.get("timestamp").and_then(ts_str).unwrap_or_default();
        match ty {
            "session.start" | "session.resume" => {
                if meta.started.is_none() && !ts.is_empty() {
                    meta.started = Some(ts.clone());
                }
                let ctx = data.get("context").cloned().unwrap_or(Value::Null);
                meta.project_path = meta
                    .project_path
                    .clone()
                    .or_else(|| s(&ctx, &["cwd", "gitRoot"]))
                    .or_else(|| s(&data, &["cwd", "workingDirectory"]));
                meta.git_branch = meta.git_branch.clone().or_else(|| s(&ctx, &["branch"]));
                if let Some(m) = s(&data, &["selectedModel", "model", "currentModel"]) {
                    model = Some(m);
                }
            }
            "session.model_change" => {
                if let Some(m) = s(&data, &["newModel", "model"]) {
                    model = Some(m);
                }
            }
            "user.message" => {
                let text = s(&data, &["content", "text"]).unwrap_or_default();
                let mut t = turn(Role::User, ts, text);
                t.message_id = s(ev, &["id"]);
                turns.push(t);
            }
            "assistant.message" => {
                let text = data.get("content").map(content_text).unwrap_or_default();
                let mut calls = Vec::new();
                for r in data
                    .get("toolRequests")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let name = s(r, &["name", "toolName"]).unwrap_or_else(|| "unknown".into());
                    let args = r.get("arguments").cloned().unwrap_or(Value::Null);
                    let args = match args {
                        Value::String(ref st) => serde_json::from_str(st).unwrap_or(args),
                        other => other,
                    };
                    calls.push(tool_call(
                        s(r, &["toolCallId", "id"]).unwrap_or_default(),
                        &name,
                        args,
                    ));
                }
                let m = s(&data, &["model"]).or_else(|| model.clone());
                let merge = matches!(turns.last(), Some(t) if t.role == Role::Assistant && t.message_id == s(&data, &["messageId"]) && t.message_id.is_some());
                if merge {
                    let t = turns.last_mut().expect("existe");
                    t.text.push_str(&text);
                    t.tool_calls.extend(calls);
                } else {
                    let mut t = turn(Role::Assistant, ts, text);
                    t.tool_calls = calls;
                    t.model = m;
                    t.message_id = s(&data, &["messageId"]).or_else(|| s(ev, &["id"]));
                    turns.push(t);
                }
            }
            "tool.execution_complete" => {
                let id = s(&data, &["toolCallId"]).unwrap_or_default();
                let ok = data.get("success").and_then(Value::as_bool).unwrap_or(true);
                let body = data
                    .pointer("/result/content")
                    .or_else(|| data.get("result"))
                    .map(result_text)
                    .or_else(|| data.pointer("/error/message").map(result_text))
                    .unwrap_or_default();
                results.push((id, body, !ok));
            }
            "subagent.started" => {
                // Chamada `task` já registrada no assistant.message; só anota o tipo.
                if let (Some(id), Some(name)) = (
                    s(&data, &["toolCallId"]),
                    s(&data, &["agentName", "agentDisplayName", "name"]),
                ) {
                    for t in turns.iter_mut().rev() {
                        if let Some(c) = t.tool_calls.iter_mut().find(|c| c.id == id) {
                            c.subagent = Some(name.clone());
                            break;
                        }
                    }
                }
            }
            "session.shutdown" => {
                shutdown = Some(data);
                if !ts.is_empty() {
                    meta.ended = Some(ts);
                }
            }
            _ => {}
        }
    }
    attach_results(&mut turns, results);

    let store = session_store(id).unwrap_or_default();
    if let Some(c) = store.cwd {
        meta.project_path = Some(c);
    }
    if store.branch.is_some() {
        meta.git_branch = store.branch;
    }
    meta.title = store.summary.filter(|x| !x.trim().is_empty());
    if !store.rows.is_empty() {
        let assistant_idx: Vec<usize> = turns
            .iter()
            .enumerate()
            .filter(|(_, t)| t.role == Role::Assistant)
            .map(|(i, _)| i)
            .collect();
        let user_idx: Vec<usize> = turns
            .iter()
            .enumerate()
            .filter(|(_, t)| t.role == Role::User)
            .map(|(i, _)| i)
            .collect();
        for r in store.rows {
            // turn_index conta interações do usuário; lança no último turno do
            // assistente daquela interação.
            let target = if r.turn_index >= 0 {
                let ui = r.turn_index as usize;
                let start = user_idx.get(ui).copied().unwrap_or(0);
                let end = user_idx.get(ui + 1).copied().unwrap_or(turns.len());
                assistant_idx
                    .iter()
                    .rev()
                    .find(|&&i| i > start && i < end)
                    .copied()
            } else {
                None
            }
            .or(assistant_idx.last().copied());
            match target {
                Some(i) => {
                    turns[i].usage.add(&r.usage);
                    if turns[i].model.is_none() {
                        turns[i].model = r.model.clone();
                    }
                }
                None => {
                    let mut t = turn(Role::Assistant, r.ts.unwrap_or_default(), "");
                    t.usage = r.usage;
                    t.model = r.model;
                    turns.push(t);
                }
            }
        }
    } else if let Some(sd) = shutdown {
        if let Some(Value::Object(mm)) = sd.get("modelMetrics") {
            for (m, metrics) in mm {
                let us = metrics.get("usage").cloned().unwrap_or(Value::Null);
                let cache_read = u(&us, &["cacheReadTokens"]);
                let cache_write = u(&us, &["cacheWriteTokens"]);
                let usage = TokenUsage {
                    input: u(&us, &["inputTokens"]).saturating_sub(cache_read + cache_write),
                    output: u(&us, &["outputTokens"]),
                    cache_read,
                    cache_write,
                    reasoning: u(&us, &["reasoningTokens"]),
                };
                let idx = turns
                    .iter()
                    .rposition(|t| {
                        t.role == Role::Assistant && t.model.as_deref() == Some(m.as_str())
                    })
                    .or_else(|| turns.iter().rposition(|t| t.role == Role::Assistant));
                match idx {
                    Some(i) => {
                        turns[i].usage.add(&usage);
                        if turns[i].model.is_none() {
                            turns[i].model = Some(m.clone());
                        }
                    }
                    None => {
                        let mut t =
                            turn(Role::Assistant, meta.ended.clone().unwrap_or_default(), "");
                        t.usage = usage;
                        t.model = Some(m.clone());
                        turns.push(t);
                    }
                }
            }
        }
    }
    finalize(&mut meta, &mut turns);
    Session { meta, turns }
}

// ------------------------------------------------------------------ VS Code

fn path_key(k: &Value) -> Option<PathKey> {
    match k {
        Value::String(s) => Some(PathKey::Key(s.clone())),
        Value::Number(n) => n.as_u64().map(|i| PathKey::Idx(i as usize)),
        _ => None,
    }
}

enum PathKey {
    Key(String),
    Idx(usize),
}

fn walk_mut<'a>(root: &'a mut Value, path: &[Value], create: bool) -> Option<&'a mut Value> {
    let mut cur = root;
    for k in path {
        let key = path_key(k)?;
        cur = match key {
            PathKey::Key(s) => {
                if !cur.is_object() {
                    if !create {
                        return None;
                    }
                    *cur = Value::Object(Map::new());
                }
                let o = cur.as_object_mut()?;
                if !o.contains_key(&s) {
                    if !create {
                        return None;
                    }
                    o.insert(s.clone(), Value::Null);
                }
                o.get_mut(&s)?
            }
            PathKey::Idx(i) => cur.as_array_mut()?.get_mut(i)?,
        };
    }
    Some(cur)
}

/// Reaplica o log de mutações do `chatSessions/*.jsonl` do VS Code.
pub fn replay_mutations(lines: &[Value]) -> Value {
    let mut state = Value::Null;
    for l in lines {
        let kind = l.get("kind").and_then(as_u64).unwrap_or(99);
        let k: Vec<Value> = l
            .get("k")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let v = l.get("v").cloned().unwrap_or(Value::Null);
        match kind {
            0 => state = v,
            1 => {
                if k.is_empty() {
                    state = v;
                    continue;
                }
                let (last, parent) = k.split_last().expect("não vazio");
                if let Some(p) = walk_mut(&mut state, parent, true) {
                    match path_key(last) {
                        Some(PathKey::Key(s)) => {
                            if !p.is_object() {
                                *p = Value::Object(Map::new());
                            }
                            if let Some(o) = p.as_object_mut() {
                                o.insert(s, v);
                            }
                        }
                        Some(PathKey::Idx(i)) => {
                            if let Some(a) = p.as_array_mut() {
                                if i < a.len() {
                                    a[i] = v;
                                } else if i == a.len() {
                                    a.push(v);
                                }
                            }
                        }
                        None => {}
                    }
                }
            }
            2 => {
                if let Some(target) = walk_mut(&mut state, &k, true) {
                    if !target.is_array() {
                        *target = Value::Array(Vec::new());
                    }
                    if let Some(a) = target.as_array_mut() {
                        if let Some(i) = l.get("i").and_then(as_u64) {
                            a.truncate(i as usize);
                        }
                        match v {
                            Value::Array(items) => a.extend(items),
                            Value::Null => {}
                            other => a.push(other),
                        }
                    }
                }
            }
            3 => {
                if let Some((last, parent)) = k.split_last() {
                    if let Some(p) = walk_mut(&mut state, parent, false) {
                        match path_key(last) {
                            Some(PathKey::Key(s)) => {
                                if let Some(o) = p.as_object_mut() {
                                    o.remove(&s);
                                }
                            }
                            Some(PathKey::Idx(i)) => {
                                if let Some(a) = p.as_array_mut() {
                                    if i < a.len() {
                                        a.remove(i);
                                    }
                                }
                            }
                            None => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    state
}

fn vscode_state(f: &Path) -> Option<Value> {
    if f.extension().and_then(|e| e.to_str()) == Some("jsonl") {
        let lines = read_jsonl(f);
        if lines.is_empty() {
            return None;
        }
        Some(replay_mutations(&lines))
    } else {
        read_json(f)
    }
}

fn response_text(resp: &Value) -> String {
    let mut parts = Vec::new();
    for p in resp.as_array().into_iter().flatten() {
        let kind = p.get("kind").and_then(Value::as_str);
        match kind {
            None | Some("markdownContent") | Some("markdownVuln") => {
                let v = p.get("value");
                let txt = match v {
                    Some(Value::String(s)) => s.clone(),
                    Some(o @ Value::Object(_)) => s(o, &["value"]).unwrap_or_default(),
                    _ => String::new(),
                };
                if !txt.is_empty() {
                    parts.push(txt);
                }
            }
            _ => {}
        }
    }
    parts.join("")
}

/// Estado do chat do VS Code em sessão.
pub fn parse_vscode(
    state: &Value,
    fallback_id: &str,
    source: &Path,
    account: &str,
    folder: Option<String>,
) -> Option<Session> {
    let id = s(state, &["sessionId"]).unwrap_or_else(|| fallback_id.to_string());
    let mut meta = new_meta(TOOL, id, source);
    meta.account = Some(account.to_string());
    meta.project_path = folder;
    meta.title = s(state, &["customTitle", "title"]);
    meta.started = ts_field(state, &["creationDate"]).map(ms_to_rfc3339);
    meta.ended = ts_field(state, &["lastMessageDate"]).map(ms_to_rfc3339);
    let mut turns = Vec::new();
    for req in state
        .get("requests")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let ts = req.get("timestamp").and_then(ts_str).unwrap_or_default();
        let user_text = req
            .pointer("/message/text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| s(req, &["message"]))
            .unwrap_or_default();
        let mut ut = turn(Role::User, ts.clone(), user_text);
        ut.message_id = s(req, &["requestId"]);
        turns.push(ut);

        let md = req
            .pointer("/result/metadata")
            .cloned()
            .unwrap_or(Value::Null);
        let mut at = turn(
            Role::Assistant,
            ts,
            response_text(req.get("response").unwrap_or(&Value::Null)),
        );
        at.message_id = s(&md, &["responseId"]).or_else(|| s(req, &["requestId"]));
        at.model = s(&md, &["resolvedModel"])
            .or_else(|| s(req, &["modelId"]).map(|m| m.trim_start_matches("copilot/").to_string()));
        at.usage.input = {
            let a = u(req, &["promptTokens"]);
            if a > 0 {
                a
            } else {
                u(&md, &["promptTokens"])
            }
        };
        at.usage.output = {
            let a = u(req, &["completionTokens"]);
            if a > 0 {
                a
            } else {
                u(&md, &["outputTokens", "completionTokens"])
            }
        };
        let mut results = HashMap::new();
        if let Some(Value::Object(r)) = md.get("toolCallResults") {
            for (k, v) in r {
                results.insert(k.clone(), result_text(v.get("content").unwrap_or(v)));
            }
        }
        for round in md
            .get("toolCallRounds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            at.usage.reasoning += round
                .pointer("/thinking/tokens")
                .and_then(as_u64)
                .unwrap_or(0);
            for tc in round
                .get("toolCalls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let name = s(tc, &["name"]).unwrap_or_else(|| "unknown".into());
                let args = tc.get("arguments").cloned().unwrap_or(Value::Null);
                let args = match args {
                    Value::String(ref st) => serde_json::from_str(st).unwrap_or(args),
                    other => other,
                };
                let cid = s(tc, &["id"]).unwrap_or_default();
                let mut c = tool_call(cid.clone(), &name, args);
                if let Some(r) = results.get(&cid) {
                    c.result = Some(r.clone());
                    c.status = ToolStatus::Ok;
                }
                at.tool_calls.push(c);
            }
        }
        if at.tool_calls.is_empty() {
            for p in req
                .get("response")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if p.get("kind").and_then(Value::as_str) != Some("toolInvocationSerialized") {
                    continue;
                }
                let name = s(p, &["toolId"]).unwrap_or_else(|| "unknown".into());
                let input = p
                    .get("toolSpecificData")
                    .cloned()
                    .unwrap_or_else(|| p.get("invocationMessage").cloned().unwrap_or(Value::Null));
                let mut c = tool_call(s(p, &["toolCallId"]).unwrap_or_default(), &name, input);
                c.status = if p
                    .get("isComplete")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    ToolStatus::Ok
                } else {
                    ToolStatus::Pending
                };
                at.tool_calls.push(c);
            }
        }
        if req
            .get("isCanceled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && at.text.is_empty()
        {
            at.text = "(cancelado)".into();
        }
        turns.push(at);
    }
    if turns.is_empty() {
        return None;
    }
    finalize(&mut meta, &mut turns);
    Some(Session { meta, turns })
}

impl SessionSource for CopilotSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Some(h) = copilot_home() {
            if h.is_dir() {
                v.push(h);
            }
        }
        for (label, user) in vscode_user_dirs() {
            if label != "Cursor" && label != "cursor-server" {
                v.push(user.join("workspaceStorage"));
            }
        }
        v
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out = Vec::new();
        for f in cli_session_files() {
            let id = parent_name(&f);
            let s = parse_cli(&read_jsonl(&f), &id, &f);
            if !s.turns.is_empty() {
                out.push(s.meta);
            }
        }
        for (label, f, folder) in vscode_files() {
            let Some(state) = vscode_state(&f) else {
                continue;
            };
            if let Some(s) = parse_vscode(&state, &file_stem(&f), &f, &label, folder) {
                out.push(s.meta);
            }
        }
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        if let Some(f) = cli_session_files()
            .into_iter()
            .find(|f| parent_name(f) == id)
        {
            return Some(parse_cli(&read_jsonl(&f), id, &f));
        }
        for (label, f, folder) in vscode_files() {
            if file_stem(&f) != id {
                continue;
            }
            let state = vscode_state(&f)?;
            return parse_vscode(&state, id, &f, &label, folder);
        }
        None
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        (meta.account.as_deref() == Some("cli"))
            .then(|| format!("copilot --resume {}", shell_quote(&meta.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_fixture() {
        let lines: Vec<Value> = [
            r#"{"id":"e1","timestamp":"2026-09-20T10:00:00Z","type":"session.start","data":{"selectedModel":"gpt-5","context":{"cwd":"/p","branch":"main"}}}"#,
            r#"{"id":"e2","timestamp":"2026-09-20T10:00:01Z","type":"user.message","data":{"content":"liste os arquivos"}}"#,
            r#"{"id":"e3","timestamp":"2026-09-20T10:00:02Z","type":"assistant.message","data":{"messageId":"m1","content":"ok","toolRequests":[{"toolCallId":"c1","name":"bash","arguments":{"command":"ls"}}]}}"#,
            r#"{"id":"e4","timestamp":"2026-09-20T10:00:03Z","type":"tool.execution_complete","data":{"toolCallId":"c1","success":true,"result":{"content":"a.txt"}}}"#,
            r#"{"id":"e5","timestamp":"2026-09-20T10:05:00Z","type":"session.shutdown","data":{"modelMetrics":{"gpt-5":{"requests":{"count":2},"usage":{"inputTokens":1000,"outputTokens":50,"cacheReadTokens":400,"cacheWriteTokens":0,"reasoningTokens":7}}}}}"#,
        ]
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
        let s = parse_cli(
            &lines,
            "fixture-sem-store-xyz",
            Path::new("/x/events.jsonl"),
        );
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.meta.project_path.as_deref(), Some("/p"));
        assert_eq!(s.meta.git_branch.as_deref(), Some("main"));
        let c = &s.turns[1].tool_calls[0];
        assert_eq!(c.name_canonical, "Bash");
        assert_eq!(c.result.as_deref(), Some("a.txt"));
        assert_eq!(s.meta.usage.input, 600);
        assert_eq!(s.meta.usage.cache_read, 400);
        assert_eq!(s.meta.usage.reasoning, 7);
        assert_eq!(s.meta.models, vec!["gpt-5".to_string()]);
        assert_eq!(s.meta.ended.as_deref(), Some("2026-09-20T10:05:00.000Z"));
    }

    #[test]
    fn vscode_mutation_log() {
        let lines: Vec<Value> = [
            r#"{"kind":0,"v":{"version":3,"creationDate":1781772492290,"sessionId":"s1","requests":[]}}"#,
            r#"{"kind":2,"k":["requests"],"v":[{"requestId":"r1","timestamp":1783918310000,"modelId":"copilot/auto","message":{"text":"explique"},"response":[]}]}"#,
            r#"{"kind":2,"k":["requests",0,"response"],"v":[{"kind":"markdownContent","value":{"value":"É assim."}}]}"#,
            r#"{"kind":1,"k":["requests",0,"result"],"v":{"metadata":{"promptTokens":5000,"outputTokens":200,"resolvedModel":"gpt-5.3-codex","toolCallRounds":[{"thinking":{"tokens":88},"toolCalls":[{"id":"t1","name":"copilot_readFile","arguments":"{\"path\":\"a\"}"}]}],"toolCallResults":{"t1":{"content":[{"value":"conteúdo"}]}}}}}"#,
            r#"{"kind":1,"k":["customTitle"],"v":"Explicação"}"#,
        ]
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
        let state = replay_mutations(&lines);
        let s = parse_vscode(
            &state,
            "s1",
            Path::new("/x/s1.jsonl"),
            "Code",
            Some("/proj".into()),
        )
        .unwrap();
        assert_eq!(s.meta.title.as_deref(), Some("Explicação"));
        assert_eq!(s.turns[1].text, "É assim.");
        assert_eq!(s.turns[1].usage.input, 5000);
        assert_eq!(s.turns[1].usage.reasoning, 88);
        assert_eq!(s.turns[1].model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Read");
        assert_eq!(s.turns[1].tool_calls[0].result.as_deref(), Some("conteúdo"));
    }
}
