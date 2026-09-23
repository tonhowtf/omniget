//! Kiro (AWS): kiro-cli e IDE (estudo 06, Kiro §j e Kiro CLI §j; Parte 6 §8.2).
//!
//! Três lugares:
//! 1. kiro-cli 3.x (ACP/V3): `~/.kiro/sessions/cli/<id>.json` (estado:
//!    `session_id`, `cwd`, `session_state.rts_model_state.model_info.model_id`,
//!    `session_state.conversation_metadata.user_turn_metadatas[]` com
//!    `input_token_count`, `output_token_count`, `cache_read_input_token_count`,
//!    `cache_write_input_token_count`, `end_timestamp`, `message_ids[]`,
//!    `metering_usage[{value, unit:"credit"}]`) + `<id>.jsonl`
//!    (`{version, kind:"Prompt"|"AssistantMessage"|"ToolResults", data:{message_id,
//!    content[{kind:"text", data}|{kind:"toolUse", data}|{kind:"toolResult", data}],
//!    meta:{timestamp}}}`).
//! 2. IDE: `~/.kiro/sessions/<workspace>/sess_<uuid>/session.json` (`id`,
//!    `title`, `workspacePaths[]`, `createdAt`, `lastModifiedAt`, `modelId`) +
//!    `messages.jsonl` (`{payload:{type:"user"|"assistant"|"tool_call"|
//!    "tool_result"|"turn_end"|…}, timestamp}` ou `{role, content}`).
//! 3. kiro-cli 2.x / Amazon Q: SQLite `data.sqlite3` em `<dados>/kiro-cli`
//!    (macOS `~/Library/Application Support`, Linux `~/.local/share`), tabela
//!    `conversations_v2(key = cwd, conversation_id, value JSON)` (ou
//!    `conversations(key, value)` antiga): `model_info`, `history[]` com
//!    `user.content.{Prompt{prompt} | ToolUseResults{tool_use_results[]}}`,
//!    `assistant.{Response{content} | ToolUse{content, tool_uses[{id, name, args}]}}`
//!    e `request_metadata` (tokens quando existem, horários em ms).
//!
//! Kiro cobra em créditos, não em dólar nem por token: os créditos
//! (`metering_usage`) não são dinheiro gravado, então `Turn.cost_usd` fica
//! `None` e o custo, se houver tokens, sai da tabela de preços. Os tokens
//! costumam vir zerados no agente Auto. Kiro não está instalado aqui: só fixture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::util::*;
use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus, Turn,
};

pub struct KiroSource;

const TOOL: &str = "kiro";

fn kiro_home() -> Option<PathBuf> {
    env_dir("KIRO_HOME").or_else(|| home_join(&[".kiro"]))
}

fn cli_files() -> Vec<PathBuf> {
    let Some(h) = kiro_home() else {
        return Vec::new();
    };
    find_files(&h.join("sessions").join("cli"), 1, |p| {
        p.extension().and_then(|e| e.to_str()) == Some("json")
    })
}

fn ide_dirs() -> Vec<PathBuf> {
    let Some(h) = kiro_home() else {
        return Vec::new();
    };
    find_files(&h.join("sessions"), 3, |p| file_name(p) == "session.json")
        .into_iter()
        .filter_map(|f| f.parent().map(Path::to_path_buf))
        .filter(|d| file_name(d).starts_with("sess_"))
        .collect()
}

fn sqlite_dbs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    let mut bases = Vec::new();
    if let Some(d) = dirs::data_dir() {
        bases.push(d);
    }
    bases.extend(xdg_data_homes());
    for b in dedup_paths(bases) {
        for app in ["kiro-cli", "amazon-q"] {
            v.push(b.join(app).join("data.sqlite3"));
        }
    }
    dedup_paths(v).into_iter().filter(|p| p.is_file()).collect()
}

fn text_parts(
    content: &Value,
) -> (
    String,
    Vec<(String, String, Value)>,
    Vec<(String, String, bool)>,
) {
    let mut text = Vec::new();
    let mut uses = Vec::new();
    let mut results = Vec::new();
    for c in content.as_array().into_iter().flatten() {
        let kind = c.get("kind").and_then(Value::as_str).unwrap_or("");
        let data = c.get("data").cloned().unwrap_or(Value::Null);
        match kind {
            "text" => {
                if let Some(t) = data.as_str() {
                    text.push(t.to_string());
                }
            }
            "toolUse" => {
                uses.push((
                    s(&data, &["toolUseId", "id"]).unwrap_or_default(),
                    s(&data, &["name"]).unwrap_or_else(|| "unknown".into()),
                    data.get("input")
                        .or_else(|| data.get("args"))
                        .cloned()
                        .unwrap_or(Value::Null),
                ));
            }
            "toolResult" => {
                let body = data
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|p| match p.get("data") {
                                Some(Value::String(s)) => s.clone(),
                                Some(other) => other.to_string(),
                                None => String::new(),
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                let err = s(&data, &["status"])
                    .map(|x| x != "success")
                    .unwrap_or(false);
                results.push((
                    s(&data, &["toolUseId", "id"]).unwrap_or_default(),
                    body,
                    err,
                ));
            }
            _ => {}
        }
    }
    (text.join("\n"), uses, results)
}

fn turn_usage(m: &Value) -> TokenUsage {
    TokenUsage {
        input: u(
            m,
            &["input_token_count", "input_tokens", "uncached_input_tokens"],
        ),
        output: u(m, &["output_token_count", "output_tokens"]),
        cache_read: u(
            m,
            &[
                "cache_read_input_token_count",
                "cache_read_input_tokens",
                "cache_read_tokens",
            ],
        ),
        cache_write: u(
            m,
            &[
                "cache_write_input_token_count",
                "cache_write_input_tokens",
                "cache_creation_input_tokens",
                "cache_write_tokens",
            ],
        ),
        reasoning: u(m, &["reasoning_tokens", "thinking_tokens"]),
    }
}

/// kiro-cli 3.x: estado `.json` + log `.jsonl`.
pub fn parse_cli(state: &Value, lines: &[Value], fallback_id: &str, source: &Path) -> Session {
    let id = s(state, &["session_id", "id"]).unwrap_or_else(|| fallback_id.to_string());
    let mut meta = new_meta(TOOL, id, source);
    meta.account = Some("cli".into());
    meta.project_path = s(state, &["cwd"]);
    let model = state
        .pointer("/session_state/rts_model_state/model_info/model_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut turns: Vec<Turn> = Vec::new();
    let mut msg_turn: HashMap<String, usize> = HashMap::new();
    let mut results = Vec::new();
    for l in lines {
        let kind = l.get("kind").and_then(Value::as_str).unwrap_or("");
        let data = l.get("data").cloned().unwrap_or(Value::Null);
        let ts = data
            .pointer("/meta/timestamp")
            .and_then(ts_str)
            .unwrap_or_default();
        let (text, uses, res) = text_parts(data.get("content").unwrap_or(&Value::Null));
        results.extend(res);
        let role = match kind {
            "Prompt" => Role::User,
            "AssistantMessage" => Role::Assistant,
            _ => continue,
        };
        let mut t = turn(role, ts, text);
        t.message_id = s(&data, &["message_id"]);
        if role == Role::Assistant {
            t.model = model.clone();
            for (cid, name, input) in uses {
                t.tool_calls.push(tool_call(cid, &name, input));
            }
        }
        if let Some(mid) = &t.message_id {
            msg_turn.insert(mid.clone(), turns.len());
        }
        turns.push(t);
    }
    attach_results(&mut turns, results);
    let metas = state
        .pointer("/session_state/conversation_metadata/user_turn_metadatas")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for m in &metas {
        let us = turn_usage(m);
        // Turno do assistente cujo message_id está na lista; senão, o último.
        let target = m
            .get("message_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|x| msg_turn.get(x).copied())
            .filter(|i| turns[*i].role == Role::Assistant)
            .last();
        match target {
            Some(i) => turns[i].usage.add(&us),
            None if us.total() > 0 => {
                let ts = m.get("end_timestamp").and_then(ts_str).unwrap_or_default();
                let mut t = turn(Role::Assistant, ts, "");
                t.usage = us;
                t.model = model.clone();
                turns.push(t);
            }
            None => {}
        }
    }
    for t in turns.iter_mut() {
        for c in t.tool_calls.iter_mut() {
            if c.status == ToolStatus::Pending && c.result.is_some() {
                c.status = ToolStatus::Ok;
            }
        }
    }
    finalize(&mut meta, &mut turns);
    if meta.models.is_empty() {
        meta.models.extend(model);
    }
    Session { meta, turns }
}

/// IDE: `session.json` + `messages.jsonl`.
pub fn parse_ide(sess: &Value, lines: &[Value], fallback_id: &str, source: &Path) -> Session {
    let id = s(sess, &["id"]).unwrap_or_else(|| fallback_id.to_string());
    let mut meta = new_meta(TOOL, id, source);
    meta.account = Some("ide".into());
    meta.title = s(sess, &["title"]);
    meta.project_path = sess
        .get("workspacePaths")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .map(str::to_string);
    meta.started = sess.get("createdAt").and_then(ts_str);
    meta.ended = sess.get("lastModifiedAt").and_then(ts_str);
    let model = s(sess, &["modelId"]);
    let mut turns: Vec<Turn> = Vec::new();
    let mut results = Vec::new();
    for l in lines {
        let ts = l.get("timestamp").and_then(ts_str).unwrap_or_default();
        let (ty, p) = match l.get("payload") {
            Some(p) => (
                p.get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                p.clone(),
            ),
            None => (
                l.get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                l.clone(),
            ),
        };
        match ty.as_str() {
            "user" => turns.push(turn(
                Role::User,
                ts,
                p.get("content").map(content_text).unwrap_or_default(),
            )),
            "assistant" => {
                let mut t = turn(
                    Role::Assistant,
                    ts,
                    p.get("content").map(content_text).unwrap_or_default(),
                );
                t.model = model.clone();
                turns.push(t);
            }
            "tool_call" => {
                let name = s(&p, &["name", "toolName", "tool"]).unwrap_or_else(|| "unknown".into());
                let input = p
                    .get("args")
                    .or_else(|| p.get("input"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let c = tool_call(
                    s(&p, &["id", "toolCallId", "toolUseId"]).unwrap_or_default(),
                    &name,
                    input,
                );
                if !matches!(turns.last(), Some(t) if t.role == Role::Assistant) {
                    let mut t = turn(Role::Assistant, ts, "");
                    t.model = model.clone();
                    turns.push(t);
                }
                turns.last_mut().expect("existe").tool_calls.push(c);
            }
            "tool_result" => {
                results.push((
                    s(&p, &["id", "toolCallId", "toolUseId"]).unwrap_or_default(),
                    p.get("content").map(result_text).unwrap_or_default(),
                    p.get("isError").and_then(Value::as_bool).unwrap_or(false),
                ));
            }
            _ => {}
        }
    }
    attach_results(&mut turns, results);
    finalize(&mut meta, &mut turns);
    if meta.models.is_empty() {
        meta.models.extend(model);
    }
    Session { meta, turns }
}

/// kiro-cli 2.x / Amazon Q: valor JSON de uma conversa do SQLite.
pub fn parse_db_conversation(v: &Value, id: &str, cwd: Option<String>, source: &Path) -> Session {
    let mut meta = new_meta(TOOL, id, source);
    meta.account = Some("cli-db".into());
    meta.project_path = cwd;
    let model = v
        .pointer("/model_info/model_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut turns: Vec<Turn> = Vec::new();
    let mut results = Vec::new();
    for item in v
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (user, asst, rm) = match item {
            Value::Array(a) => (a.first().cloned(), a.get(1).cloned(), None),
            o => (
                o.get("user").cloned(),
                o.get("assistant").cloned(),
                o.get("request_metadata").cloned(),
            ),
        };
        let rm = rm.unwrap_or(Value::Null);
        let ts = rm
            .get("request_start_timestamp_ms")
            .or_else(|| user.as_ref().and_then(|u_| u_.get("timestamp")))
            .and_then(ts_str)
            .unwrap_or_default();
        if let Some(u_) = user {
            let c = u_.get("content").cloned().unwrap_or(Value::Null);
            if let Some(p) = c.pointer("/Prompt/prompt").and_then(Value::as_str) {
                turns.push(turn(Role::User, ts.clone(), p));
            }
            for r in c
                .pointer("/ToolUseResults/tool_use_results")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let body = r
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|p| match (p.get("Text"), p.get("Json")) {
                                (Some(Value::String(t)), _) => t.clone(),
                                (_, Some(j)) => j.to_string(),
                                _ => p.to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                let err = s(r, &["status"])
                    .map(|x| x.eq_ignore_ascii_case("error"))
                    .unwrap_or(false);
                results.push((s(r, &["tool_use_id"]).unwrap_or_default(), body, err));
            }
        }
        if let Some(a) = asst {
            let (body, uses) = if let Some(r) = a.get("Response") {
                (s(r, &["content"]).unwrap_or_default(), Vec::new())
            } else if let Some(tu) = a.get("ToolUse") {
                (
                    s(tu, &["content"]).unwrap_or_default(),
                    tu.get("tool_uses")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                )
            } else {
                (content_text(&a), Vec::new())
            };
            let mut t = turn(Role::Assistant, ts, body);
            t.model = model.clone();
            for x in uses {
                let name = s(&x, &["name"]).unwrap_or_else(|| "unknown".into());
                let args = x
                    .get("args")
                    .or_else(|| x.get("orig_args"))
                    .cloned()
                    .unwrap_or(Value::Null);
                t.tool_calls
                    .push(tool_call(s(&x, &["id"]).unwrap_or_default(), &name, args));
            }
            let mut us = turn_usage(&rm);
            if us.total() == 0 {
                if let Some(nested) = rm.get("token_usage").or_else(|| rm.get("usage")) {
                    us = turn_usage(nested);
                }
            }
            t.usage = us;
            turns.push(t);
        }
    }
    attach_results(&mut turns, results);
    for t in turns.iter_mut() {
        for c in t.tool_calls.iter_mut() {
            if c.status == ToolStatus::Pending && c.result.is_some() {
                c.status = ToolStatus::Ok;
            }
        }
    }
    finalize(&mut meta, &mut turns);
    if meta.models.is_empty() {
        meta.models.extend(model);
    }
    Session { meta, turns }
}

fn db_sessions(db: &Path, only: Option<&str>) -> Vec<Session> {
    let Some(conn) = open_ro(db) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if table_exists(&conn, "conversations_v2") {
        let sql = if only.is_some() {
            "SELECT key, conversation_id, value FROM conversations_v2 WHERE conversation_id = ?1"
        } else {
            "SELECT key, conversation_id, value FROM conversations_v2"
        };
        if let Ok(mut st) = conn.prepare(sql) {
            let map = |r: &rusqlite::Row| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
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
            for (cwd, cid, val) in rows {
                let Some(v) = val.and_then(|x| serde_json::from_str::<Value>(&x).ok()) else {
                    continue;
                };
                let id = cid
                    .or_else(|| s(&v, &["conversation_id"]))
                    .unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                out.push(parse_db_conversation(&v, &id, cwd, db));
            }
        }
    } else if table_exists(&conn, "conversations") {
        if let Ok(mut st) = conn.prepare("SELECT key, value FROM conversations") {
            if let Ok(rows) = st.query_map([], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            }) {
                for (cwd, val) in rows.flatten() {
                    let Some(v) = val.and_then(|x| serde_json::from_str::<Value>(&x).ok()) else {
                        continue;
                    };
                    let id = s(&v, &["conversation_id"])
                        .unwrap_or_else(|| cwd.clone().unwrap_or_default());
                    if only.map(|o| o != id).unwrap_or(false) {
                        continue;
                    }
                    out.push(parse_db_conversation(&v, &id, cwd, db));
                }
            }
        }
    }
    out
}

fn load_cli_file(f: &Path) -> Option<Session> {
    let state = read_json(f)?;
    let lines = read_jsonl(&f.with_extension("jsonl"));
    Some(parse_cli(&state, &lines, &file_stem(f), f))
}

fn load_ide_dir(d: &Path) -> Option<Session> {
    let sess = read_json(&d.join("session.json"))?;
    let lines = read_jsonl(&d.join("messages.jsonl"));
    Some(parse_ide(
        &sess,
        &lines,
        &file_name(d),
        &d.join("session.json"),
    ))
}

impl SessionSource for KiroSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = kiro_home()
            .map(|h| h.join("sessions"))
            .filter(|p| p.is_dir())
            .into_iter()
            .collect();
        v.extend(sqlite_dbs());
        v
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out: Vec<SessionMeta> = cli_files()
            .iter()
            .filter_map(|f| load_cli_file(f))
            .map(|s| s.meta)
            .collect();
        out.extend(
            ide_dirs()
                .iter()
                .filter_map(|d| load_ide_dir(d))
                .map(|s| s.meta),
        );
        for db in sqlite_dbs() {
            out.extend(db_sessions(&db, None).into_iter().map(|s| s.meta));
        }
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        if let Some(f) = cli_files().into_iter().find(|f| file_stem(f) == id) {
            return load_cli_file(&f);
        }
        if let Some(d) = ide_dirs().into_iter().find(|d| file_name(d) == id) {
            return load_ide_dir(&d);
        }
        for db in sqlite_dbs() {
            if let Some(s) = db_sessions(&db, Some(id)).into_iter().next() {
                return Some(s);
            }
        }
        None
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        match meta.account.as_deref() {
            Some("cli") | Some("cli-db") => Some(format!(
                "kiro-cli chat --resume-id {}",
                shell_quote(&meta.id)
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_fixture() {
        let state: Value = serde_json::from_str(r#"{"session_id":"session-1","cwd":"/tmp/project","session_state":{"rts_model_state":{"model_info":{"model_id":"claude-sonnet-4-5"}},"conversation_metadata":{"user_turn_metadatas":[{"input_token_count":120,"output_token_count":30,"cache_read_input_token_count":10,"end_timestamp":1770983427,"message_ids":["prompt-1","assistant-1"],"metering_usage":[{"value":0.0313,"unit":"credit"}]}]}}}"#).unwrap();
        let lines: Vec<Value> = [
            r#"{"version":"v1","kind":"Prompt","data":{"message_id":"prompt-1","content":[{"kind":"text","data":"hello world"}],"meta":{"timestamp":1770983426.420942}}}"#,
            r#"{"version":"v1","kind":"AssistantMessage","data":{"message_id":"assistant-1","content":[{"kind":"text","data":"response text"},{"kind":"toolUse","data":{"toolUseId":"t1","name":"read","input":{"path":"a"}}}]}}"#,
            r#"{"version":"v1","kind":"ToolResults","data":{"message_id":"toolresults-tr","content":[{"kind":"toolResult","data":{"toolUseId":"t1","content":[{"kind":"text","data":"conteúdo"}],"status":"success"}}]}}"#,
        ]
        .iter()
        .map(|x| serde_json::from_str(x).unwrap())
        .collect();
        let s = parse_cli(&state, &lines, "x", Path::new("/x/session-1.json"));
        assert_eq!(s.meta.id, "session-1");
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.turns[1].usage.input, 120);
        assert_eq!(s.turns[1].cost_usd, None);
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Read");
        assert_eq!(s.turns[1].tool_calls[0].result.as_deref(), Some("conteúdo"));
        assert_eq!(s.meta.models, vec!["claude-sonnet-4-5".to_string()]);
    }

    #[test]
    fn db_fixture() {
        let v: Value = serde_json::from_str(r#"{"conversation_id":"c1","model_info":{"model_id":"auto"},"history":[
          {"user":{"content":{"Prompt":{"prompt":"liste arquivos"}}},"assistant":{"ToolUse":{"message_id":"m1","content":"vou listar","tool_uses":[{"id":"tu1","name":"execute_bash","args":{"command":"ls"}}]}},"request_metadata":{"request_start_timestamp_ms":1770983426000,"input_tokens":50,"output_tokens":10}},
          {"user":{"content":{"ToolUseResults":{"tool_use_results":[{"tool_use_id":"tu1","content":[{"Text":"a.txt"}],"status":"Success"}]}}},"assistant":{"Response":{"message_id":"m2","content":"Tem a.txt"}},"request_metadata":{"request_start_timestamp_ms":1770983427000}}
        ]}"#).unwrap();
        let s = parse_db_conversation(&v, "c1", Some("/p".into()), Path::new("/x/data.sqlite3"));
        assert_eq!(s.turns.len(), 3);
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Bash");
        assert_eq!(s.turns[1].tool_calls[0].result.as_deref(), Some("a.txt"));
        assert_eq!(s.meta.usage.input, 50);
        assert_eq!(s.meta.project_path.as_deref(), Some("/p"));
    }
}
