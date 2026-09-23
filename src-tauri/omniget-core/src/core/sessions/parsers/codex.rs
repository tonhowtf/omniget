//! Codex CLI / Codex Desktop: `$CODEX_HOME/{sessions,archived_sessions}/**/
//! rollout-*.jsonl[.zst]`. Envelope `{timestamp, type, payload}`.
//!
//! Tokens: o `event_msg` `token_count` traz o acumulado da thread
//! (`total_token_usage`). O total da sessão é o último acumulado; o uso de
//! cada resposta é a diferença entre dois acumulados seguidos (eventos
//! repetidos com o mesmo total viram zero). Numa thread bifurcada o primeiro
//! evento usa `last_token_usage`, porque o acumulado pode trazer o da mãe.
//! OpenAI conta o cache dentro de `input_tokens` e o raciocínio dentro de
//! `output_tokens`: aqui os dois saem separados.
//!
//! Títulos vêm do `session_index.jsonl` (o nome que o app deu à thread).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolCall, ToolStatus, Turn,
};
use crate::core::sessions::util::{self, canonical_tool, truncate_chars, value_text, RESULT_MAX};

pub const TOOL: &str = "codex";

#[derive(Debug, Clone)]
pub struct Root {
    pub account: String,
    pub home: PathBuf,
    pub is_default: bool,
}

pub struct CodexSource {
    roots: Vec<Root>,
    cache: Mutex<HashMap<PathBuf, (i64, u64, SessionMeta)>>,
    ids: Mutex<HashMap<String, (PathBuf, String)>>,
}

impl Default for CodexSource {
    fn default() -> Self {
        Self::new()
    }
}

pub fn default_roots() -> Vec<Root> {
    let mut roots = Vec::new();
    let mut seen = Vec::new();
    let home = match std::env::var_os("CODEX_HOME") {
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => util::home().map(|h| h.join(".codex")),
    };
    if let Some(h) = home {
        seen.push(util::canon(&h));
        roots.push(Root {
            account: "default".into(),
            home: h,
            is_default: true,
        });
    }
    for (id, dir) in super::account_dirs("codex") {
        let c = util::canon(&dir);
        if seen.contains(&c) {
            continue;
        }
        seen.push(c);
        roots.push(Root {
            account: id,
            home: dir,
            is_default: false,
        });
    }
    roots
}

impl CodexSource {
    pub fn new() -> Self {
        Self::with_roots(default_roots())
    }

    pub fn with_roots(roots: Vec<Root>) -> Self {
        Self {
            roots,
            cache: Mutex::new(HashMap::new()),
            ids: Mutex::new(HashMap::new()),
        }
    }

    pub fn root_list(&self) -> &[Root] {
        &self.roots
    }

    pub fn files(&self) -> Vec<(PathBuf, String)> {
        let mut out = Vec::new();
        for r in &self.roots {
            for sub in ["sessions", "archived_sessions"] {
                let dir = r.home.join(sub);
                if !dir.is_dir() {
                    continue;
                }
                for e in walkdir::WalkDir::new(&dir)
                    .max_depth(6)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    let name = e.file_name().to_string_lossy();
                    if e.file_type().is_file()
                        && name.starts_with("rollout-")
                        && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"))
                    {
                        out.push((e.path().to_path_buf(), r.account.clone()));
                    }
                }
            }
        }
        out
    }

    fn titles(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        for r in &self.roots {
            let p = r.home.join("session_index.jsonl");
            let _ = util::for_each_line(&p, 0, |l| {
                if let Ok(v) = serde_json::from_slice::<Value>(l) {
                    if let (Some(id), Some(name)) =
                        (util::str_of(&v, "id"), util::str_of(&v, "thread_name"))
                    {
                        m.insert(id.to_string(), name.to_string());
                    }
                }
            });
        }
        m
    }

    pub fn home_of(&self, account: &str) -> Option<&Root> {
        self.roots.iter().find(|r| r.account == account)
    }

    fn path_of(&self, id: &str) -> Option<(PathBuf, String)> {
        if let Some(hit) = self.ids.lock().ok().and_then(|m| m.get(id).cloned()) {
            if hit.0.exists() {
                return Some(hit);
            }
        }
        // O nome do arquivo termina no id da thread.
        for (p, acc) in self.files() {
            if file_id(&p).as_deref() == Some(id) {
                return Some((p, acc));
            }
        }
        self.list();
        self.ids.lock().ok().and_then(|m| m.get(id).cloned())
    }
}

/// `rollout-2026-09-20T20-12-46-<uuid>.jsonl[.zst]` → `<uuid>`.
pub fn file_id(p: &Path) -> Option<String> {
    let name = p.file_name()?.to_string_lossy().to_string();
    let stem = name.trim_end_matches(".zst").trim_end_matches(".jsonl");
    if stem.len() < 36 {
        return None;
    }
    let tail = &stem[stem.len() - 36..];
    if tail.chars().filter(|c| *c == '-').count() == 4 {
        Some(tail.to_string())
    } else {
        None
    }
}

#[derive(Default, Clone, Copy)]
struct Raw {
    input: u64,
    cached: u64,
    cache_write: u64,
    output: u64,
    reasoning: u64,
}

impl Raw {
    fn of(v: &Value) -> Raw {
        Raw {
            input: util::num(v, "input_tokens"),
            cached: util::num(v, "cached_input_tokens"),
            cache_write: util::num(v, "cache_write_input_tokens"),
            output: util::num(v, "output_tokens"),
            reasoning: util::num(v, "reasoning_output_tokens"),
        }
    }
    fn sum(&self) -> u64 {
        self.input + self.output
    }
    fn minus(&self, o: &Raw) -> Option<Raw> {
        if self.input < o.input || self.output < o.output {
            return None;
        }
        Some(Raw {
            input: self.input - o.input,
            cached: self.cached.saturating_sub(o.cached),
            cache_write: self.cache_write.saturating_sub(o.cache_write),
            output: self.output - o.output,
            reasoning: self.reasoning.saturating_sub(o.reasoning),
        })
    }
    fn normalized(&self) -> TokenUsage {
        let cached = self.cached.min(self.input);
        let write = self.cache_write.min(self.input - cached);
        let reasoning = self.reasoning.min(self.output);
        TokenUsage {
            input: self.input - cached - write,
            output: self.output - reasoning,
            cache_read: cached,
            cache_write: write,
            reasoning,
        }
    }
}

/// Texto injetado pelo app (contexto de ambiente, AGENTS.md, plugins…) e
/// não digitado pela pessoa.
fn injected(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with('<') && t.ends_with('>'))
        || t.starts_with("# AGENTS.md instructions")
        || t.starts_with("<environment_context>")
        || t.starts_with("<user_instructions>")
}

#[derive(Default)]
pub struct CodexParser {
    pub turns: Vec<Turn>,
    calls: HashMap<String, (usize, usize)>,
    pub id: Option<String>,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub parent: Option<String>,
    pub agent_path: Option<String>,
    pub first_user: Option<String>,
    forked: bool,
    model: Option<String>,
    prev_total: Option<Raw>,
    pub total: TokenUsage,
    pub first_ms: Option<i64>,
    pub last_ms: Option<i64>,
}

fn texts_of(content: Option<&Value>, kinds: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(Value::Array(parts)) = content {
        for p in parts {
            let k = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if kinds.contains(&k) {
                if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                    out.push(t.to_string());
                }
            }
        }
    } else if let Some(Value::String(s)) = content {
        out.push(s.clone());
    }
    out
}

impl CodexParser {
    pub fn new() -> Self {
        Self::default()
    }

    fn current_assistant(&mut self, ts: &str) -> usize {
        if let Some(last) = self.turns.last() {
            if last.role == Role::Assistant {
                return self.turns.len() - 1;
            }
        }
        self.turns.push(Turn {
            role: Role::Assistant,
            ts: ts.to_string(),
            text: String::new(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            cost_usd: None,
            model: self.model.clone(),
            message_id: None,
        });
        self.turns.len() - 1
    }

    fn push_call(&mut self, ts: &str, call_id: String, name: String, input: Value) {
        let idx = self.current_assistant(ts);
        let subagent = if canonical_tool(&name) == "Agent" {
            util::str_of(&input, "agent_type")
                .or_else(|| util::str_of(&input, "task_name"))
                .or_else(|| util::str_of(&input, "name"))
                .map(|s| s.to_string())
                .or_else(|| Some("agent".into()))
        } else {
            None
        };
        let turn = &mut self.turns[idx];
        turn.tool_calls.push(ToolCall {
            id: call_id.clone(),
            name_canonical: canonical_tool(&name).to_string(),
            name_raw: name,
            input,
            result: None,
            status: ToolStatus::Pending,
            ms: None,
            subagent,
        });
        if !call_id.is_empty() {
            self.calls.insert(call_id, (idx, turn.tool_calls.len() - 1));
        }
    }

    pub fn feed(&mut self, raw: &[u8]) {
        let Ok(v) = serde_json::from_slice::<Value>(raw) else {
            return;
        };
        let ts = util::str_of(&v, "timestamp").unwrap_or("").to_string();
        let ts_ms = util::parse_ts_str(&ts);
        if let Some(ms) = ts_ms {
            self.first_ms = Some(self.first_ms.map_or(ms, |f| f.min(ms)));
            self.last_ms = Some(self.last_ms.map_or(ms, |l| l.max(ms)));
        }
        let kind = util::str_of(&v, "type").unwrap_or("");
        let Some(p) = v.get("payload") else {
            return;
        };
        match kind {
            "session_meta" => {
                if self.id.is_some() {
                    return;
                }
                self.id = util::str_of(p, "id").map(|s| s.to_string());
                self.cwd = util::str_of(p, "cwd").map(|s| s.to_string());
                if let Some(b) = p.get("git").and_then(|g| util::str_of(g, "branch")) {
                    self.branch = Some(b.to_string());
                }
                self.forked = util::str_of(p, "forked_from_id").is_some();
                let spawn = p
                    .get("source")
                    .and_then(|s| s.get("subagent"))
                    .and_then(|s| s.get("thread_spawn"));
                if let Some(sp) = spawn {
                    self.parent = util::str_of(sp, "parent_thread_id").map(|s| s.to_string());
                    self.agent_path = util::str_of(sp, "agent_path")
                        .or_else(|| util::str_of(sp, "agent_nickname"))
                        .map(|s| s.to_string());
                }
                if self.parent.is_none() {
                    self.parent = util::str_of(p, "parent_thread_id").map(|s| s.to_string());
                }
            }
            "turn_context" => {
                if let Some(m) = util::str_of(p, "model") {
                    self.model = Some(m.to_string());
                }
                if self.cwd.is_none() {
                    self.cwd = util::str_of(p, "cwd").map(|s| s.to_string());
                }
            }
            "response_item" => self.response_item(p, &ts, ts_ms),
            "event_msg" => match util::str_of(p, "type") {
                Some("token_count") => self.token_count(p, &ts),
                Some("thread_settings_applied") => {
                    if let Some(m) = p
                        .get("thread_settings")
                        .and_then(|s| util::str_of(s, "model"))
                    {
                        self.model = Some(m.to_string());
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn response_item(&mut self, p: &Value, ts: &str, ts_ms: Option<i64>) {
        match util::str_of(p, "type").unwrap_or("") {
            "message" => {
                let role = util::str_of(p, "role").unwrap_or("");
                match role {
                    "user" => {
                        let texts: Vec<String> =
                            texts_of(p.get("content"), &["input_text", "text"])
                                .into_iter()
                                .filter(|t| !injected(t))
                                .collect();
                        let text = texts.join("\n");
                        if text.trim().is_empty() {
                            return;
                        }
                        if self.first_user.is_none() {
                            self.first_user = Some(text.clone());
                        }
                        self.turns.push(Turn {
                            role: Role::User,
                            ts: ts.to_string(),
                            text,
                            tool_calls: Vec::new(),
                            usage: TokenUsage::default(),
                            cost_usd: None,
                            model: None,
                            message_id: util::str_of(p, "id").map(|s| s.to_string()),
                        });
                    }
                    "assistant" => {
                        let text = texts_of(p.get("content"), &["output_text", "text"]).join("\n");
                        if text.is_empty() {
                            return;
                        }
                        // Um texto novo depois de tools abre outra `Turn`
                        // só se a atual já tem texto.
                        let reuse = matches!(self.turns.last(), Some(t) if t.role == Role::Assistant && t.text.is_empty());
                        if reuse {
                            let t = self.turns.last_mut().unwrap();
                            t.text = text;
                            if t.message_id.is_none() {
                                t.message_id = util::str_of(p, "id").map(|s| s.to_string());
                            }
                        } else {
                            self.turns.push(Turn {
                                role: Role::Assistant,
                                ts: ts.to_string(),
                                text,
                                tool_calls: Vec::new(),
                                usage: TokenUsage::default(),
                                cost_usd: None,
                                model: self.model.clone(),
                                message_id: util::str_of(p, "id").map(|s| s.to_string()),
                            });
                        }
                    }
                    _ => {}
                }
            }
            "function_call" => {
                let mut name = util::str_of(p, "name").unwrap_or("?").to_string();
                if let Some(ns) = util::str_of(p, "namespace") {
                    if ns.starts_with("mcp__") {
                        name = format!("{ns}__{name}");
                    }
                }
                let args = match p.get("arguments") {
                    Some(Value::String(s)) => {
                        serde_json::from_str(s).unwrap_or(Value::String(s.clone()))
                    }
                    Some(v) => v.clone(),
                    None => Value::Null,
                };
                let id = util::str_of(p, "call_id").unwrap_or("").to_string();
                self.push_call(ts, id, name, args);
            }
            "custom_tool_call" => {
                let name = util::str_of(p, "name").unwrap_or("?").to_string();
                let input = match p.get("input") {
                    Some(Value::String(s)) => Value::String(s.clone()),
                    Some(v) => v.clone(),
                    None => Value::Null,
                };
                let id = util::str_of(p, "call_id").unwrap_or("").to_string();
                self.push_call(ts, id, name, input);
            }
            "local_shell_call" => {
                let id = util::str_of(p, "call_id").unwrap_or("").to_string();
                let input = p.get("action").cloned().unwrap_or(Value::Null);
                self.push_call(ts, id, "local_shell".into(), input);
            }
            "web_search_call" => {
                let input = p.get("action").cloned().unwrap_or(Value::Null);
                let id = util::str_of(p, "id").unwrap_or("").to_string();
                self.push_call(ts, id, "web_search".into(), input);
                if let Some(last) = self.turns.last_mut() {
                    if let Some(c) = last.tool_calls.last_mut() {
                        c.status = ToolStatus::Ok;
                    }
                }
            }
            "function_call_output" | "custom_tool_call_output" => {
                let id = util::str_of(p, "call_id").unwrap_or("").to_string();
                let out = p.get("output").cloned().unwrap_or(Value::Null);
                let (text, failed) = match &out {
                    Value::Object(o) => (
                        o.get("content")
                            .map(|c| value_text(c, RESULT_MAX))
                            .unwrap_or_default(),
                        o.get("success").and_then(|s| s.as_bool()) == Some(false),
                    ),
                    other => (value_text(other, RESULT_MAX), false),
                };
                if let Some(&(ti, ci)) = self.calls.get(&id) {
                    let start = util::parse_ts_str(&self.turns[ti].ts);
                    let call = &mut self.turns[ti].tool_calls[ci];
                    call.result = Some(text);
                    call.status = if failed {
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
            "agent_message" => {
                let from = util::str_of(p, "author").unwrap_or("?");
                let to = util::str_of(p, "recipient").unwrap_or("?");
                let text =
                    texts_of(p.get("content"), &["input_text", "output_text", "text"]).join("\n");
                self.turns.push(Turn {
                    role: Role::System,
                    ts: ts.to_string(),
                    text: format!(
                        "<agent-message from=\"{from}\" to=\"{to}\">{}</agent-message>",
                        truncate_chars(text.trim(), 2000)
                    ),
                    tool_calls: Vec::new(),
                    usage: TokenUsage::default(),
                    cost_usd: None,
                    model: None,
                    message_id: util::str_of(p, "id").map(|s| s.to_string()),
                });
            }
            _ => {}
        }
    }

    fn token_count(&mut self, p: &Value, ts: &str) {
        let Some(info) = p.get("info").filter(|i| !i.is_null()) else {
            return;
        };
        let Some(total_v) = info.get("total_token_usage") else {
            return;
        };
        let total = Raw::of(total_v);
        let last = info.get("last_token_usage").map(Raw::of);
        let delta = match self.prev_total {
            Some(prev) => {
                if total.sum() == prev.sum() {
                    None
                } else {
                    total.minus(&prev).or(last)
                }
            }
            None => {
                if self.forked {
                    last.or(Some(total))
                } else {
                    Some(total)
                }
            }
        };
        self.prev_total = Some(total);
        let Some(d) = delta else {
            return;
        };
        if d.sum() == 0 {
            return;
        }
        let u = d.normalized();
        self.total.add(&u);
        let idx = self.current_assistant(ts);
        let model = self.model.clone();
        let t = &mut self.turns[idx];
        t.usage.add(&u);
        if t.model.is_none() {
            t.model = model;
        }
    }

    pub fn into_session(self, mut meta: SessionMeta, titles: &HashMap<String, String>) -> Session {
        // O id é o do nome do arquivo: um rollout retomado pode começar com o
        // `session_meta` da thread de origem.
        if file_id(&meta.source).is_none() {
            if let Some(id) = &self.id {
                meta.id = id.clone();
            }
        }
        meta.title = titles
            .get(&meta.id)
            .cloned()
            .or_else(|| self.agent_path.clone())
            .or_else(|| {
                self.first_user
                    .as_ref()
                    .map(|t| truncate_chars(t.lines().next().unwrap_or(t).trim(), 80))
            });
        meta.project_path = self.cwd.clone();
        meta.git_branch = self.branch.clone();
        meta.parent_session = self.parent.clone();
        meta.started = self.first_ms.map(util::ms_to_rfc3339);
        meta.ended = self.last_ms.map(util::ms_to_rfc3339);
        let mut models = Vec::new();
        for t in &self.turns {
            if let Some(m) = &t.model {
                if !models.contains(m) {
                    models.push(m.clone());
                }
            }
        }
        meta.models = models;
        meta.usage = self.total.clone();
        meta.turn_count = self.turns.len() as u32;
        Session {
            meta,
            turns: self.turns,
        }
    }
}

fn base_meta(path: &Path, account: &str) -> SessionMeta {
    SessionMeta {
        tool: TOOL.into(),
        account: Some(account.to_string()),
        id: file_id(path).unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        }),
        title: None,
        project_path: None,
        git_branch: None,
        started: None,
        ended: None,
        models: Vec::new(),
        parent_session: None,
        turn_count: 0,
        usage: TokenUsage::default(),
        cost_usd: 0.0,
        source: path.to_path_buf(),
        mtime_ms: util::file_mtime_ms(path),
    }
}

pub fn load_file(path: &Path, account: &str, titles: &HashMap<String, String>) -> Session {
    let mut p = CodexParser::new();
    let _ = util::for_each_line(path, 0, |l| p.feed(l));
    p.into_session(base_meta(path, account), titles)
}

fn quick_meta(path: &Path, account: &str, titles: &HashMap<String, String>) -> SessionMeta {
    let zst = path.extension().map(|e| e == "zst").unwrap_or(false);
    if zst {
        let mut s = load_file(path, account, titles);
        s.turns.clear();
        return s.meta;
    }
    let mut p = CodexParser::new();
    for l in util::head_lines(path, 60, 2 * 1024 * 1024) {
        p.feed(l.as_bytes());
    }
    let head = p.turns.len();
    // O rodapé traz o último acumulado: vira o total da sessão.
    let mut last_total: Option<Raw> = None;
    let mut last_ms = None;
    for l in util::tail_lines(path, 256 * 1024) {
        if let Ok(v) = serde_json::from_str::<Value>(&l) {
            if let Some(ms) = util::ts_ms_of(v.get("timestamp")) {
                last_ms = Some(ms);
            }
            if let Some(pl) = v.get("payload") {
                if util::str_of(pl, "type") == Some("token_count") {
                    if let Some(t) = pl.get("info").and_then(|i| i.get("total_token_usage")) {
                        last_total = Some(Raw::of(t));
                    }
                }
                if v.get("type").and_then(|t| t.as_str()) == Some("turn_context") {
                    if let Some(m) = util::str_of(pl, "model") {
                        p.model = Some(m.to_string());
                    }
                }
            }
        }
    }
    let model = p.model.clone();
    let mut s = p.into_session(base_meta(path, account), titles);
    if let Some(ms) = last_ms {
        s.meta.ended = Some(util::ms_to_rfc3339(ms));
    }
    if let Some(t) = last_total {
        s.meta.usage = t.normalized();
    }
    if let Some(m) = model {
        if !s.meta.models.contains(&m) {
            s.meta.models.push(m);
        }
    }
    s.meta.turn_count = head as u32;
    s.meta
}

impl SessionSource for CodexSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        let mut v = Vec::new();
        for r in &self.roots {
            v.push(r.home.join("sessions"));
            v.push(r.home.join("archived_sessions"));
        }
        v
    }

    fn list(&self) -> Vec<SessionMeta> {
        let titles = self.titles();
        let mut cache = self.cache.lock().map(|g| g.clone()).unwrap_or_default();
        let mut ids = HashMap::new();
        let mut out = Vec::new();
        for (path, account) in self.files() {
            let mtime = util::file_mtime_ms(&path);
            let size = util::file_size(&path);
            let mut meta = match cache.get(&path) {
                Some((m, s, meta)) if *m == mtime && *s == size => meta.clone(),
                _ => {
                    let meta = quick_meta(&path, &account, &titles);
                    cache.insert(path.clone(), (mtime, size, meta.clone()));
                    meta
                }
            };
            if let Some(t) = titles.get(&meta.id) {
                meta.title = Some(t.clone());
            }
            ids.insert(meta.id.clone(), (path, account));
            out.push(meta);
        }
        if let Ok(mut g) = self.cache.lock() {
            *g = cache;
        }
        if let Ok(mut g) = self.ids.lock() {
            *g = ids;
        }
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        let (path, account) = self.path_of(id)?;
        Some(load_file(&path, &account, &self.titles()))
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        let root = meta.account.as_deref().and_then(|a| self.home_of(a));
        let env = root.filter(|r| !r.is_default).map(|r| r.home.clone());
        let cmd = util::with_env(
            "CODEX_HOME",
            env.as_deref(),
            format!("codex resume {}", meta.id),
        );
        Some(util::in_dir(meta.project_path.as_deref(), cmd))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{"timestamp":"2026-09-20T10:00:00.000Z","type":"session_meta","payload":{"id":"01a0c117-ee53-7b43-9930-8d41b55a4f00","cwd":"/w/p","git":{"branch":"main"}}}
{"timestamp":"2026-09-20T10:00:00.100Z","type":"turn_context","payload":{"model":"gpt-6-astra","cwd":"/w/p"}}
{"timestamp":"2026-09-20T10:00:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>x</environment_context>"},{"type":"input_text","text":"liste os arquivos"}]}}
{"timestamp":"2026-09-20T10:00:02.000Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"ls\"}","call_id":"c1"}}
{"timestamp":"2026-09-20T10:00:03.000Z","type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"a.txt"}}
{"timestamp":"2026-09-20T10:00:03.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":800,"output_tokens":50,"reasoning_output_tokens":10},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":800,"output_tokens":50,"reasoning_output_tokens":10}}}}
{"timestamp":"2026-09-20T10:00:03.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":800,"output_tokens":50,"reasoning_output_tokens":10},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":800,"output_tokens":50,"reasoning_output_tokens":10}}}}
{"timestamp":"2026-09-20T10:00:04.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Tem a.txt"}]}}
{"timestamp":"2026-09-20T10:00:04.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2100,"cached_input_tokens":1800,"output_tokens":70,"reasoning_output_tokens":10},"last_token_usage":{"input_tokens":1100,"cached_input_tokens":1000,"output_tokens":20,"reasoning_output_tokens":0}}}}
"#;

    #[test]
    fn cumulative_counts_become_deltas_and_the_total_is_the_last() {
        let mut p = CodexParser::new();
        for l in FIXTURE.lines() {
            p.feed(l.as_bytes());
        }
        let s = p.into_session(
            base_meta(Path::new("/x/rollout-a.jsonl"), "default"),
            &HashMap::new(),
        );
        // Sem uuid no nome, o id vem do `session_meta`.
        assert_eq!(s.meta.id, "01a0c117-ee53-7b43-9930-8d41b55a4f00");
        assert_eq!(s.turns[0].text, "liste os arquivos");
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Bash");
        assert_eq!(s.turns[1].tool_calls[0].result.as_deref(), Some("a.txt"));
        // Total = último acumulado, normalizado.
        assert_eq!(s.meta.usage.input, 300);
        assert_eq!(s.meta.usage.cache_read, 1800);
        assert_eq!(s.meta.usage.output, 60);
        assert_eq!(s.meta.usage.reasoning, 10);
        let summed: u64 = s.turns.iter().map(|t| t.usage.total()).sum();
        assert_eq!(summed, s.meta.usage.total());
        assert_eq!(s.meta.models, vec!["gpt-6-astra".to_string()]);
    }

    #[test]
    fn file_id_is_the_trailing_uuid() {
        let p = Path::new(
            "/x/rollout-2026-09-20T20-12-46-01a0c117-ee53-7b43-9930-8d41b55a4f00.jsonl.zst",
        );
        assert_eq!(
            file_id(p).as_deref(),
            Some("01a0c117-ee53-7b43-9930-8d41b55a4f00")
        );
    }
}
