//! Zed Agent: `threads/threads.db` (estudo 06, Zed §j; Parte 6 §8.2).
//!
//! Caminho (`paths::data_dir()/threads`): macOS
//! `~/Library/Application Support/Zed`, Linux `$XDG_DATA_HOME/zed`
//! (`~/.local/share/zed`), Windows `%LOCALAPPDATA%\Zed`.
//! Tabela `threads(id, summary, updated_at, data_type 'json'|'zstd', data,
//! parent_id?, folder_paths?, folder_paths_order?, created_at?)`. `data` é o
//! `DbThread` em JSON (comprimido com zstd quando `data_type = 'zstd'`):
//! `title`, `messages[]` (`{"User":{id, content:[{"Text":…}|{"Mention":…}]}}`,
//! `{"Agent":{content:[{"Text"}|{"Thinking"}|{"ToolUse":{id,name,input}}],
//! tool_results:{<id>:{is_error, content, output}}}}`, `"Resume"`; o formato
//! antigo 0.1/0.2 usa `{role, segments[], tool_uses[], tool_results[]}`),
//! `request_token_usage` (id da mensagem do usuário → uso),
//! `cumulative_token_usage`, `model{provider, model}`, `imported`.
//! Threads importadas de agentes externos (ACP) ficam sem uso para não contar
//! em dobro com o log do próprio agente. Zed não está instalado aqui: só fixture.

use std::path::PathBuf;

use serde_json::Value;

use super::util::*;
use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus, Turn,
};

pub struct ZedSource;

const TOOL: &str = "zed";

fn db_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(target_os = "macos") {
        if let Some(d) = dirs::data_dir() {
            v.push(d.join("Zed"));
        }
    } else if cfg!(windows) {
        if let Some(d) = dirs::data_local_dir() {
            v.push(d.join("Zed"));
        }
    } else {
        for d in xdg_data_homes() {
            v.push(d.join("zed"));
        }
    }
    if let Some(d) = env_dir("ZED_DATA_DIR") {
        v.insert(0, d);
    }
    dedup_paths(v)
        .into_iter()
        .map(|d| d.join("threads").join("threads.db"))
        .filter(|p| p.is_file())
        .collect()
}

fn zed_usage(v: &Value) -> TokenUsage {
    TokenUsage {
        input: u(v, &["input_tokens"]),
        output: u(v, &["output_tokens"]),
        cache_read: u(v, &["cache_read_input_tokens"]),
        cache_write: u(v, &["cache_creation_input_tokens"]),
        reasoning: 0,
    }
}

fn decode(data_type: &str, data: &[u8]) -> Option<Value> {
    if data_type == "zstd" {
        let raw = zstd::stream::decode_all(data).ok()?;
        serde_json::from_slice(&raw).ok()
    } else {
        serde_json::from_slice(data).ok()
    }
}

fn user_content_text(c: &Value) -> String {
    let mut out = Vec::new();
    for part in c.as_array().into_iter().flatten() {
        if let Some(t) = part.get("Text").and_then(Value::as_str) {
            out.push(t.to_string());
        } else if let Some(m) = part.get("Mention") {
            if let Some(u) = m.get("uri") {
                out.push(format!(
                    "@{}",
                    match u {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    }
                ));
            }
        } else if let Some(t) = part.as_str() {
            out.push(t.to_string());
        }
    }
    out.join("\n")
}

/// `DbThread` (JSON já descomprimido) em turnos.
pub fn thread_turns(doc: &Value, ts: &str) -> Vec<Turn> {
    let model = doc
        .pointer("/model/model")
        .and_then(Value::as_str)
        .map(str::to_string);
    let imported = doc
        .get("imported")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let req_usage = doc.get("request_token_usage");
    let mut turns: Vec<Turn> = Vec::new();
    let mut last_user_id: Option<String> = None;
    for m in doc
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(u) = m.get("User") {
            let mut t = turn(
                Role::User,
                ts,
                user_content_text(u.get("content").unwrap_or(&Value::Null)),
            );
            t.message_id = s(u, &["id"]);
            last_user_id = t.message_id.clone();
            turns.push(t);
        } else if let Some(a) = m.get("Agent") {
            let mut t = turn(Role::Assistant, ts, "");
            t.model = model.clone();
            let mut texts = Vec::new();
            for part in a
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(x) = part.get("Text").and_then(Value::as_str) {
                    texts.push(x.to_string());
                } else if let Some(tu) = part.get("ToolUse") {
                    let name = s(tu, &["name"]).unwrap_or_else(|| "unknown".into());
                    let input = tu.get("input").cloned().unwrap_or_else(|| {
                        tu.get("raw_input")
                            .and_then(Value::as_str)
                            .and_then(|r| serde_json::from_str(r).ok())
                            .unwrap_or(Value::Null)
                    });
                    t.tool_calls
                        .push(tool_call(s(tu, &["id"]).unwrap_or_default(), &name, input));
                }
            }
            t.text = texts.join("\n");
            if let Some(Value::Object(res)) = a.get("tool_results") {
                for c in t.tool_calls.iter_mut() {
                    if let Some(r) = res.get(&c.id) {
                        let body = r
                            .get("content")
                            .map(|x| {
                                x.get("Text")
                                    .and_then(Value::as_str)
                                    .map(str::to_string)
                                    .unwrap_or_else(|| result_text(x))
                            })
                            .unwrap_or_default();
                        c.result = Some(body);
                        c.status = if r.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
                            ToolStatus::Error
                        } else {
                            ToolStatus::Ok
                        };
                    }
                }
            }
            // Uso do pedido disparado pela última mensagem do usuário.
            if !imported {
                if let (Some(uid), Some(Value::Object(map))) = (&last_user_id, req_usage) {
                    if let Some(us) = map.get(uid) {
                        t.usage = zed_usage(us);
                        last_user_id = None;
                    }
                }
            }
            turns.push(t);
        } else if m.get("role").is_some() {
            // Formato antigo (0.1/0.2).
            let role = role_of(m.get("role").and_then(Value::as_str).unwrap_or(""));
            let mut texts = Vec::new();
            for seg in m
                .get("segments")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if seg.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(x) = seg.get("text").and_then(Value::as_str) {
                        texts.push(x.to_string());
                    }
                }
            }
            let mut t = turn(role, ts, texts.join("\n"));
            if role == Role::Assistant {
                t.model = model.clone();
            }
            for tu in m
                .get("tool_uses")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let name = s(tu, &["name"]).unwrap_or_else(|| "unknown".into());
                t.tool_calls.push(tool_call(
                    s(tu, &["id"]).unwrap_or_default(),
                    &name,
                    tu.get("input").cloned().unwrap_or(Value::Null),
                ));
            }
            let results: Vec<(String, String, bool)> = m
                .get("tool_results")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|r| {
                    (
                        s(r, &["tool_use_id"]).unwrap_or_default(),
                        r.get("content").map(result_text).unwrap_or_default(),
                        r.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    )
                })
                .collect();
            turns.push(t);
            attach_results(&mut turns, results);
        }
    }
    // Sem uso por pedido: o acumulado vai para o último turno do assistente.
    let per_req: u64 = turns.iter().map(|t| t.usage.total()).sum();
    if per_req == 0 && !imported {
        if let Some(c) = doc.get("cumulative_token_usage") {
            let us = zed_usage(c);
            if us.total() > 0 {
                match turns.iter().rposition(|t| t.role == Role::Assistant) {
                    Some(i) => turns[i].usage = us,
                    None => {
                        let mut t = turn(Role::Assistant, ts, "");
                        t.usage = us;
                        t.model = model.clone();
                        turns.push(t);
                    }
                }
            }
        }
    }
    turns
}

struct Row {
    id: String,
    summary: Option<String>,
    updated_at: Option<String>,
    created_at: Option<String>,
    parent_id: Option<String>,
    folder_paths: Option<String>,
    folder_order: Option<String>,
    data_type: String,
    data: Vec<u8>,
}

fn rows(db: &std::path::Path, only: Option<&str>) -> Vec<Row> {
    let Some(conn) = open_ro(db) else {
        return Vec::new();
    };
    let cols = table_columns(&conn, "threads");
    if cols.is_empty() {
        return Vec::new();
    }
    let pick = |c: &str| {
        if cols.iter().any(|x| x == c) {
            c.to_string()
        } else {
            "NULL".into()
        }
    };
    let mut sql = format!(
        "SELECT id, {}, {}, {}, {}, {}, {}, {}, data FROM threads",
        pick("summary"),
        pick("updated_at"),
        pick("created_at"),
        pick("parent_id"),
        pick("folder_paths"),
        pick("folder_paths_order"),
        pick("data_type"),
    );
    if only.is_some() {
        sql.push_str(" WHERE id = ?1");
    }
    let Ok(mut st) = conn.prepare(&sql) else {
        return Vec::new();
    };
    let map = |r: &rusqlite::Row| -> rusqlite::Result<Row> {
        let data: Vec<u8> = match r.get::<_, rusqlite::types::Value>(8)? {
            rusqlite::types::Value::Blob(b) => b,
            rusqlite::types::Value::Text(t) => t.into_bytes(),
            _ => Vec::new(),
        };
        Ok(Row {
            id: r.get(0)?,
            summary: r.get(1).ok().flatten(),
            updated_at: r.get(2).ok().flatten(),
            created_at: r.get(3).ok().flatten(),
            parent_id: r.get(4).ok().flatten(),
            folder_paths: r.get(5).ok().flatten(),
            folder_order: r.get(6).ok().flatten(),
            data_type: r
                .get::<_, Option<String>>(7)
                .ok()
                .flatten()
                .unwrap_or_else(|| "json".into()),
            data,
        })
    };
    let res = match only {
        Some(id) => st.query_map([id], map).map(|r| r.flatten().collect()),
        None => st.query_map([], map).map(|r| r.flatten().collect()),
    };
    res.unwrap_or_default()
}

/// Pasta do projeto: `folder_paths` separado por quebra de linha, e
/// `folder_paths_order` (posições separadas por vírgula) diz qual vem primeiro.
fn primary_folder(paths: &str, order: Option<&str>) -> Option<String> {
    let list: Vec<&str> = paths
        .lines()
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect();
    if list.is_empty() {
        return None;
    }
    if let Some(o) = order {
        let ranks: Vec<usize> = o.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if ranks.len() == list.len() {
            if let Some((i, _)) = ranks.iter().enumerate().min_by_key(|(_, r)| **r) {
                return Some(list[i].to_string());
            }
        }
    }
    Some(list[0].to_string())
}

fn to_session(db: &std::path::Path, r: Row) -> Option<Session> {
    let doc = decode(&r.data_type, &r.data)?;
    let mut meta = new_meta(TOOL, r.id, db);
    meta.title = r
        .summary
        .filter(|x| !x.trim().is_empty())
        .or_else(|| s(&doc, &["title"]));
    meta.started = r
        .created_at
        .as_deref()
        .and_then(str_to_ms)
        .map(ms_to_rfc3339);
    let upd = r
        .updated_at
        .as_deref()
        .and_then(str_to_ms)
        .or_else(|| doc.get("updated_at").and_then(ts_ms));
    if let Some(u) = upd {
        meta.ended = Some(ms_to_rfc3339(u));
        meta.mtime_ms = u;
    }
    meta.parent_session = r.parent_id;
    meta.project_path = r
        .folder_paths
        .as_deref()
        .and_then(|p| primary_folder(p, r.folder_order.as_deref()));
    meta.account = doc
        .pointer("/model/provider")
        .and_then(Value::as_str)
        .map(str::to_string);
    let ts = meta
        .started
        .clone()
        .or(meta.ended.clone())
        .unwrap_or_default();
    let mut turns = thread_turns(&doc, &ts);
    finalize(&mut meta, &mut turns);
    Some(Session { meta, turns })
}

impl SessionSource for ZedSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        db_paths()
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out = Vec::new();
        for db in db_paths() {
            for r in rows(&db, None) {
                if let Some(s) = to_session(&db, r) {
                    out.push(s.meta);
                }
            }
        }
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        for db in db_paths() {
            if let Some(r) = rows(&db, Some(id)).into_iter().next() {
                return to_session(&db, r);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"{"version":"0.3.0","title":"Refatorar","updated_at":"2026-05-01T12:30:00Z",
      "messages":[
        {"User":{"id":"user-1","content":[{"Text":"refatore o loader"}]}},
        {"Agent":{"content":[{"Text":"Lendo."},{"ToolUse":{"id":"t1","name":"read_file","input":{"path":"a.rs"},"is_input_complete":true}}],
                  "tool_results":{"t1":{"tool_use_id":"t1","tool_name":"read_file","is_error":false,"content":{"Text":"fn main(){}"}}}}},
        "Resume"
      ],
      "request_token_usage":{"user-1":{"input_tokens":100,"output_tokens":20,"cache_creation_input_tokens":5,"cache_read_input_tokens":10}},
      "cumulative_token_usage":{"input_tokens":999,"output_tokens":999},
      "model":{"provider":"zed.dev","model":"claude-sonnet-4-5"}}"#;

    #[test]
    fn thread_fixture() {
        let doc: Value = serde_json::from_str(DOC).unwrap();
        let t = thread_turns(&doc, "2026-05-01T12:00:00Z");
        assert_eq!(t.len(), 2);
        assert_eq!(t[1].usage.input, 100);
        assert_eq!(t[1].usage.cache_read, 10);
        assert_eq!(t[1].tool_calls[0].name_canonical, "Read");
        assert_eq!(t[1].tool_calls[0].result.as_deref(), Some("fn main(){}"));
        assert_eq!(t[1].model.as_deref(), Some("claude-sonnet-4-5"));
    }

    #[test]
    fn zstd_roundtrip_and_folder() {
        let packed = zstd::stream::encode_all(DOC.as_bytes(), 3).unwrap();
        assert!(decode("zstd", &packed).is_some());
        assert!(decode("zstd", b"lixo").is_none());
        assert_eq!(primary_folder("/a\n/b", Some("1,0")).as_deref(), Some("/b"));
    }
}
