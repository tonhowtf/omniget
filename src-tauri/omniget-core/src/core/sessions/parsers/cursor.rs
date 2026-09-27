//! Cursor: três armazenamentos que se sobrepõem (estudo 06, Cursor §j).
//!
//! 1. IDE: `<config>/Cursor/User/globalStorage/state.vscdb`, tabela
//!    `cursorDiskKV`, chaves `composerData:<id>` (cabeçalho da conversa) e
//!    `bubbleId:<id>:<bubble>` (cada mensagem; `type` 1 = usuário, 2 =
//!    assistente; `toolFormerData` = chamada de tool; `tokenCount`).
//! 2. CLI (`cursor-agent`/`agent`): `~/.cursor/chats/<hash>/<uuid>/store.db`,
//!    tabelas `meta(key, value)` (JSON, às vezes em hex) e `blobs(id, data)`
//!    (árvore endereçada por conteúdo; as mensagens são linhas JSON com `role`).
//! 3. Transcrições de agente: `~/.cursor/projects/<slug>/agent-transcripts/
//!    <id>.jsonl` ou `<id>/<id>.jsonl` (+ `subagents/*.jsonl`), linhas
//!    `{role, message:{content:[text|tool_use]}}` e `{type:"turn_ended"}`.
//!
//! O id das transcrições é o mesmo `composerId` da IDE: a IDE vence (tem
//! resultado de tool e horário por mensagem); a transcrição cobre o resto.
//! Verificado em arquivos reais nesta máquina (IDE 2 GB e 91 transcrições).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::util::{self, *};
use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus, Turn,
};

pub struct CursorSource;

const TOOL: &str = "cursor";

fn cursor_home() -> Option<PathBuf> {
    home_join(&[".cursor"])
}

/// `state.vscdb` globais do Cursor (desktop e servidor remoto).
fn ide_dbs() -> Vec<(String, PathBuf)> {
    vscode_user_dirs()
        .into_iter()
        .filter(|(label, _)| label == "Cursor" || label == "cursor-server")
        .map(|(label, p)| (label, p.join("globalStorage").join("state.vscdb")))
        .filter(|(_, p)| p.is_file())
        .collect()
}

fn chat_dbs() -> Vec<PathBuf> {
    let Some(root) = cursor_home().map(|h| h.join("chats")) else {
        return Vec::new();
    };
    find_files(&root, 3, |p| file_name(p) == "store.db")
}

/// Transcrições: (arquivo, id, pai).
fn transcripts() -> Vec<(PathBuf, String, Option<String>)> {
    let Some(root) = cursor_home().map(|h| h.join("projects")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for f in find_files(&root, 5, |p| {
        p.extension().and_then(|e| e.to_str()) == Some("jsonl")
            && p.components().any(|c| c.as_os_str() == "agent-transcripts")
    }) {
        let id = file_stem(&f);
        let parent_dir = parent_name(&f);
        if parent_dir == "subagents" {
            let owner = f
                .parent()
                .and_then(Path::parent)
                .map(file_name)
                .unwrap_or_default();
            out.push((f, id, Some(owner).filter(|s| !s.is_empty())));
        } else {
            out.push((f, id, None));
        }
    }
    out
}

/// Slug do Cursor (`Users-tonho-Documents-tonho-wtf`) de volta para caminho,
/// testando no disco onde cada `-` era `/`, `-`, `.`, `_` ou espaço.
pub fn resolve_slug(slug: &str) -> Option<String> {
    let segs: Vec<&str> = slug.split('-').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return None;
    }
    let (root, rest): (PathBuf, &[&str]) = if cfg!(windows) && segs[0].len() == 1 {
        (PathBuf::from(format!("{}:\\", segs[0])), &segs[1..])
    } else {
        (PathBuf::from("/"), &segs[..])
    };
    fn dfs(dir: &Path, rest: &[&str], budget: &mut u32) -> Option<PathBuf> {
        if rest.is_empty() {
            return Some(dir.to_path_buf());
        }
        for k in 1..=rest.len() {
            for j in ["-", ".", "_", " "] {
                if k == 1 && j != "-" {
                    continue;
                }
                if *budget == 0 {
                    return None;
                }
                *budget -= 1;
                let name = rest[..k].join(j);
                let cand = dir.join(&name);
                if cand.is_dir() {
                    if let Some(p) = dfs(&cand, &rest[k..], budget) {
                        return Some(p);
                    }
                }
            }
        }
        None
    }
    let mut budget = 400;
    dfs(&root, rest, &mut budget).map(|p| p.to_string_lossy().into_owned())
}

/// `<timestamp>Tuesday, Sep 1, 2026, 10:39 AM (UTC-3)</timestamp>` que o
/// Cursor injeta no prompt da transcrição.
fn transcript_ts(text: &str) -> Option<String> {
    let a = text.find("<timestamp>")? + "<timestamp>".len();
    let b = text[a..].find("</timestamp>")? + a;
    let raw = text[a..b].trim();
    let (dt, off) = match raw.rfind("(UTC") {
        Some(i) => (raw[..i].trim(), raw[i + 4..].trim_end_matches(')').trim()),
        None => (raw, ""),
    };
    let naive = chrono::NaiveDateTime::parse_from_str(dt, "%A, %b %d, %Y, %I:%M %p").ok()?;
    let mut offset_min: i32 = 0;
    if !off.is_empty() {
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let body = off.trim_start_matches(['+', '-']);
        let mut parts = body.split(':');
        let h: i32 = parts.next()?.parse().ok()?;
        let m: i32 = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        offset_min = sign * (h * 60 + m);
    }
    let utc = naive - chrono::Duration::minutes(offset_min as i64);
    Some(ms_to_rfc3339(utc.and_utc().timestamp_millis()))
}

// ---------------------------------------------------------------- IDE

fn composer_meta(v: &Value, db: &Path, account: &str) -> Option<SessionMeta> {
    let id = s(v, &["composerId"])?;
    let headers = v
        .get("fullConversationHeadersOnly")
        .and_then(Value::as_array);
    let inline = v.get("conversation").and_then(Value::as_array);
    let n = headers
        .map(|h| h.len())
        .or(inline.map(|c| c.len()))
        .unwrap_or(0);
    if n == 0 {
        return None;
    }
    let mut m = new_meta(TOOL, id, db);
    m.account = Some(account.to_string());
    m.title = s(v, &["name"]);
    m.started = ts_field(v, &["createdAt"]).map(ms_to_rfc3339);
    let upd = ts_field(
        v,
        &[
            "lastUpdatedAt",
            "conversationCheckpointLastUpdatedAt",
            "createdAt",
        ],
    );
    m.ended = upd.map(ms_to_rfc3339);
    if let Some(u) = upd {
        m.mtime_ms = u;
    }
    m.project_path = v
        .pointer("/workspaceIdentifier/uri/fsPath")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            v.pointer("/trackedGitRepos/0/repoPath")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    // Branch mais recente entre os repositórios acompanhados.
    if let Some(repos) = v.get("trackedGitRepos").and_then(Value::as_array) {
        let mut best: Option<(i64, String)> = None;
        for r in repos {
            for b in r
                .get("branches")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let at = b.get("lastInteractionAt").and_then(ts_ms).unwrap_or(0);
                if let Some(name) = s(b, &["branchName"]) {
                    if best.as_ref().map(|(t, _)| at > *t).unwrap_or(true) {
                        best = Some((at, name));
                    }
                }
            }
        }
        m.git_branch = best.map(|(_, n)| n);
    }
    if let Some(model) = v.pointer("/modelConfig/modelName").and_then(Value::as_str) {
        if !model.is_empty() {
            m.models.push(model.to_string());
        }
    }
    m.turn_count = headers
        .map(|h| {
            h.iter()
                .filter(|x| x.get("type").and_then(as_u64) == Some(1))
                .count()
        })
        .unwrap_or(n) as u32;
    Some(m)
}

fn list_ide() -> Vec<SessionMeta> {
    let mut out = Vec::new();
    for (label, db) in ide_dbs() {
        let Some(conn) = open_ro(&db) else { continue };
        let Ok(mut st) = conn.prepare(
            "SELECT value FROM cursorDiskKV WHERE key >= 'composerData:' AND key < 'composerData;'",
        ) else {
            continue;
        };
        let rows = st.query_map([], |r| r.get::<_, Option<String>>(0));
        let Ok(rows) = rows else { continue };
        for raw in rows.flatten().flatten() {
            let Ok(v) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            if let Some(m) = composer_meta(&v, &db, &label) {
                out.push(m);
            }
        }
    }
    out
}

fn bubble_status(s: &str) -> ToolStatus {
    match s {
        "completed" | "success" | "succeeded" => ToolStatus::Ok,
        "error" | "failed" | "cancelled" | "rejected" | "aborted" => ToolStatus::Error,
        _ => ToolStatus::Pending,
    }
}

/// Converte bolhas (na ordem) em turnos, juntando bolhas seguidas do
/// assistente num turno só.
pub fn bubbles_to_turns(bubbles: &[Value], fallback_model: Option<&str>) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for b in bubbles {
        let ty = b.get("type").and_then(as_u64).unwrap_or(0);
        let ts = b.get("createdAt").and_then(ts_str).unwrap_or_default();
        let text = s(b, &["text"]).unwrap_or_default();
        let mut usage = TokenUsage::default();
        if let Some(tc) = b.get("tokenCount") {
            usage.input = u(tc, &["inputTokens"]);
            usage.output = u(tc, &["outputTokens"]);
        }
        let model = b
            .pointer("/modelInfo/modelName")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .map(str::to_string);
        if ty == 1 {
            let mut t = turn(Role::User, ts, text);
            t.message_id = s(b, &["bubbleId"]);
            t.usage = usage;
            turns.push(t);
            continue;
        }
        let need_new = !matches!(turns.last(), Some(t) if t.role == Role::Assistant);
        if need_new {
            let mut t = turn(Role::Assistant, ts, String::new());
            t.message_id = s(b, &["bubbleId"]);
            turns.push(t);
        }
        let t = turns.last_mut().expect("turno recém criado");
        if !text.is_empty() {
            if !t.text.is_empty() {
                t.text.push('\n');
            }
            t.text.push_str(&text);
        }
        t.usage.add(&usage);
        if t.model.is_none() {
            t.model = model.or_else(|| fallback_model.map(str::to_string));
        }
        if let Some(tf) = b.get("toolFormerData").filter(|x| x.is_object()) {
            let raw = s(tf, &["name"]).unwrap_or_else(|| format!("tool_{}", u(tf, &["tool"])));
            let input = tf
                .get("params")
                .or_else(|| tf.get("rawArgs"))
                .map(|p| match p {
                    Value::String(st) => {
                        serde_json::from_str(st).unwrap_or(Value::String(st.clone()))
                    }
                    other => other.clone(),
                })
                .unwrap_or(Value::Null);
            let mut c = tool_call(s(tf, &["toolCallId"]).unwrap_or_default(), &raw, input);
            c.result = tf
                .get("result")
                .map(result_text)
                .filter(|r| !r.is_empty() && r != "{}");
            c.status = bubble_status(tf.get("status").and_then(Value::as_str).unwrap_or(""));
            t.tool_calls.push(c);
        }
    }
    turns
}

fn load_ide(id: &str) -> Option<Session> {
    for (label, db) in ide_dbs() {
        let Some(conn) = open_ro(&db) else { continue };
        let key = format!("composerData:{id}");
        let Ok(raw) = conn.query_row(
            "SELECT value FROM cursorDiskKV WHERE key = ?1",
            [&key],
            |r| r.get::<_, Option<String>>(0),
        ) else {
            continue;
        };
        let Some(v) = raw.and_then(|r| serde_json::from_str::<Value>(&r).ok()) else {
            continue;
        };
        let Some(mut meta) = composer_meta(&v, &db, &label) else {
            continue;
        };
        let fallback_model = meta.models.first().cloned();
        let bubbles: Vec<Value> = if let Some(conv) =
            v.get("conversation").and_then(Value::as_array)
        {
            conv.clone()
        } else {
            let lo = format!("bubbleId:{id}:");
            let hi = format!("bubbleId:{id};");
            let mut map: HashMap<String, Value> = HashMap::new();
            if let Ok(mut st) =
                conn.prepare("SELECT key, value FROM cursorDiskKV WHERE key >= ?1 AND key < ?2")
            {
                if let Ok(rows) = st.query_map([&lo, &hi], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                }) {
                    for (k, val) in rows.flatten() {
                        if let Some(val) = val.and_then(|x| serde_json::from_str::<Value>(&x).ok())
                        {
                            map.insert(k[lo.len()..].to_string(), val);
                        }
                    }
                }
            }
            let order: Vec<String> = v
                .get("fullConversationHeadersOnly")
                .and_then(Value::as_array)
                .map(|h| h.iter().filter_map(|x| s(x, &["bubbleId"])).collect())
                .unwrap_or_default();
            let mut out = Vec::new();
            for b in &order {
                if let Some(x) = map.remove(b) {
                    out.push(x);
                }
            }
            let mut rest: Vec<Value> = map.into_values().collect();
            rest.sort_by_key(|x| x.get("createdAt").and_then(ts_ms).unwrap_or(0));
            out.extend(rest);
            out
        };
        if bubbles.is_empty() {
            return None;
        }
        let mut turns = bubbles_to_turns(&bubbles, fallback_model.as_deref());
        meta.models.clear();
        finalize(&mut meta, &mut turns);
        if meta.models.is_empty() {
            meta.models.extend(fallback_model);
        }
        return Some(Session { meta, turns });
    }
    None
}

// ---------------------------------------------------------------- CLI

/// Valor do `meta` do store.db: JSON direto ou JSON codificado em hex.
fn decode_meta_value(raw: &[u8]) -> Option<Value> {
    if let Ok(v) = serde_json::from_slice::<Value>(raw) {
        if v.is_object() {
            return Some(v);
        }
    }
    let txt = std::str::from_utf8(raw).ok()?.trim();
    let bytes = hex::decode(txt).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn chat_meta(db: &Path) -> Option<SessionMeta> {
    let id = parent_name(db);
    if id.is_empty() {
        return None;
    }
    let mut m = new_meta(TOOL, id, db);
    m.account = Some("cli".into());
    let mut info: Option<Value> = db.parent().and_then(|d| read_json(&d.join("meta.json")));
    if let Some(conn) = open_ro(db) {
        if table_exists(&conn, "meta") {
            if let Ok(mut st) = conn.prepare("SELECT value FROM meta") {
                if let Ok(rows) = st.query_map([], |r| {
                    let v: rusqlite::types::Value = r.get(0)?;
                    Ok(v)
                }) {
                    for v in rows.flatten() {
                        let bytes = match v {
                            rusqlite::types::Value::Text(t) => t.into_bytes(),
                            rusqlite::types::Value::Blob(b) => b,
                            _ => continue,
                        };
                        if let Some(j) = decode_meta_value(&bytes) {
                            match info.as_mut() {
                                Some(Value::Object(o)) => {
                                    if let Value::Object(j) = j {
                                        for (k, x) in j {
                                            o.entry(k).or_insert(x);
                                        }
                                    }
                                }
                                _ => info = Some(j),
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(i) = info {
        m.title = s(&i, &["title", "name"]);
        m.project_path = s(&i, &["cwd", "workspace", "workspacePath"]);
        m.started = ts_field(&i, &["createdAtMs", "createdAt"]).map(ms_to_rfc3339);
        if let Some(upd) = ts_field(&i, &["updatedAtMs", "updatedAt", "lastUsedAt"]) {
            m.ended = Some(ms_to_rfc3339(upd));
            m.mtime_ms = m.mtime_ms.max(upd);
        }
        if let Some(model) = s(&i, &["lastUsedModel", "model"]) {
            m.models.push(model);
        }
    }
    Some(m)
}

/// Mensagens JSON de um store.db (ignora os blobs binários da árvore).
fn chat_messages(db: &Path) -> Vec<Value> {
    let Some(conn) = open_ro(db) else {
        return Vec::new();
    };
    if !table_exists(&conn, "blobs") {
        return Vec::new();
    }
    let Ok(mut st) = conn.prepare("SELECT data FROM blobs ORDER BY rowid") else {
        return Vec::new();
    };
    let Ok(rows) = st.query_map([], |r| r.get::<_, Option<Vec<u8>>>(0)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for data in rows.flatten().flatten() {
        let first = data.iter().find(|b| !b.is_ascii_whitespace()).copied();
        if first != Some(b'{') {
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<Value>(&data) {
            if v.get("role").and_then(Value::as_str).is_some() {
                out.push(v);
            }
        }
    }
    out
}

/// Mensagens estilo AI SDK/OpenAI/Anthropic (`role` + `content`) em turnos.
pub fn role_messages_to_turns(msgs: &[Value]) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    let mut results = Vec::new();
    for m in msgs {
        let role = m.get("role").and_then(Value::as_str).unwrap_or("");
        if role == "system" {
            continue;
        }
        let content = m
            .get("content")
            .or_else(|| m.pointer("/message/content"))
            .cloned()
            .unwrap_or(Value::Null);
        let (text, calls, res) = anthropic_blocks(&content);
        results.extend(res);
        // OpenAI: tool_calls[] + role "tool" com tool_call_id.
        let mut calls = calls;
        if let Some(tc) = m.get("tool_calls").and_then(Value::as_array) {
            for c in tc {
                let name = c
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let args = c
                    .pointer("/function/arguments")
                    .map(|a| match a {
                        Value::String(st) => serde_json::from_str(st).unwrap_or(a.clone()),
                        other => other.clone(),
                    })
                    .unwrap_or(Value::Null);
                calls.push(tool_call(s(c, &["id"]).unwrap_or_default(), name, args));
            }
        }
        if role == "tool" {
            if let Some(id) = s(m, &["tool_call_id", "toolCallId"]) {
                results.push((id, text.clone(), false));
            }
            continue;
        }
        let r = role_of(role);
        if r == Role::User && text.trim().is_empty() && calls.is_empty() {
            continue;
        }
        let ts = m
            .get("timestamp")
            .or_else(|| m.get("createdAt"))
            .and_then(ts_str)
            .unwrap_or_default();
        let merge =
            r == Role::Assistant && matches!(turns.last(), Some(t) if t.role == Role::Assistant);
        if merge {
            let t = turns.last_mut().expect("existe");
            if !text.is_empty() {
                if !t.text.is_empty() {
                    t.text.push('\n');
                }
                t.text.push_str(&text);
            }
            t.tool_calls.extend(calls);
        } else {
            let ts = if ts.is_empty() && r == Role::User {
                transcript_ts(&text).unwrap_or_default()
            } else {
                ts
            };
            let mut t = turn(r, ts, text);
            t.tool_calls = calls;
            t.message_id = s(m, &["id"]);
            turns.push(t);
        }
    }
    attach_results(&mut turns, results);
    turns
}

fn load_chat(db: &Path) -> Option<Session> {
    let mut meta = chat_meta(db)?;
    let msgs = chat_messages(db);
    let mut turns = role_messages_to_turns(&msgs);
    let model = meta.models.first().cloned();
    for t in turns.iter_mut().filter(|t| t.role == Role::Assistant) {
        t.model = model.clone();
    }
    finalize(&mut meta, &mut turns);
    Some(Session { meta, turns })
}

// ---------------------------------------------------------------- transcrições

pub fn parse_transcript(path: &Path, id: &str, parent: Option<String>) -> Session {
    let mut rows = Vec::new();
    let mut errors = 0u32;
    for_each_jsonl(path, |v| {
        if v.get("type").and_then(Value::as_str) == Some("turn_ended") {
            if v.get("status").and_then(Value::as_str) == Some("error") {
                errors += 1;
            }
            return;
        }
        if v.get("role").is_some() {
            rows.push(v);
        }
    });
    let mut turns = role_messages_to_turns(&rows);
    // A transcrição não guarda resultado: chamada conhecida = concluída.
    for t in turns.iter_mut() {
        for c in t.tool_calls.iter_mut() {
            if c.status == ToolStatus::Pending {
                c.status = ToolStatus::Ok;
            }
        }
    }
    let _ = errors;
    let mut meta = new_meta(TOOL, id, path);
    meta.parent_session = parent;
    meta.account = Some("agent-transcripts".into());
    // projects/<slug>/agent-transcripts/...
    let slug = path
        .ancestors()
        .find(|a| file_name(a) == "agent-transcripts")
        .and_then(Path::parent)
        .map(file_name)
        .unwrap_or_default();
    if !slug.is_empty() && slug != "empty-window" {
        meta.project_path = resolve_slug(&slug);
    }
    meta.ended = Some(ms_to_rfc3339(meta.mtime_ms));
    finalize(&mut meta, &mut turns);
    Session { meta, turns }
}

impl SessionSource for CursorSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = ide_dbs().into_iter().map(|(_, p)| p).collect();
        if let Some(h) = cursor_home() {
            for sub in ["chats", "projects"] {
                let p = h.join(sub);
                if p.is_dir() {
                    v.push(p);
                }
            }
        }
        v
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out = list_ide();
        let mut seen: HashSet<String> = out.iter().map(|m| m.id.clone()).collect();
        for db in chat_dbs() {
            if let Some(m) = chat_meta(&db) {
                if seen.insert(m.id.clone()) {
                    out.push(m);
                }
            }
        }
        for (f, id, parent) in transcripts() {
            if seen.contains(&id) {
                continue;
            }
            let s = parse_transcript(&f, &id, parent);
            seen.insert(id);
            out.push(s.meta);
        }
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        if let Some(s) = load_ide(id) {
            return Some(s);
        }
        if let Some(db) = chat_dbs().into_iter().find(|p| parent_name(p) == id) {
            return load_chat(&db);
        }
        transcripts()
            .into_iter()
            .find(|(_, tid, _)| tid == id)
            .map(|(f, tid, parent)| parse_transcript(&f, &tid, parent))
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        // Só as conversas do CLI retomam pelo CLI; as da IDE abrem na IDE.
        if meta.account.as_deref() == Some("cli") {
            Some(format!(
                "cursor-agent --resume {}",
                util::shell_quote(&meta.id)
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_fixture() {
        let dir = std::env::temp_dir().join(format!("omniget-cursor-{}", std::process::id()));
        let tdir = dir.join("agent-transcripts").join("abc");
        std::fs::create_dir_all(&tdir).unwrap();
        let f = tdir.join("abc.jsonl");
        std::fs::write(
            &f,
            concat!(
                r#"{"role":"user","message":{"content":[{"type":"text","text":"<timestamp>Tuesday, Sep 1, 2026, 10:39 AM (UTC-3)</timestamp>\n<user_query>\nArrume o build\n</user_query>"}]}}"#, "\n",
                r#"{"role":"assistant","message":{"content":[{"type":"text","text":"Vou ler."},{"type":"tool_use","name":"Read","input":{"path":"/a"}},{"type":"tool_use","name":"Shell","input":{"command":"ls"}}]}}"#, "\n",
                r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Task","input":{"subagent_type":"explore","prompt":"x"}}]}}"#, "\n",
                r#"{"type":"turn_ended","status":"success"}"#, "\n",
                "lixo\n",
            ),
        )
        .unwrap();
        let s = parse_transcript(&f, "abc", None);
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.turns[0].ts, "2026-09-01T13:39:00.000Z");
        assert_eq!(s.meta.title.as_deref(), Some("Arrume o build"));
        let names: Vec<_> = s.turns[1]
            .tool_calls
            .iter()
            .map(|c| c.name_canonical.as_str())
            .collect();
        assert_eq!(names, ["Read", "Bash", "Agent"]);
        assert_eq!(
            s.turns[1].tool_calls[2].subagent.as_deref(),
            Some("explore")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ide_bubbles() {
        let b: Vec<Value> = serde_json::from_str(r#"[
            {"type":1,"bubbleId":"u1","text":"oi","createdAt":"2026-09-22T09:15:17.028Z","tokenCount":{"inputTokens":0,"outputTokens":0}},
            {"type":2,"bubbleId":"a1","text":"lendo","createdAt":"2026-09-22T09:15:20.000Z","tokenCount":{"inputTokens":10,"outputTokens":5},"modelInfo":{"modelName":"claude-4"}},
            {"type":2,"bubbleId":"a2","createdAt":"2026-09-22T09:15:21.000Z","toolFormerData":{"toolCallId":"t1","name":"run_terminal_command_v2","params":"{\"command\":\"ls\"}","result":"{\"output\":\"a\"}","status":"completed"}},
            {"type":2,"bubbleId":"a3","createdAt":"2026-09-22T09:15:22.000Z","toolFormerData":{"toolCallId":"t2","name":"edit_file_v2","params":"{}","status":"error"}}
        ]"#).unwrap();
        let t = bubbles_to_turns(&b, Some("default"));
        assert_eq!(t.len(), 2);
        assert_eq!(t[1].usage.input, 10);
        assert_eq!(t[1].model.as_deref(), Some("claude-4"));
        assert_eq!(t[1].tool_calls[0].name_canonical, "Bash");
        assert_eq!(t[1].tool_calls[0].status, ToolStatus::Ok);
        assert_eq!(t[1].tool_calls[0].input["command"], "ls");
        assert_eq!(t[1].tool_calls[1].status, ToolStatus::Error);
    }

    #[test]
    fn meta_hex() {
        let hexed = hex::encode(br#"{"name":"x","createdAt":1}"#);
        assert_eq!(decode_meta_value(hexed.as_bytes()).unwrap()["name"], "x");
    }

    /// Contra os arquivos reais desta máquina (só leitura); ignora se não há Cursor.
    #[test]
    #[ignore]
    fn real_cursor() {
        let src = CursorSource;
        let l = src.list();
        eprintln!("cursor: {} sessões", l.len());
        for m in l.iter().take(5) {
            let s = src.load(&m.id).expect("load");
            eprintln!(
                "{} {:?} turns={} tools={}",
                m.id,
                m.title,
                s.turns.len(),
                s.turns.iter().map(|t| t.tool_calls.len()).sum::<usize>()
            );
        }
        // Transcrições direto (na lista elas perdem para a IDE pelo id).
        let (mut n, mut turns, mut tools, mut dated, mut with_proj) = (0, 0, 0, 0, 0);
        let mut other = std::collections::BTreeMap::new();
        for (f, id, parent) in transcripts() {
            let s = parse_transcript(&f, &id, parent);
            n += 1;
            turns += s.turns.len();
            dated += s
                .turns
                .iter()
                .filter(|t| t.role == Role::User && !t.ts.is_empty())
                .count();
            with_proj += s.meta.project_path.is_some() as usize;
            for c in s.turns.iter().flat_map(|t| t.tool_calls.iter()) {
                tools += 1;
                if c.name_canonical == "Other" {
                    *other.entry(c.name_raw.clone()).or_insert(0) += 1;
                }
            }
        }
        eprintln!("transcrições={n} turnos={turns} tools={tools} prompts_datados={dated} com_projeto={with_proj} other={other:?}");
    }
}
