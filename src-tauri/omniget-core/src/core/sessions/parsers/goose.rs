//! Goose (aaif-goose): SQLite `sessions.db` com custo gravado.
//!
//! Caminhos: `$GOOSE_PATH_ROOT/data/sessions/sessions.db`,
//! `~/.local/share/goose/sessions/sessions.db`,
//! `~/Library/Application Support/goose/sessions/sessions.db`,
//! `~/.local/share/Block/goose/sessions/sessions.db` e, no Windows,
//! `%APPDATA%\Block\goose\data\sessions\sessions.db`.
//!
//! Tabelas: `sessions(id, name, description, working_dir, created_at,
//! updated_at, accumulated_*_tokens, accumulated_cost, parent_session_id…)`,
//! `messages(id, message_id, session_id, role, content_json,
//! created_timestamp, …)` e `usage_ledger(session_id, created_timestamp,
//! model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
//! cost, …)`. Cada linha do ledger é uma chamada: o uso e o custo vão para a
//! última resposta do assistente até aquele instante. Sem ledger (versão
//! antiga), o acumulado da sessão vai para a última resposta.
//! `input_tokens` é tratado como entrada total (cache incluso).

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolCall, ToolStatus, Turn,
};
use crate::core::sessions::util::{self, canonical_tool, truncate_chars, value_text, RESULT_MAX};

pub const TOOL: &str = "goose";

pub struct GooseSource {
    candidates: Vec<PathBuf>,
}

impl Default for GooseSource {
    fn default() -> Self {
        Self::new()
    }
}

impl GooseSource {
    pub fn new() -> Self {
        let mut c = Vec::new();
        if let Some(root) = std::env::var_os("GOOSE_PATH_ROOT").filter(|v| !v.is_empty()) {
            c.push(
                PathBuf::from(root)
                    .join("data")
                    .join("sessions")
                    .join("sessions.db"),
            );
        }
        if let Some(d) = util::xdg_data_home() {
            c.push(d.join("goose").join("sessions").join("sessions.db"));
            c.push(
                d.join("Block")
                    .join("goose")
                    .join("sessions")
                    .join("sessions.db"),
            );
        }
        if let Some(d) = dirs::data_dir() {
            c.push(d.join("goose").join("sessions").join("sessions.db"));
            c.push(
                d.join("Block")
                    .join("goose")
                    .join("data")
                    .join("sessions")
                    .join("sessions.db"),
            );
        }
        c.dedup();
        Self { candidates: c }
    }

    pub fn with_dbs(dbs: Vec<PathBuf>) -> Self {
        Self { candidates: dbs }
    }

    fn dbs(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for p in &self.candidates {
            if p.is_file() && !out.iter().any(|x| util::canon(x) == util::canon(p)) {
                out.push(p.clone());
            }
        }
        out
    }
}

fn i(m: &Map<String, Value>, k: &str) -> u64 {
    util::map_i64(m, k).unwrap_or(0).max(0) as u64
}

fn session_usage(r: &Map<String, Value>) -> TokenUsage {
    let acc = r.contains_key("accumulated_input_tokens")
        && util::map_i64(r, "accumulated_input_tokens").is_some();
    let (inp, out, cr, cw) = if acc {
        (
            i(r, "accumulated_input_tokens"),
            i(r, "accumulated_output_tokens"),
            i(r, "accumulated_cache_read_tokens"),
            i(r, "accumulated_cache_write_tokens"),
        )
    } else {
        (
            i(r, "input_tokens"),
            i(r, "output_tokens"),
            i(r, "cache_read_tokens"),
            i(r, "cache_write_tokens"),
        )
    };
    normalize(inp, out, cr, cw)
}

fn normalize(inp: u64, out: u64, cr: u64, cw: u64) -> TokenUsage {
    let cr = cr.min(inp);
    let cw = cw.min(inp - cr);
    TokenUsage {
        input: inp - cr - cw,
        output: out,
        cache_read: cr,
        cache_write: cw,
        reasoning: 0,
    }
}

fn meta_of(db: &Path, r: &Map<String, Value>) -> SessionMeta {
    let created = util::map_ts(r, "created_at");
    let updated = util::map_ts(r, "updated_at").or(created);
    let model = match util::json_col(r, "model_config_json") {
        Value::Object(o) => o
            .get("model_name")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        _ => None,
    };
    SessionMeta {
        tool: TOOL.into(),
        account: None,
        id: util::map_str(r, "id").unwrap_or_default(),
        title: util::map_str(r, "name")
            .or_else(|| util::map_str(r, "description"))
            .map(|t| truncate_chars(&t, 120)),
        project_path: util::map_str(r, "working_dir"),
        git_branch: None,
        started: created.map(util::ms_to_rfc3339),
        ended: updated.map(util::ms_to_rfc3339),
        models: model.into_iter().collect(),
        parent_session: util::map_str(r, "parent_session_id"),
        turn_count: 0,
        usage: session_usage(r),
        cost_usd: util::map_f64(r, "accumulated_cost").unwrap_or(0.0),
        source: db.to_path_buf(),
        mtime_ms: updated.unwrap_or_else(|| util::file_mtime_ms(db)),
    }
}

fn load_db(db: &Path, id: &str) -> Option<Session> {
    let c = util::open_ro(db)?;
    let rows = util::query_json(&c, "SELECT * FROM sessions WHERE id = ?1", &[&id]);
    let row = rows.first()?;
    let mut meta = meta_of(db, row);
    let ts_col = if util::table_columns(&c, "messages")
        .iter()
        .any(|n| n == "created_timestamp")
    {
        "created_timestamp"
    } else {
        "timestamp"
    };
    let msgs = util::query_json(
        &c,
        &format!("SELECT * FROM messages WHERE session_id = ?1 ORDER BY {ts_col}, id"),
        &[&id],
    );
    let mut turns: Vec<Turn> = Vec::new();
    let mut calls: std::collections::HashMap<String, (usize, usize)> = Default::default();
    for m in msgs {
        let role = match util::map_str(&m, "role").as_deref() {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => Role::System,
        };
        let ts_ms = util::map_ts(&m, ts_col).or_else(|| util::map_ts(&m, "timestamp"));
        let ts = ts_ms.map(util::ms_to_rfc3339).unwrap_or_default();
        let content = match util::json_col(&m, "content_json") {
            Value::Array(a) => a,
            _ => Vec::new(),
        };
        let mut text = Vec::new();
        let mut tcs = Vec::new();
        let mut only_results = true;
        for part in &content {
            match util::str_of(part, "type") {
                Some("text") => {
                    only_results = false;
                    if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                        text.push(t.to_string());
                    }
                }
                Some("toolRequest") => {
                    only_results = false;
                    let value = part.get("toolCall").and_then(|t| t.get("value"));
                    let name = value
                        .and_then(|v| util::str_of(v, "name"))
                        .unwrap_or("?")
                        .to_string();
                    let input = value
                        .and_then(|v| v.get("arguments"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    let mut canon = canonical_tool(&name);
                    // `developer__text_editor` com `command: view|write`.
                    if name.ends_with("text_editor") {
                        canon = match util::str_of(&input, "command") {
                            Some("view") => "Read",
                            Some("write") | Some("create") => "Write",
                            _ => "Edit",
                        };
                    }
                    tcs.push(ToolCall {
                        id: util::str_of(part, "id").unwrap_or("").to_string(),
                        name_canonical: canon.to_string(),
                        subagent: (canon == "Agent").then(|| name.clone()),
                        name_raw: name,
                        input,
                        result: None,
                        status: ToolStatus::Pending,
                        ms: None,
                    });
                }
                Some("toolResponse") => {
                    let id = util::str_of(part, "id").unwrap_or("").to_string();
                    let tr = part.get("toolResult");
                    let ok = tr.and_then(|t| util::str_of(t, "status")) != Some("error");
                    let val = tr
                        .and_then(|t| t.get("value").or_else(|| t.get("error")))
                        .cloned()
                        .unwrap_or(Value::Null);
                    if let Some(&(ti, ci)) = calls.get(&id) {
                        let start = util::parse_ts_str(&turns[ti].ts);
                        let call = &mut turns[ti].tool_calls[ci];
                        call.result = Some(value_text(&val, RESULT_MAX));
                        call.status = if ok {
                            ToolStatus::Ok
                        } else {
                            ToolStatus::Error
                        };
                        if let (Some(a), Some(b)) = (start, ts_ms) {
                            if b >= a {
                                call.ms = Some((b - a) as u64);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let text = text.join("\n");
        if (only_results && tcs.is_empty()) || (text.trim().is_empty() && tcs.is_empty()) {
            continue;
        }
        turns.push(Turn {
            role,
            ts,
            text,
            tool_calls: tcs,
            usage: TokenUsage::default(),
            cost_usd: None,
            model: None,
            message_id: util::map_str(&m, "message_id").or_else(|| util::map_str(&m, "id")),
        });
        let idx = turns.len() - 1;
        for (ci, c) in turns[idx].tool_calls.iter().enumerate() {
            if !c.id.is_empty() {
                calls.insert(c.id.clone(), (idx, ci));
            }
        }
    }
    let ledger = util::query_json(
        &c,
        "SELECT * FROM usage_ledger WHERE session_id = ?1 ORDER BY created_timestamp, id",
        &[&id],
    );
    if !ledger.is_empty() {
        for l in ledger {
            let at = util::map_ts(&l, "created_timestamp").unwrap_or(i64::MAX);
            let u = normalize(
                i(&l, "input_tokens"),
                i(&l, "output_tokens"),
                i(&l, "cache_read_tokens"),
                i(&l, "cache_write_tokens"),
            );
            let cost = util::map_f64(&l, "cost");
            let model = util::map_str(&l, "model");
            let idx = turns
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    t.role == Role::Assistant && util::parse_ts_str(&t.ts).unwrap_or(0) <= at + 1000
                })
                .map(|(i, _)| i)
                .next_back();
            let idx = match idx {
                Some(i) => i,
                None => {
                    turns.push(Turn {
                        role: Role::Assistant,
                        ts: if at == i64::MAX {
                            String::new()
                        } else {
                            util::ms_to_rfc3339(at)
                        },
                        text: String::new(),
                        tool_calls: Vec::new(),
                        usage: TokenUsage::default(),
                        cost_usd: None,
                        model: None,
                        message_id: None,
                    });
                    turns.len() - 1
                }
            };
            let t = &mut turns[idx];
            t.usage.add(&u);
            if let Some(c) = cost {
                t.cost_usd = Some(t.cost_usd.unwrap_or(0.0) + c);
            }
            if t.model.is_none() {
                t.model = model;
            }
        }
    } else if let Some(t) = turns.iter_mut().rev().find(|t| t.role == Role::Assistant) {
        t.usage = meta.usage.clone();
        if meta.cost_usd > 0.0 {
            t.cost_usd = Some(meta.cost_usd);
        }
        t.model = meta.models.first().cloned();
    }
    let mut usage = TokenUsage::default();
    let mut cost = 0.0;
    let mut models = meta.models.clone();
    for t in &turns {
        usage.add(&t.usage);
        cost += t.cost_usd.unwrap_or(0.0);
        if let Some(m) = &t.model {
            if !models.contains(m) {
                models.push(m.clone());
            }
        }
    }
    if usage.total() > 0 {
        meta.usage = usage;
    }
    if cost > 0.0 {
        meta.cost_usd = cost;
    }
    meta.models = models;
    meta.turn_count = turns.len() as u32;
    if meta.title.is_none() {
        meta.title = turns
            .iter()
            .find(|t| t.role == Role::User)
            .map(|t| truncate_chars(t.text.lines().next().unwrap_or("").trim(), 80));
    }
    Some(Session { meta, turns })
}

impl SessionSource for GooseSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.candidates.clone()
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out = Vec::new();
        for db in self.dbs() {
            let Some(c) = util::open_ro(&db) else {
                continue;
            };
            let mut counts = std::collections::HashMap::new();
            if let Ok(mut st) =
                c.prepare("SELECT session_id, COUNT(*) FROM messages GROUP BY session_id")
            {
                if let Ok(rows) =
                    st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                {
                    for (k, n) in rows.flatten() {
                        counts.insert(k, n as u32);
                    }
                }
            }
            for r in util::query_json(&c, "SELECT * FROM sessions", &[]) {
                let mut m = meta_of(&db, &r);
                if m.id.is_empty() {
                    continue;
                }
                m.turn_count = counts.get(&m.id).copied().unwrap_or(0);
                out.push(m);
            }
        }
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        self.dbs().into_iter().find_map(|db| load_db(&db, id))
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        Some(util::in_dir(
            meta.project_path.as_deref(),
            format!("goose session --resume --session-id {}", meta.id),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_rows_carry_usage_and_recorded_cost() {
        let dir = std::env::temp_dir().join(format!("omniget-goose-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("sessions.db");
        let _ = std::fs::remove_file(&db);
        {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch(
                r#"CREATE TABLE sessions(id TEXT PRIMARY KEY, name TEXT, description TEXT, working_dir TEXT, created_at TEXT, updated_at TEXT, accumulated_input_tokens INTEGER, accumulated_output_tokens INTEGER, accumulated_cost REAL, model_config_json TEXT, parent_session_id TEXT);
                CREATE TABLE messages(id INTEGER PRIMARY KEY, message_id TEXT, session_id TEXT, role TEXT, content_json TEXT, created_timestamp INTEGER);
                CREATE TABLE usage_ledger(id INTEGER PRIMARY KEY, session_id TEXT, created_timestamp INTEGER, model TEXT, input_tokens INTEGER, output_tokens INTEGER, total_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER, cost REAL, cost_source TEXT, is_compaction INTEGER);
                INSERT INTO sessions VALUES('20260920_1','Deploy','', '/w/p','2026-09-20 10:00:00','2026-09-20 10:05:00',1500,300,0.9,'{"model_name":"gpt-5"}',NULL);
                INSERT INTO messages VALUES(1,'m1','20260920_1','user','[{"type":"text","text":"faça deploy"}]',1789900000);
                INSERT INTO messages VALUES(2,'m2','20260920_1','assistant','[{"type":"toolRequest","id":"r1","toolCall":{"status":"success","value":{"name":"developer__shell","arguments":{"command":"make deploy"}}}}]',1789900002);
                INSERT INTO messages VALUES(3,'m3','20260920_1','user','[{"type":"toolResponse","id":"r1","toolResult":{"status":"success","value":[{"type":"text","text":"done"}]}}]',1789900009);
                INSERT INTO messages VALUES(4,'m4','20260920_1','assistant','[{"type":"text","text":"feito"}]',1789900010);
                INSERT INTO usage_ledger VALUES(1,'20260920_1',1789900002,'gpt-5',1000,100,1100,400,0,0.5,'provider',0);
                INSERT INTO usage_ledger VALUES(2,'20260920_1',1789900010,'gpt-5',500,200,700,0,0,0.4,'provider',0);
                "#,
            )
            .unwrap();
        }
        let src = GooseSource::with_dbs(vec![db.clone()]);
        assert_eq!(src.list().len(), 1);
        let s = src.load("20260920_1").unwrap();
        assert_eq!(
            s.turns.len(),
            3,
            "the tool-response-only user message is folded into the call"
        );
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Bash");
        assert_eq!(s.turns[1].tool_calls[0].result.as_deref(), Some("done"));
        assert_eq!(s.turns[1].usage.cache_read, 400);
        assert!((s.meta.cost_usd - 0.9).abs() < 1e-9);
        assert_eq!(s.meta.project_path.as_deref(), Some("/w/p"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
