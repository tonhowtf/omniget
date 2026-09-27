//! Crush (charmbracelet): um SQLite por projeto em `<projeto>/.crush/crush.db`.
//!
//! Descoberta dos projetos: o registro `projects.json` do Crush
//! (`${XDG_DATA_HOME:-~/.local/share}/crush/projects.json`, no Windows
//! `%LOCALAPPDATA%\crush`), os `project_path` que o índice já conhece das
//! outras ferramentas e os irmãos desses diretórios (só um `stat` cada).
//!
//! Tabelas: `sessions(id, parent_session_id, title, message_count,
//! prompt_tokens, completion_tokens, cost, created_at, updated_at)` e
//! `messages(id, session_id, role, parts JSON, model, provider, created_at)`.
//! Tokens e custo só existem por sessão: são repartidos igualmente entre as
//! respostas do assistente para as visões por dia (o total não muda).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolCall, ToolStatus, Turn,
};
use crate::core::sessions::util::{self, canonical_tool, truncate_chars, value_text, RESULT_MAX};

pub const TOOL: &str = "crush";

pub struct CrushSource {
    extra_projects: Vec<PathBuf>,
    registry: Vec<PathBuf>,
}

impl Default for CrushSource {
    fn default() -> Self {
        Self::new()
    }
}

impl CrushSource {
    pub fn new() -> Self {
        let mut registry = Vec::new();
        if let Some(d) = util::xdg_data_home() {
            registry.push(d.join("crush").join("projects.json"));
        }
        if let Some(d) = dirs::data_local_dir() {
            let p = d.join("crush").join("projects.json");
            if !registry.contains(&p) {
                registry.push(p);
            }
        }
        Self {
            extra_projects: crate::core::sessions::index::known_projects(),
            registry,
        }
    }

    pub fn with_projects(projects: Vec<PathBuf>) -> Self {
        Self {
            extra_projects: projects,
            registry: Vec::new(),
        }
    }

    /// (banco, diretório do projeto)
    pub fn dbs(&self) -> Vec<(PathBuf, PathBuf)> {
        let mut out: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut push = |db: PathBuf, proj: PathBuf| {
            if db.is_file() && !out.iter().any(|(d, _)| *d == db) {
                out.push((db, proj));
            }
        };
        for reg in &self.registry {
            let Ok(text) = std::fs::read_to_string(reg) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let mut found = Vec::new();
            collect_projects(&v, &mut found);
            for (path, data_dir) in found {
                let proj = PathBuf::from(&path);
                let data = match data_dir {
                    Some(d) if Path::new(&d).is_absolute() => PathBuf::from(d),
                    Some(d) => proj.join(d),
                    None => proj.join(".crush"),
                };
                push(data.join("crush.db"), proj);
            }
        }
        let mut parents: Vec<PathBuf> = Vec::new();
        for p in &self.extra_projects {
            push(p.join(".crush").join("crush.db"), p.clone());
            if let Some(par) = p.parent() {
                if !parents.iter().any(|x| x == par) && par.components().count() > 2 {
                    parents.push(par.to_path_buf());
                }
            }
        }
        for par in parents {
            let Ok(rd) = std::fs::read_dir(&par) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    push(p.join(".crush").join("crush.db"), p);
                }
            }
        }
        if let Some(h) = util::home() {
            push(h.join(".crush").join("crush.db"), h);
        }
        out
    }
}

fn collect_projects(v: &Value, out: &mut Vec<(String, Option<String>)>) {
    match v {
        Value::Object(o) => {
            if let Some(p) = o.get("path").and_then(|p| p.as_str()) {
                let dd = o
                    .get("data_dir")
                    .or_else(|| o.get("dataDir"))
                    .and_then(|d| d.as_str())
                    .map(|s| s.to_string());
                out.push((p.to_string(), dd));
            }
            for (_, x) in o {
                collect_projects(x, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                collect_projects(x, out);
            }
        }
        _ => {}
    }
}

fn meta_of(db: &Path, proj: &Path, r: &Map<String, Value>) -> SessionMeta {
    let created = util::map_ts(r, "created_at");
    let updated = util::map_ts(r, "updated_at").or(created);
    SessionMeta {
        tool: TOOL.into(),
        account: None,
        id: util::map_str(r, "id").unwrap_or_default(),
        title: util::map_str(r, "title"),
        project_path: Some(proj.to_string_lossy().to_string()),
        git_branch: None,
        started: created.map(util::ms_to_rfc3339),
        ended: updated.map(util::ms_to_rfc3339),
        models: Vec::new(),
        parent_session: util::map_str(r, "parent_session_id"),
        turn_count: util::map_i64(r, "message_count").unwrap_or(0).max(0) as u32,
        usage: TokenUsage {
            input: util::map_i64(r, "prompt_tokens").unwrap_or(0).max(0) as u64,
            output: util::map_i64(r, "completion_tokens").unwrap_or(0).max(0) as u64,
            ..Default::default()
        },
        cost_usd: util::map_f64(r, "cost").unwrap_or(0.0),
        source: db.to_path_buf(),
        mtime_ms: updated.unwrap_or_else(|| util::file_mtime_ms(db)),
    }
}

/// Uma parte do `parts`: embrulhada (`{type, data}`) ou plana.
fn part_parts(p: &Value) -> (String, &Value) {
    let t = util::str_of(p, "type").unwrap_or("").to_string();
    (t, p.get("data").filter(|d| d.is_object()).unwrap_or(p))
}

fn load_db(db: &Path, proj: &Path, id: &str) -> Option<Session> {
    let c = util::open_ro(db)?;
    let rows = util::query_json(&c, "SELECT * FROM sessions WHERE id = ?1", &[&id]);
    let mut meta = meta_of(db, proj, rows.first()?);
    let msgs = util::query_json(
        &c,
        "SELECT * FROM messages WHERE session_id = ?1 ORDER BY created_at, rowid",
        &[&id],
    );
    let mut turns: Vec<Turn> = Vec::new();
    let mut calls: HashMap<String, (usize, usize)> = HashMap::new();
    for m in msgs {
        let role = match util::map_str(&m, "role").as_deref() {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            Some("tool") => Role::Tool,
            _ => Role::System,
        };
        let ts_ms = util::map_ts(&m, "created_at");
        let ts = ts_ms.map(util::ms_to_rfc3339).unwrap_or_default();
        let parts = match util::json_col(&m, "parts") {
            Value::Array(a) => a,
            _ => Vec::new(),
        };
        let mut text = Vec::new();
        let mut tcs = Vec::new();
        for p in &parts {
            let (kind, d) = part_parts(p);
            match kind.as_str() {
                "text" => {
                    if let Some(t) = d.get("text").and_then(|t| t.as_str()) {
                        text.push(t.to_string());
                    }
                }
                "tool_call" => {
                    let name = util::str_of(d, "name").unwrap_or("?").to_string();
                    let input = match d.get("input") {
                        Some(Value::String(s)) => {
                            serde_json::from_str(s).unwrap_or(Value::String(s.clone()))
                        }
                        Some(v) => v.clone(),
                        None => Value::Null,
                    };
                    let canon = canonical_tool(&name);
                    tcs.push(ToolCall {
                        id: util::str_of(d, "id").unwrap_or("").to_string(),
                        name_canonical: canon.to_string(),
                        subagent: (canon == "Agent").then(|| "agent".to_string()),
                        name_raw: name,
                        input,
                        result: None,
                        status: ToolStatus::Pending,
                        ms: None,
                    });
                }
                "tool_result" => {
                    let cid = util::str_of(d, "tool_call_id").unwrap_or("").to_string();
                    if let Some(&(ti, ci)) = calls.get(&cid) {
                        let start = util::parse_ts_str(&turns[ti].ts);
                        let call = &mut turns[ti].tool_calls[ci];
                        call.result = d.get("content").map(|c| value_text(c, RESULT_MAX));
                        call.status = if d.get("is_error").and_then(|e| e.as_bool()) == Some(true) {
                            ToolStatus::Error
                        } else {
                            ToolStatus::Ok
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
        if role == Role::Tool {
            continue;
        }
        let text = text.join("\n");
        if text.trim().is_empty() && tcs.is_empty() {
            continue;
        }
        turns.push(Turn {
            role,
            ts,
            text,
            tool_calls: tcs,
            usage: TokenUsage::default(),
            cost_usd: None,
            model: util::map_str(&m, "model"),
            message_id: util::map_str(&m, "id"),
        });
        let idx = turns.len() - 1;
        for (ci, c) in turns[idx].tool_calls.iter().enumerate() {
            if !c.id.is_empty() {
                calls.insert(c.id.clone(), (idx, ci));
            }
        }
    }
    // Reparte o uso/custo da sessão entre as respostas.
    let idxs: Vec<usize> = turns
        .iter()
        .enumerate()
        .filter(|(_, t)| t.role == Role::Assistant)
        .map(|(i, _)| i)
        .collect();
    if !idxs.is_empty() {
        let n = idxs.len() as u64;
        let (inp, out) = (meta.usage.input, meta.usage.output);
        for (k, &i) in idxs.iter().enumerate() {
            let last = k + 1 == idxs.len();
            let share = |total: u64| {
                if last {
                    total - (total / n) * (n - 1)
                } else {
                    total / n
                }
            };
            turns[i].usage.input = share(inp);
            turns[i].usage.output = share(out);
            if meta.cost_usd > 0.0 {
                turns[i].cost_usd = Some(meta.cost_usd / n as f64);
            }
        }
    }
    let mut models = Vec::new();
    let mut first_user = None;
    for t in &turns {
        if let Some(m) = &t.model {
            if !models.contains(m) {
                models.push(m.clone());
            }
        }
        if first_user.is_none() && t.role == Role::User {
            first_user = Some(t.text.clone());
        }
    }
    meta.models = models;
    if meta.title.is_none() {
        meta.title = first_user.map(|t| truncate_chars(t.lines().next().unwrap_or(&t).trim(), 80));
    }
    meta.turn_count = turns.len() as u32;
    Some(Session { meta, turns })
}

impl SessionSource for CrushSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.dbs().into_iter().map(|(d, _)| d).collect()
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out = Vec::new();
        for (db, proj) in self.dbs() {
            let Some(c) = util::open_ro(&db) else {
                continue;
            };
            for r in util::query_json(&c, "SELECT * FROM sessions", &[]) {
                let m = meta_of(&db, &proj, &r);
                if !m.id.is_empty() {
                    out.push(m);
                }
            }
        }
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        self.dbs()
            .into_iter()
            .find_map(|(db, proj)| load_db(&db, &proj, id))
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        // O TUI do Crush lista as sessões do projeto; `run --session`
        // continua uma sessão sem TUI.
        Some(util::in_dir(
            meta.project_path.as_deref(),
            "crush".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_project_db_with_wrapped_parts() {
        let proj = std::env::temp_dir().join(format!("omniget-crush-{}", std::process::id()));
        let data = proj.join(".crush");
        std::fs::create_dir_all(&data).unwrap();
        let db = data.join("crush.db");
        let _ = std::fs::remove_file(&db);
        {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch(
                r#"CREATE TABLE sessions(id TEXT PRIMARY KEY, parent_session_id TEXT, title TEXT, message_count INTEGER, prompt_tokens INTEGER, completion_tokens INTEGER, cost REAL, updated_at INTEGER, created_at INTEGER);
                CREATE TABLE messages(id TEXT PRIMARY KEY, session_id TEXT, role TEXT, parts TEXT, model TEXT, provider TEXT, created_at INTEGER, updated_at INTEGER);
                INSERT INTO sessions VALUES('s1',NULL,'Refatorar',3,1000,200,0.05,1789900060,1789900000);
                INSERT INTO messages VALUES('m1','s1','user','[{"type":"text","data":{"text":"refatore"}}]',NULL,NULL,1789900000,1789900000);
                INSERT INTO messages VALUES('m2','s1','assistant','[{"type":"tool_call","data":{"id":"tc1","name":"view","input":"{\"file_path\":\"a.go\"}","finished":true}},{"type":"finish","data":{"reason":"tool_use"}}]','claude-sonnet-4-5','anthropic',1789900002,1789900002);
                INSERT INTO messages VALUES('m3','s1','tool','[{"type":"tool_result","data":{"tool_call_id":"tc1","name":"view","content":"package main","is_error":false}}]',NULL,NULL,1789900004,1789900004);
                "#,
            )
            .unwrap();
        }
        let src = CrushSource::with_projects(vec![proj.clone()]);
        let metas = src.list();
        assert_eq!(metas.len(), 1);
        let s = src.load("s1").unwrap();
        assert_eq!(s.turns.len(), 2);
        let c = &s.turns[1].tool_calls[0];
        assert_eq!(c.name_canonical, "Read");
        assert_eq!(c.result.as_deref(), Some("package main"));
        assert_eq!(c.ms, Some(2000));
        assert_eq!(s.turns[1].usage.input, 1000);
        assert_eq!(s.turns[1].cost_usd, Some(0.05));
        std::fs::remove_dir_all(&proj).ok();
    }
}
