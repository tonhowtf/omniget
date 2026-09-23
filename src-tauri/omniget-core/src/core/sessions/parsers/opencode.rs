//! OpenCode e Kilo (fork do OpenCode): SQLite `<data>/opencode.db` /
//! `<data>/kilo.db` (+ canais `opencode-<canal>.db`), com o armazenamento
//! antigo em `<data>/storage/{session,message,part}/**.json`.
//!
//! `<data>` = `$OPENCODE_DATA_DIR` | `$KILO_DATA_DIR`, senão
//! `${XDG_DATA_HOME:-~/.local/share}/{opencode,kilo}`; `OPENCODE_DB` aponta
//! um banco direto.
//!
//! Tabelas: `session(id, parent_id, directory, title, time_created,
//! time_updated, …)`, `message(id, session_id, data JSON)`,
//! `part(id, message_id, session_id?, data JSON)`. `message.data` do
//! assistente traz `modelID`, `cost` (muitas vezes 0: aí vale a tabela de
//! preço), `tokens{input, output, reasoning, cache{read, write}}`. As colunas
//! são lidas por nome (`SELECT *`) para aguentar mudança de esquema.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolCall, ToolStatus, Turn,
};
use crate::core::sessions::util::{self, canonical_tool, truncate_chars, value_text, RESULT_MAX};

pub struct OpenCodeSource {
    tool: &'static str,
    data_dirs: Vec<PathBuf>,
    extra_dbs: Vec<PathBuf>,
}

impl OpenCodeSource {
    pub fn opencode() -> Self {
        let mut dirs = util::env_paths("OPENCODE_DATA_DIR");
        if let Some(d) = util::xdg_data_home() {
            dirs.push(d.join("opencode"));
        }
        Self {
            tool: "opencode",
            data_dirs: dirs,
            extra_dbs: util::env_paths("OPENCODE_DB"),
        }
    }

    pub fn kilo() -> Self {
        let mut dirs = util::env_paths("KILO_DATA_DIR");
        if let Some(d) = util::xdg_data_home() {
            dirs.push(d.join("kilo"));
        }
        Self {
            tool: "kilo",
            data_dirs: dirs,
            extra_dbs: Vec::new(),
        }
    }

    pub fn custom(tool: &'static str, data_dirs: Vec<PathBuf>) -> Self {
        Self {
            tool,
            data_dirs,
            extra_dbs: Vec::new(),
        }
    }

    fn db_name(&self) -> &'static str {
        if self.tool == "kilo" {
            "kilo"
        } else {
            "opencode"
        }
    }

    pub fn dbs(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = self
            .extra_dbs
            .iter()
            .filter(|p| p.is_file())
            .cloned()
            .collect();
        let base = self.db_name();
        for d in &self.data_dirs {
            let Ok(rd) = std::fs::read_dir(d) else {
                continue;
            };
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.ends_with(".db")
                    && (n == format!("{base}.db") || n.starts_with(&format!("{base}-")))
                {
                    let p = e.path();
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
        out
    }

    fn storage_dirs(&self) -> Vec<PathBuf> {
        self.data_dirs
            .iter()
            .map(|d| d.join("storage"))
            .filter(|d| d.join("session").is_dir())
            .collect()
    }

    fn meta_from_row(
        &self,
        db: &Path,
        r: &Map<String, Value>,
        counts: &HashMap<String, u32>,
    ) -> SessionMeta {
        let id = util::map_str(r, "id").unwrap_or_default();
        let created = util::map_ts(r, "time_created");
        let updated = util::map_ts(r, "time_updated").or(created);
        let model = match util::json_col(r, "model") {
            Value::Object(o) => o.get("id").and_then(|x| x.as_str()).map(|s| s.to_string()),
            Value::String(s) if !s.is_empty() => Some(s),
            _ => None,
        };
        let usage = TokenUsage {
            input: util::map_i64(r, "tokens_input").unwrap_or(0).max(0) as u64,
            output: util::map_i64(r, "tokens_output").unwrap_or(0).max(0) as u64,
            cache_read: util::map_i64(r, "tokens_cache_read").unwrap_or(0).max(0) as u64,
            cache_write: util::map_i64(r, "tokens_cache_write").unwrap_or(0).max(0) as u64,
            reasoning: util::map_i64(r, "tokens_reasoning").unwrap_or(0).max(0) as u64,
        };
        SessionMeta {
            tool: self.tool.into(),
            account: None,
            turn_count: counts.get(&id).copied().unwrap_or(0),
            id,
            title: util::map_str(r, "title"),
            project_path: util::map_str(r, "directory"),
            git_branch: None,
            started: created.map(util::ms_to_rfc3339),
            ended: updated.map(util::ms_to_rfc3339),
            models: model.into_iter().collect(),
            parent_session: util::map_str(r, "parent_id").or_else(|| util::map_str(r, "parentID")),
            usage,
            cost_usd: util::map_f64(r, "cost").unwrap_or(0.0),
            source: db.to_path_buf(),
            mtime_ms: updated.unwrap_or_else(|| util::file_mtime_ms(db)),
        }
    }

    fn list_db(&self, db: &Path) -> Vec<SessionMeta> {
        let Some(c) = util::open_ro(db) else {
            return Vec::new();
        };
        let mut counts = HashMap::new();
        if let Ok(mut st) =
            c.prepare("SELECT session_id, COUNT(*) FROM message GROUP BY session_id")
        {
            if let Ok(rows) =
                st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            {
                for (k, n) in rows.flatten() {
                    counts.insert(k, n as u32);
                }
            }
        }
        util::query_json(&c, "SELECT * FROM session", &[])
            .iter()
            .map(|r| self.meta_from_row(db, r, &counts))
            .filter(|m| !m.id.is_empty())
            .collect()
    }

    fn load_db(&self, db: &Path, id: &str) -> Option<Session> {
        let c = util::open_ro(db)?;
        let rows = util::query_json(&c, "SELECT * FROM session WHERE id = ?1", &[&id]);
        let row = rows.first()?;
        let mut meta = self.meta_from_row(db, row, &HashMap::new());
        let order = if util::table_columns(&c, "message")
            .iter()
            .any(|n| n == "time_created")
        {
            "ORDER BY time_created, id"
        } else {
            "ORDER BY id"
        };
        let messages = util::query_json(
            &c,
            &format!("SELECT * FROM message WHERE session_id = ?1 {order}"),
            &[&id],
        );
        let part_cols = util::table_columns(&c, "part");
        let part_order = if part_cols.iter().any(|n| n == "time_created") {
            "ORDER BY time_created, id"
        } else {
            "ORDER BY id"
        };
        let parts = if part_cols.iter().any(|n| n == "session_id") {
            util::query_json(
                &c,
                &format!("SELECT * FROM part WHERE session_id = ?1 {part_order}"),
                &[&id],
            )
        } else {
            util::query_json(
                &c,
                &format!(
                    "SELECT * FROM part WHERE message_id IN (SELECT id FROM message WHERE session_id = ?1) {part_order}"
                ),
                &[&id],
            )
        };
        let mut by_msg: HashMap<String, Vec<Value>> = HashMap::new();
        for p in parts {
            let mid = util::map_str(&p, "message_id").unwrap_or_default();
            let mut data = util::json_col(&p, "data");
            if let Value::Object(o) = &mut data {
                o.entry("id")
                    .or_insert_with(|| p.get("id").cloned().unwrap_or(Value::Null));
            }
            by_msg.entry(mid).or_default().push(data);
        }
        let msgs: Vec<(String, Value)> = messages
            .into_iter()
            .map(|m| {
                let mid = util::map_str(&m, "id").unwrap_or_default();
                let mut data = util::json_col(&m, "data");
                if let Value::Object(o) = &mut data {
                    if !o.contains_key("time") {
                        if let Some(t) = util::map_ts(&m, "time_created") {
                            o.insert("time".into(), serde_json::json!({ "created": t }));
                        }
                    }
                }
                (mid, data)
            })
            .collect();
        let turns = build_turns(&msgs, &by_msg);
        finish_meta(&mut meta, &turns);
        Some(Session { meta, turns })
    }

    // --- armazenamento antigo em JSON -------------------------------------

    fn legacy_sessions(&self) -> Vec<(PathBuf, PathBuf)> {
        let mut out = Vec::new();
        for st in self.storage_dirs() {
            for e in walkdir::WalkDir::new(st.join("session"))
                .max_depth(3)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if e.file_type().is_file()
                    && e.path().extension().map(|x| x == "json").unwrap_or(false)
                {
                    out.push((st.clone(), e.path().to_path_buf()));
                }
            }
        }
        out
    }

    fn legacy_meta(&self, file: &Path) -> Option<SessionMeta> {
        let v: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        let id = util::str_of(&v, "id")?.to_string();
        let t = v.get("time");
        let created = t.and_then(|t| util::ts_ms_of(t.get("created")));
        let updated = t.and_then(|t| util::ts_ms_of(t.get("updated"))).or(created);
        Some(SessionMeta {
            tool: self.tool.into(),
            account: None,
            id,
            title: util::str_of(&v, "title").map(|s| s.to_string()),
            project_path: util::str_of(&v, "directory").map(|s| s.to_string()),
            git_branch: None,
            started: created.map(util::ms_to_rfc3339),
            ended: updated.map(util::ms_to_rfc3339),
            models: Vec::new(),
            parent_session: util::str_of(&v, "parentID").map(|s| s.to_string()),
            turn_count: 0,
            usage: TokenUsage::default(),
            cost_usd: 0.0,
            source: file.to_path_buf(),
            mtime_ms: updated.unwrap_or_else(|| util::file_mtime_ms(file)),
        })
    }

    fn legacy_load(&self, storage: &Path, file: &Path) -> Option<Session> {
        let mut meta = self.legacy_meta(file)?;
        let mut msgs: Vec<(String, Value)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(storage.join("message").join(&meta.id)) {
            for e in rd.flatten() {
                if let Ok(v) =
                    serde_json::from_slice::<Value>(&std::fs::read(e.path()).unwrap_or_default())
                {
                    let id = util::str_of(&v, "id").unwrap_or("").to_string();
                    msgs.push((id, v));
                }
            }
        }
        msgs.sort_by_key(|(id, v)| {
            (
                v.get("time")
                    .and_then(|t| util::ts_ms_of(t.get("created")))
                    .unwrap_or(0),
                id.clone(),
            )
        });
        let mut by_msg: HashMap<String, Vec<Value>> = HashMap::new();
        for (mid, _) in &msgs {
            let mut parts = Vec::new();
            if let Ok(rd) = std::fs::read_dir(storage.join("part").join(mid)) {
                for e in rd.flatten() {
                    if let Ok(v) = serde_json::from_slice::<Value>(
                        &std::fs::read(e.path()).unwrap_or_default(),
                    ) {
                        parts.push(v);
                    }
                }
            }
            parts.sort_by(|a, b| util::str_of(a, "id").cmp(&util::str_of(b, "id")));
            by_msg.insert(mid.clone(), parts);
        }
        let turns = build_turns(&msgs, &by_msg);
        finish_meta(&mut meta, &turns);
        Some(Session { meta, turns })
    }
}

fn tokens_of(t: &Value) -> TokenUsage {
    let cache = t.get("cache");
    TokenUsage {
        input: util::num(t, "input"),
        output: util::num(t, "output"),
        cache_read: cache.map(|c| util::num(c, "read")).unwrap_or(0),
        cache_write: cache.map(|c| util::num(c, "write")).unwrap_or(0),
        reasoning: util::num(t, "reasoning"),
    }
}

fn build_turns(msgs: &[(String, Value)], parts: &HashMap<String, Vec<Value>>) -> Vec<Turn> {
    let mut turns = Vec::new();
    for (mid, m) in msgs {
        let role = match util::str_of(m, "role") {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => Role::System,
        };
        let created = m
            .get("time")
            .and_then(|t| util::ts_ms_of(t.get("created")))
            .unwrap_or(0);
        let mut text = Vec::new();
        let mut calls = Vec::new();
        for p in parts.get(mid).map(|v| v.as_slice()).unwrap_or(&[]) {
            match util::str_of(p, "type") {
                Some("text") => {
                    if p.get("synthetic").and_then(|s| s.as_bool()) == Some(true)
                        && role == Role::User
                    {
                        continue;
                    }
                    if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                        text.push(t.to_string());
                    }
                }
                Some("tool") => {
                    let name = util::str_of(p, "tool").unwrap_or("?").to_string();
                    let state = p.get("state").cloned().unwrap_or(Value::Null);
                    let input = state.get("input").cloned().unwrap_or(Value::Null);
                    let status = match util::str_of(&state, "status") {
                        Some("completed") => ToolStatus::Ok,
                        Some("error") => ToolStatus::Error,
                        _ => ToolStatus::Pending,
                    };
                    let result = state
                        .get("output")
                        .or_else(|| state.get("error"))
                        .map(|o| value_text(o, RESULT_MAX));
                    let time = state.get("time");
                    let ms = match (
                        time.and_then(|t| util::ts_ms_of(t.get("start"))),
                        time.and_then(|t| util::ts_ms_of(t.get("end"))),
                    ) {
                        (Some(a), Some(b)) if b >= a => Some((b - a) as u64),
                        _ => None,
                    };
                    let mut canon = canonical_tool(&name);
                    // MCP no OpenCode é `<servidor>_<tool>`.
                    if canon == "Other" && name.contains('_') {
                        canon = "Mcp";
                    }
                    calls.push(ToolCall {
                        id: util::str_of(p, "callID")
                            .or_else(|| util::str_of(p, "id"))
                            .unwrap_or("")
                            .to_string(),
                        name_canonical: canon.to_string(),
                        subagent: (canon == "Agent").then(|| {
                            util::str_of(&input, "subagent_type")
                                .or_else(|| util::str_of(&input, "agent"))
                                .unwrap_or("general")
                                .to_string()
                        }),
                        name_raw: name,
                        input,
                        result,
                        status,
                        ms,
                    });
                }
                Some("subtask") | Some("agent") => {
                    let agent = util::str_of(p, "agent")
                        .or_else(|| util::str_of(p, "name"))
                        .unwrap_or("agent")
                        .to_string();
                    calls.push(ToolCall {
                        id: util::str_of(p, "id").unwrap_or("").to_string(),
                        name_canonical: "Agent".into(),
                        name_raw: util::str_of(p, "type").unwrap_or("subtask").to_string(),
                        input: p.clone(),
                        result: None,
                        status: ToolStatus::Ok,
                        ms: None,
                        subagent: Some(agent),
                    });
                }
                _ => {}
            }
        }
        let usage = m.get("tokens").map(tokens_of).unwrap_or_default();
        let cost = m.get("cost").and_then(|c| c.as_f64()).filter(|c| *c > 0.0);
        let text = text.join("\n");
        if text.trim().is_empty() && calls.is_empty() && usage.total() == 0 {
            continue;
        }
        turns.push(Turn {
            role,
            ts: if created > 0 {
                util::ms_to_rfc3339(created)
            } else {
                String::new()
            },
            text,
            tool_calls: calls,
            usage,
            cost_usd: cost,
            model: util::str_of(m, "modelID").map(|s| s.to_string()),
            message_id: Some(mid.clone()).filter(|s| !s.is_empty()),
        });
    }
    turns
}

fn finish_meta(meta: &mut SessionMeta, turns: &[Turn]) {
    let mut usage = TokenUsage::default();
    let mut cost = 0.0;
    let mut models = Vec::new();
    let mut first_user = None;
    for t in turns {
        usage.add(&t.usage);
        cost += t.cost_usd.unwrap_or(0.0);
        if let Some(m) = &t.model {
            if !models.contains(m) {
                models.push(m.clone());
            }
        }
        if first_user.is_none() && t.role == Role::User && !t.text.is_empty() {
            first_user = Some(t.text.clone());
        }
    }
    if usage.total() > 0 {
        meta.usage = usage;
    }
    if cost > 0.0 {
        meta.cost_usd = cost;
    }
    if !models.is_empty() {
        meta.models = models;
    }
    if meta.title.is_none() {
        meta.title = first_user.map(|t| truncate_chars(t.lines().next().unwrap_or(&t).trim(), 80));
    }
    meta.turn_count = turns.len() as u32;
    if let Some(first) = turns.iter().filter_map(|t| util::parse_ts_str(&t.ts)).min() {
        meta.started = Some(util::ms_to_rfc3339(first));
    }
    if let Some(last) = turns.iter().filter_map(|t| util::parse_ts_str(&t.ts)).max() {
        meta.ended = Some(util::ms_to_rfc3339(last));
    }
}

impl SessionSource for OpenCodeSource {
    fn tool(&self) -> &'static str {
        self.tool
    }

    fn roots(&self) -> Vec<PathBuf> {
        let mut v = self.data_dirs.clone();
        v.extend(self.extra_dbs.iter().cloned());
        v
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for db in self.dbs() {
            for m in self.list_db(&db) {
                if seen.insert(m.id.clone()) {
                    out.push(m);
                }
            }
        }
        for (_, f) in self.legacy_sessions() {
            if let Some(m) = self.legacy_meta(&f) {
                if seen.insert(m.id.clone()) {
                    out.push(m);
                }
            }
        }
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        for db in self.dbs() {
            if let Some(s) = self.load_db(&db, id) {
                return Some(s);
            }
        }
        for (st, f) in self.legacy_sessions() {
            if f.file_stem()
                .map(|s| s.to_string_lossy() == id)
                .unwrap_or(false)
            {
                return self.legacy_load(&st, &f);
            }
        }
        None
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        if meta.parent_session.is_some() {
            return None;
        }
        Some(util::in_dir(
            meta.project_path.as_deref(),
            format!("{} --session {}", self.db_name(), meta.id),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_store_with_parts_and_recorded_cost() {
        let dir = std::env::temp_dir().join(format!("omniget-opencode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("opencode.db");
        let _ = std::fs::remove_file(&db);
        {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch(
                r#"CREATE TABLE session(id TEXT PRIMARY KEY, project_id TEXT, parent_id TEXT, directory TEXT, title TEXT, time_created INTEGER, time_updated INTEGER, cost REAL);
                CREATE TABLE message(id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT);
                CREATE TABLE part(id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
                INSERT INTO session VALUES('ses_1','p','', '/w/p','Arrumar testes',1789900000000,1789900060000,0.02);
                INSERT INTO message VALUES('msg_1','ses_1',1789900000000,'{"role":"user","time":{"created":1789900000000}}');
                INSERT INTO message VALUES('msg_2','ses_1',1789900005000,'{"role":"assistant","modelID":"claude-sonnet-4-5","providerID":"anthropic","cost":0.02,"tokens":{"input":100,"output":40,"reasoning":0,"cache":{"read":900,"write":50}},"time":{"created":1789900005000}}');
                INSERT INTO part VALUES('prt_1','msg_1','ses_1',1789900000000,'{"type":"text","text":"rode os testes"}');
                INSERT INTO part VALUES('prt_2','msg_2','ses_1',1789900005000,'{"type":"tool","tool":"bash","callID":"call_1","state":{"status":"completed","input":{"command":"cargo test"},"output":"ok","time":{"start":1789900005000,"end":1789900007500}}}');
                INSERT INTO part VALUES('prt_3','msg_2','ses_1',1789900008000,'{"type":"text","text":"passou"}');
                "#,
            )
            .unwrap();
        }
        let src = OpenCodeSource::custom("opencode", vec![dir.clone()]);
        let metas = src.list();
        assert_eq!(metas.len(), 1);
        assert!(metas[0].parent_session.is_none());
        let s = src.load("ses_1").unwrap();
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Bash");
        assert_eq!(s.turns[1].tool_calls[0].ms, Some(2500));
        assert_eq!(s.turns[1].cost_usd, Some(0.02));
        assert_eq!(s.meta.usage.cache_read, 900);
        assert_eq!(s.meta.project_path.as_deref(), Some("/w/p"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
