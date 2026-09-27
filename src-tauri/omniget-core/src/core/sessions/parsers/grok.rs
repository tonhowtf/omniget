//! Grok: o oficial Grok Build (xAI) e o grok-cli comunitário
//! (superagent-ai/grok-cli), que dividem `~/.grok/` (estudo 06, Grok §j;
//! Parte 6 §8.1/§8.3).
//!
//! Grok Build: `${GROK_HOME:-~/.grok}/sessions/<cwd codificado em %>/<id>/`
//! com `updates.jsonl` (eventos ACP `{"method":"session/update","params":
//! {sessionId, update:{sessionUpdate, …}, _meta:{agentTimestampMs}}}`:
//! `user_message_chunk`, `agent_message_chunk`, `agent_thought_chunk`,
//! `tool_call{toolCallId, title, kind, rawInput}`, `tool_call_update{toolCallId,
//! status, content, rawOutput}`, `turn_completed{usage, modelUsage,
//! costUsdTicks}`), `summary.json` (título, horários, modelo, pai) e `.cwd`
//! quando o nome da pasta é um hash. Uso de `turn_completed`: `inputTokens`
//! inclui o cache lido, `outputTokens` inclui o raciocínio; aqui são separados.
//! `costUsdTicks` = 1e-10 USD.
//!
//! grok-cli: `~/.grok/grok.db` (SQLite): `workspaces(id, canonical_path)`,
//! `sessions(id, workspace_id, title, model, cwd_at_start, created_at,
//! updated_at)`, `messages(session_id, seq, role, message_json, created_at)`
//! (mensagens AI SDK), `tool_calls(id, session_id, message_seq, tool_call_id,
//! tool_name, args_json, status)`, `tool_results(tool_call_row_id,
//! output_json, success)`, `usage_events(session_id, message_seq, model,
//! input_tokens, output_tokens, cost_micros, created_at)`.
//! Nenhum dos dois está instalado aqui: só fixture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::util::*;
use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus, Turn,
};

pub struct GrokSource;

const TOOL: &str = "grok";

fn grok_home() -> Option<PathBuf> {
    env_dir("GROK_HOME").or_else(|| home_join(&[".grok"]))
}

/// Pastas de sessão do Grok Build (contêm `updates.jsonl`).
fn build_dirs() -> Vec<PathBuf> {
    let Some(h) = grok_home() else {
        return Vec::new();
    };
    find_files(&h.join("sessions"), 3, |p| file_name(p) == "updates.jsonl")
        .into_iter()
        .filter_map(|f| f.parent().map(Path::to_path_buf))
        .filter(|d| d.parent().map(file_name).as_deref() != Some("subagents"))
        .collect()
}

fn cli_db() -> Option<PathBuf> {
    grok_home()
        .map(|h| h.join("grok.db"))
        .filter(|p| p.is_file())
}

/// Tipo de tool do ACP (`kind`) para o nome canônico.
fn acp_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "read" => "Read",
        "edit" => "Edit",
        "delete" | "move" => "Edit",
        "search" => "Grep",
        "execute" => "Bash",
        "fetch" => "WebFetch",
        _ => return None,
    })
}

fn grok_usage(us: &Value) -> TokenUsage {
    let cache_read = u(
        us,
        &[
            "cachedReadTokens",
            "cacheReadTokens",
            "cache_read_input_tokens",
            "cacheReadInputTokens",
        ],
    );
    let cache_write = u(
        us,
        &[
            "cachedWriteTokens",
            "cacheWriteTokens",
            "cacheCreationTokens",
            "cache_creation_input_tokens",
        ],
    );
    let reasoning = u(
        us,
        &[
            "reasoningTokens",
            "thoughtTokens",
            "thinkingTokens",
            "reasoning_tokens",
        ],
    );
    let input_raw = u(us, &["inputTokens", "input_tokens", "promptTokens"]);
    let output_raw = u(us, &["outputTokens", "output_tokens", "completionTokens"]);
    TokenUsage {
        input: input_raw.saturating_sub(cache_read),
        output: output_raw.saturating_sub(reasoning),
        cache_read,
        cache_write,
        reasoning,
    }
}

fn chunk_text(update: &Value) -> String {
    update.get("content").map(content_text).unwrap_or_default()
}

pub fn parse_updates(lines: &[Value], id: &str, source: &Path) -> Session {
    let mut meta = new_meta(TOOL, id, source);
    meta.account = Some("grok-build".into());
    let mut turns: Vec<Turn> = Vec::new();
    let mut model: Option<String> = None;
    for l in lines {
        let params = l.get("params").cloned().unwrap_or_else(|| l.clone());
        let Some(up) = params.get("update") else {
            continue;
        };
        let ty = up
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("");
        let ts = params
            .pointer("/_meta/agentTimestampMs")
            .or_else(|| up.pointer("/_meta/agentTimestampMs"))
            .or_else(|| params.get("timestamp"))
            .and_then(ts_str)
            .unwrap_or_default();
        if let Some(m) = up
            .pointer("/_meta/modelId")
            .or_else(|| params.pointer("/_meta/modelId"))
            .and_then(Value::as_str)
        {
            model = Some(m.to_string());
        }
        match ty {
            "user_message_chunk" => {
                let text = chunk_text(up);
                match turns.last_mut() {
                    Some(t) if t.role == Role::User => t.text.push_str(&text),
                    _ => turns.push(turn(Role::User, ts, text)),
                }
            }
            "agent_message_chunk" => {
                let text = chunk_text(up);
                match turns.last_mut() {
                    Some(t) if t.role == Role::Assistant => t.text.push_str(&text),
                    _ => {
                        let mut t = turn(Role::Assistant, ts, text);
                        t.model = model.clone();
                        turns.push(t);
                    }
                }
            }
            "tool_call" => {
                let raw = up
                    .pointer("/_meta/toolName")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| s(up, &["title"]))
                    .unwrap_or_else(|| "tool".into());
                let mut c = tool_call(
                    s(up, &["toolCallId"]).unwrap_or_default(),
                    &raw,
                    up.get("rawInput").cloned().unwrap_or(Value::Null),
                );
                if c.name_canonical == "Other" {
                    if let Some(k) = up.get("kind").and_then(Value::as_str).and_then(acp_kind) {
                        c.name_canonical = k.to_string();
                    }
                }
                if !matches!(turns.last(), Some(t) if t.role == Role::Assistant) {
                    let mut t = turn(Role::Assistant, ts, "");
                    t.model = model.clone();
                    turns.push(t);
                }
                turns.last_mut().expect("existe").tool_calls.push(c);
            }
            "tool_call_update" => {
                let id = s(up, &["toolCallId"]).unwrap_or_default();
                let status = s(up, &["status"]).unwrap_or_default();
                for t in turns.iter_mut().rev() {
                    if let Some(c) = t.tool_calls.iter_mut().find(|c| c.id == id) {
                        let body = up
                            .get("rawOutput")
                            .or_else(|| up.get("content"))
                            .map(result_text)
                            .filter(|x| !x.is_empty());
                        if body.is_some() {
                            c.result = body;
                        }
                        c.status = match status.as_str() {
                            "completed" => ToolStatus::Ok,
                            "failed" => ToolStatus::Error,
                            _ => c.status,
                        };
                        break;
                    }
                }
            }
            "turn_completed" => {
                let mut us = up.get("usage").map(grok_usage).unwrap_or_default();
                let mut m = model.clone();
                if us.total() == 0 {
                    if let Some(Value::Object(mu)) = up.get("modelUsage") {
                        for (k, v) in mu {
                            us.add(&grok_usage(v));
                            m = Some(k.clone());
                        }
                    }
                }
                let cost = up
                    .get("costUsdTicks")
                    .or_else(|| up.pointer("/usage/costUsdTicks"))
                    .and_then(as_f64)
                    .map(|t| t / 1e10);
                if !matches!(turns.last(), Some(t) if t.role == Role::Assistant) {
                    turns.push(turn(Role::Assistant, ts, ""));
                }
                let t = turns.last_mut().expect("existe");
                t.usage.add(&us);
                if cost.is_some() {
                    t.cost_usd = cost;
                }
                if t.model.is_none() {
                    t.model = m;
                }
            }
            _ => {}
        }
    }
    finalize(&mut meta, &mut turns);
    Session { meta, turns }
}

fn load_build(dir: &Path) -> Session {
    let id = file_name(dir);
    let upd = dir.join("updates.jsonl");
    let mut sess = parse_updates(&read_jsonl(&upd), &id, &upd);
    if let Some(sm) = read_json(&dir.join("summary.json")) {
        if let Some(t) = s_title(&sm) {
            sess.meta.title = Some(t);
        }
        if let Some(c) = sm.get("created_at").and_then(ts_str) {
            sess.meta.started = Some(c);
        }
        if let Some(u_) = sm.get("updated_at").and_then(ts_str) {
            sess.meta.ended = Some(u_);
        }
        sess.meta.parent_session = s(&sm, &["parent_session_id", "parentSessionId", "parent"]);
        if let Some(m) = s(&sm, &["current_model_id", "model_id"]) {
            if !sess.meta.models.contains(&m) {
                sess.meta.models.push(m);
            }
        }
    }
    let ws = dir.parent().map(file_name).unwrap_or_default();
    let cwd_file = dir.parent().map(|p| p.join(".cwd"));
    sess.meta.project_path = cwd_file
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .or_else(|| {
            let d = percent_decode(&ws);
            (d.contains('/') || d.contains('\\')).then_some(d)
        });
    sess
}

fn s_title(sm: &Value) -> Option<String> {
    s(sm, &["title", "name"])
}

// ------------------------------------------------------------ grok-cli (SQLite)

fn cli_sessions(db: &Path, only: Option<&str>) -> Vec<Session> {
    let Some(conn) = open_ro(db) else {
        return Vec::new();
    };
    if !table_exists(&conn, "sessions") {
        return Vec::new();
    }
    let has_ws = table_exists(&conn, "workspaces");
    let sql = format!(
        "SELECT s.id, s.title, s.model, s.cwd_at_start, s.created_at, s.updated_at, {} FROM sessions s {} {} ORDER BY s.updated_at",
        if has_ws { "w.canonical_path" } else { "NULL" },
        if has_ws { "LEFT JOIN workspaces w ON w.id = s.workspace_id" } else { "" },
        if only.is_some() { "WHERE s.id = ?1" } else { "" },
    );
    let Ok(mut st) = conn.prepare(&sql) else {
        return Vec::new();
    };
    let map = |r: &rusqlite::Row| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1).ok().flatten(),
            r.get::<_, Option<String>>(2).ok().flatten(),
            r.get::<_, Option<String>>(3).ok().flatten(),
            r.get::<_, Option<String>>(4).ok().flatten(),
            r.get::<_, Option<String>>(5).ok().flatten(),
            r.get::<_, Option<String>>(6).ok().flatten(),
        ))
    };
    let rows: Vec<_> = match only {
        Some(id) => st
            .query_map([id], map)
            .map(|r| r.flatten().collect())
            .unwrap_or_default(),
        None => st
            .query_map([], map)
            .map(|r| r.flatten().collect())
            .unwrap_or_default(),
    };
    let mut out = Vec::new();
    for (id, title, model, cwd, created, updated, ws_path) in rows {
        let mut meta = new_meta(TOOL, id.clone(), db);
        meta.account = Some("grok-cli".into());
        meta.title = title;
        meta.project_path = ws_path.or(cwd);
        meta.started = created.as_deref().and_then(str_to_ms).map(ms_to_rfc3339);
        if let Some(u_) = updated.as_deref().and_then(str_to_ms) {
            meta.ended = Some(ms_to_rfc3339(u_));
            meta.mtime_ms = u_;
        }
        // Mensagens.
        let mut msgs: Vec<(i64, Value, String)> = Vec::new();
        if let Ok(mut ms) = conn.prepare("SELECT seq, role, message_json, created_at FROM messages WHERE session_id = ?1 ORDER BY seq") {
            if let Ok(rows) = ms.query_map([&id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            }) {
                for (seq, role, json, created) in rows.flatten() {
                    let mut v: Value = json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or(Value::Null);
                    if v.get("role").is_none() {
                        if let Value::Object(ref mut o) = v {
                            o.insert("role".into(), Value::String(role.unwrap_or_default()));
                        }
                    }
                    let ts = created.as_deref().and_then(str_to_ms).map(ms_to_rfc3339).unwrap_or_default();
                    msgs.push((seq, v, ts));
                }
            }
        }
        let mut turns: Vec<Turn> = Vec::new();
        let mut seq_turn: HashMap<i64, usize> = HashMap::new();
        let mut results = Vec::new();
        for (seq, m, ts) in &msgs {
            let role = m.get("role").and_then(Value::as_str).unwrap_or("");
            let (text, calls, res) = anthropic_blocks(m.get("content").unwrap_or(&Value::Null));
            results.extend(res);
            if role == "tool" || role == "system" {
                continue;
            }
            let r = role_of(role);
            if r == Role::User && text.trim().is_empty() {
                continue;
            }
            let mut t = turn(r, ts.clone(), text);
            t.tool_calls = calls;
            if r == Role::Assistant {
                t.model = model.clone();
            }
            seq_turn.insert(*seq, turns.len());
            turns.push(t);
        }
        attach_results(&mut turns, results);
        // Tabela de tools (status e resultado autoritativos).
        if table_exists(&conn, "tool_calls") {
            let has_res = table_exists(&conn, "tool_results");
            let sql = format!(
                "SELECT c.message_seq, c.tool_call_id, c.tool_name, c.args_json, c.status, {} FROM tool_calls c {} WHERE c.session_id = ?1 ORDER BY c.id",
                if has_res { "r.output_json, r.success" } else { "NULL, NULL" },
                if has_res { "LEFT JOIN tool_results r ON r.tool_call_row_id = c.id" } else { "" },
            );
            if let Ok(mut tc) = conn.prepare(&sql) {
                if let Ok(rows) = tc.query_map([&id], |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<i64>>(6)?,
                    ))
                }) {
                    for (seq, cid, name, args, status, out_json, success) in rows.flatten() {
                        let existing = turns
                            .iter_mut()
                            .flat_map(|t| t.tool_calls.iter_mut())
                            .find(|c| c.id == cid);
                        let status_v = match (success, status.as_deref()) {
                            (Some(1), _) | (_, Some("completed" | "success")) => ToolStatus::Ok,
                            (Some(0), _) | (_, Some("error" | "failed")) => ToolStatus::Error,
                            _ => ToolStatus::Pending,
                        };
                        let result = out_json.map(|o| {
                            serde_json::from_str::<Value>(&o)
                                .map(|v| result_text(&v))
                                .unwrap_or(o)
                        });
                        match existing {
                            Some(c) => {
                                c.status = status_v;
                                if result.is_some() {
                                    c.result = result;
                                }
                            }
                            None => {
                                let input = args
                                    .and_then(|a| serde_json::from_str(&a).ok())
                                    .unwrap_or(Value::Null);
                                let mut c = tool_call(cid, &name, input);
                                c.status = status_v;
                                c.result = result;
                                let idx = seq.and_then(|q| seq_turn.get(&q).copied());
                                match idx.and_then(|i| turns.get_mut(i)) {
                                    Some(t) => t.tool_calls.push(c),
                                    None => {
                                        if let Some(t) = turns
                                            .iter_mut()
                                            .rev()
                                            .find(|t| t.role == Role::Assistant)
                                        {
                                            t.tool_calls.push(c);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Uso.
        if table_exists(&conn, "usage_events") {
            if let Ok(mut ue) = conn.prepare(
                "SELECT message_seq, model, input_tokens, output_tokens, cost_micros, created_at FROM usage_events WHERE session_id = ?1 ORDER BY id",
            ) {
                if let Ok(rows) = ue.query_map([&id], |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                    ))
                }) {
                    for (seq, m, inp, outp, micros, created) in rows.flatten() {
                        let us = TokenUsage {
                            input: inp.unwrap_or(0).max(0) as u64,
                            output: outp.unwrap_or(0).max(0) as u64,
                            ..Default::default()
                        };
                        let cost = micros.map(|c| c.max(0) as f64 / 1e6);
                        let idx = seq
                            .and_then(|q| seq_turn.get(&q).copied())
                            .filter(|i| turns[*i].role == Role::Assistant)
                            .or_else(|| turns.iter().rposition(|t| t.role == Role::Assistant));
                        let t = match idx {
                            Some(i) => &mut turns[i],
                            None => {
                                let ts = created.as_deref().and_then(str_to_ms).map(ms_to_rfc3339).unwrap_or_default();
                                turns.push(turn(Role::Assistant, ts, ""));
                                turns.last_mut().expect("existe")
                            }
                        };
                        t.usage.add(&us);
                        if let Some(c) = cost {
                            t.cost_usd = Some(t.cost_usd.unwrap_or(0.0) + c);
                        }
                        if m.is_some() {
                            t.model = m;
                        }
                    }
                }
            }
        }
        finalize(&mut meta, &mut turns);
        out.push(Session { meta, turns });
    }
    out
}

impl SessionSource for GrokSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Some(h) = grok_home() {
            let s = h.join("sessions");
            if s.is_dir() {
                v.push(s);
            }
        }
        v.extend(cli_db());
        v
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out: Vec<SessionMeta> = build_dirs().iter().map(|d| load_build(d).meta).collect();
        if let Some(db) = cli_db() {
            out.extend(cli_sessions(&db, None).into_iter().map(|s| s.meta));
        }
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        if let Some(d) = build_dirs().into_iter().find(|d| file_name(d) == id) {
            return Some(load_build(&d));
        }
        let db = cli_db()?;
        cli_sessions(&db, Some(id)).into_iter().next()
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        (meta.account.as_deref() == Some("grok-build"))
            .then(|| format!("grok --resume {}", shell_quote(&meta.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_fixture() {
        let l: Vec<Value> = [
            r#"{"method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"rode os testes"},"_meta":{"modelId":"grok-4.5"}},"_meta":{"agentTimestampMs":1700000001000}}}"#,
            r#"{"method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Rodando"}},"_meta":{"agentTimestampMs":1700000002000}}}"#,
            r#"{"method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call","toolCallId":"c1","title":"cargo test","kind":"execute","status":"pending","rawInput":{"command":"cargo test"}}}}"#,
            r#"{"method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call_update","toolCallId":"c1","status":"completed","rawOutput":"ok"}}}"#,
            r#"{"method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"turn_completed","usage":{"inputTokens":100,"outputTokens":25,"reasoningTokens":5,"cachedReadTokens":60},"costUsdTicks":12345000000},"_meta":{"agentTimestampMs":1700000003000}}}"#,
        ]
        .iter()
        .map(|x| serde_json::from_str(x).unwrap())
        .collect();
        let s = parse_updates(&l, "s1", Path::new("/x/updates.jsonl"));
        assert_eq!(s.turns.len(), 2);
        let t = &s.turns[1];
        assert_eq!(t.usage.input, 40);
        assert_eq!(t.usage.cache_read, 60);
        assert_eq!(t.usage.output, 20);
        assert_eq!(t.usage.reasoning, 5);
        assert!((t.cost_usd.unwrap() - 1.2345).abs() < 1e-9);
        assert_eq!(t.tool_calls[0].name_canonical, "Bash");
        assert_eq!(t.tool_calls[0].status, ToolStatus::Ok);
        assert_eq!(t.model.as_deref(), Some("grok-4.5"));
    }

    #[test]
    fn cli_db_fixture() {
        let dir = std::env::temp_dir().join(format!("omniget-grok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("grok.db");
        let _ = std::fs::remove_file(&db);
        {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch(r#"
              CREATE TABLE workspaces (id TEXT PRIMARY KEY, scope_key TEXT, canonical_path TEXT, git_root TEXT, display_name TEXT, last_seen_at TEXT);
              CREATE TABLE sessions (id TEXT PRIMARY KEY, workspace_id TEXT, title TEXT, model TEXT, mode TEXT, cwd_at_start TEXT, cwd_last TEXT, status TEXT, created_at TEXT, updated_at TEXT);
              CREATE TABLE messages (session_id TEXT, seq INTEGER, role TEXT, message_json TEXT, created_at TEXT);
              CREATE TABLE tool_calls (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT, message_seq INTEGER, tool_call_id TEXT, tool_name TEXT, args_json TEXT, status TEXT, started_at TEXT, completed_at TEXT);
              CREATE TABLE tool_results (id INTEGER PRIMARY KEY AUTOINCREMENT, tool_call_row_id INTEGER, output_kind TEXT, output_json TEXT, success INTEGER, created_at TEXT);
              CREATE TABLE usage_events (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT, message_seq INTEGER, source TEXT, model TEXT, input_tokens INTEGER, output_tokens INTEGER, total_tokens INTEGER, cost_micros INTEGER, created_at TEXT);
              INSERT INTO workspaces VALUES ('w1','k','/home/u/p',NULL,'p','2026-01-01T00:00:00Z');
              INSERT INTO sessions VALUES ('g1','w1','Arrumar lint','grok-4','agent','/home/u/p','/home/u/p','done','2026-01-01T00:00:00Z','2026-01-01T00:10:00Z');
              INSERT INTO messages VALUES ('g1',0,'user','{"role":"user","content":"rode o lint"}','2026-01-01T00:00:01Z');
              INSERT INTO messages VALUES ('g1',1,'assistant','{"role":"assistant","content":[{"type":"text","text":"ok"},{"type":"tool-call","toolCallId":"c1","toolName":"bash","input":{"command":"npm run lint"}}]}','2026-01-01T00:00:02Z');
              INSERT INTO tool_calls VALUES (1,'g1',1,'c1','bash','{"command":"npm run lint"}','completed','x',NULL);
              INSERT INTO tool_results VALUES (1,1,'text','"0 problemas"',1,'x');
              INSERT INTO usage_events VALUES (1,'g1',1,'chat','grok-4',500,40,540,2500,'2026-01-01T00:00:03Z');
            "#).unwrap();
        }
        let ss = cli_sessions(&db, None);
        assert_eq!(ss.len(), 1);
        let s = &ss[0];
        assert_eq!(s.meta.project_path.as_deref(), Some("/home/u/p"));
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.turns[1].tool_calls.len(), 1);
        assert_eq!(
            s.turns[1].tool_calls[0].result.as_deref(),
            Some("0 problemas")
        );
        assert_eq!(s.meta.usage.input, 500);
        assert!((s.meta.cost_usd - 0.0025).abs() < 1e-9);
        std::fs::remove_dir_all(&dir).ok();
    }
}
