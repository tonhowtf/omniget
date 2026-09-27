//! Claude Code: `<config>/projects/<cwd codificado>/<session>.jsonl`, com
//! subagentes em `<session>/subagents/agent-*.jsonl` (+ `.meta.json`).
//!
//! Raízes: `CLAUDE_CONFIG_DIR` (lista), senão `~/.claude` e
//! `~/.config/claude`; mais as contas isoladas do OmniGet.
//!
//! Regras de contagem:
//! * Uma mensagem da API vira várias linhas (uma por bloco de conteúdo) que
//!   repetem `message.id` e `usage`. Tudo com a mesma chave
//!   `message.id:requestId` vira uma `Turn` só, com o maior valor de cada
//!   campo de uso (o streaming grava parciais crescentes).
//! * `input` é a entrada fresca; `cache_read`/`cache_write` separados;
//!   `reasoning` = `output_tokens_details.thinking_tokens`, descontado de
//!   `output` (o total da API inclui o raciocínio).
//! * A linha `cost-state` é o custo que o próprio CLI calculou. Cada processo
//!   (`startTime`) acumula desde que abriu; o custo da sessão é a soma do
//!   último valor de cada processo.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Deserialize;

use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolCall, ToolStatus, Turn,
};
use crate::core::sessions::util::{self, canonical_tool, truncate_chars, value_text, RESULT_MAX};

pub const TOOL: &str = "claude";

#[derive(Debug, Clone)]
pub struct Root {
    pub account: String,
    /// Diretório de configuração (`CLAUDE_CONFIG_DIR`), `None` = padrão.
    pub config_dir: Option<PathBuf>,
    pub projects: PathBuf,
}

pub struct ClaudeSource {
    roots: Vec<Root>,
    /// Metadados baratos por arquivo, válidos enquanto (mtime, tamanho) não mudam.
    cache: Mutex<HashMap<PathBuf, (i64, u64, SessionMeta)>>,
    /// id → (arquivo, conta), preenchido pelo `list`.
    ids: Mutex<HashMap<String, (PathBuf, String)>>,
}

impl Default for ClaudeSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeSource {
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

    /// Todos os arquivos de sessão: (arquivo, conta, id, sessão-mãe).
    pub fn files(&self) -> Vec<(PathBuf, String, String, Option<String>)> {
        let mut out = Vec::new();
        for root in &self.roots {
            let Ok(projects) = std::fs::read_dir(&root.projects) else {
                continue;
            };
            for proj in projects.flatten() {
                let pdir = proj.path();
                if !pdir.is_dir() {
                    continue;
                }
                let Ok(entries) = std::fs::read_dir(&pdir) else {
                    continue;
                };
                for e in entries.flatten() {
                    let p = e.path();
                    let name = e.file_name().to_string_lossy().to_string();
                    if p.is_file() && name.ends_with(".jsonl") {
                        let id = name.trim_end_matches(".jsonl").to_string();
                        out.push((p, root.account.clone(), id, None));
                    } else if p.is_dir() {
                        let sub = p.join("subagents");
                        let Ok(subs) = std::fs::read_dir(&sub) else {
                            continue;
                        };
                        for s in subs.flatten() {
                            let sp = s.path();
                            let sname = s.file_name().to_string_lossy().to_string();
                            if sp.is_file() && sname.ends_with(".jsonl") {
                                out.push((
                                    sp,
                                    root.account.clone(),
                                    sname.trim_end_matches(".jsonl").to_string(),
                                    Some(name.clone()),
                                ));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    pub fn path_of(&self, id: &str) -> Option<(PathBuf, String)> {
        if let Some(hit) = self.ids.lock().ok().and_then(|m| m.get(id).cloned()) {
            if hit.0.exists() {
                return Some(hit);
            }
        }
        for (p, acc, fid, _) in self.files() {
            if fid == id {
                if let Ok(mut m) = self.ids.lock() {
                    m.insert(id.to_string(), (p.clone(), acc.clone()));
                }
                return Some((p, acc));
            }
        }
        None
    }

    pub fn config_dir_of(&self, account: &str) -> Option<PathBuf> {
        self.roots
            .iter()
            .find(|r| r.account == account)
            .and_then(|r| r.config_dir.clone())
    }
}

pub fn default_roots() -> Vec<Root> {
    let mut roots: Vec<Root> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut push =
        |roots: &mut Vec<Root>, account: String, config: Option<PathBuf>, base: PathBuf| {
            let projects = base.join("projects");
            let c = util::canon(&projects);
            if seen.contains(&c) {
                return;
            }
            seen.push(c);
            roots.push(Root {
                account,
                config_dir: config,
                projects,
            });
        };
    let env = util::env_paths("CLAUDE_CONFIG_DIR");
    if env.is_empty() {
        if let Some(h) = util::home() {
            push(&mut roots, "default".into(), None, h.join(".claude"));
            push(
                &mut roots,
                "default".into(),
                None,
                h.join(".config").join("claude"),
            );
        }
    } else {
        for (i, d) in env.iter().enumerate() {
            let acc = if i == 0 {
                "default".to_string()
            } else {
                d.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("config-{i}"))
            };
            push(&mut roots, acc, Some(d.clone()), d.clone());
        }
    }
    for (id, dir) in super::account_dirs("claude") {
        push(&mut roots, id, Some(dir.clone()), dir);
    }
    roots
}

/// `<arquivo>.meta.json` de um subagente: tipo, descrição e o `tool_use` que
/// o criou na sessão-mãe.
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentMeta {
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub spawn_depth: Option<u32>,
}

pub fn subagent_meta(session_file: &Path) -> Option<SubagentMeta> {
    let p = session_file.with_extension("meta.json");
    let text = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&text).ok()
}

/// Uma linha do JSONL, só com os campos usados (o resto é pulado sem
/// construir árvore, o que mantém a varredura fria rápida).
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    subtype: Option<String>,
    uuid: Option<String>,
    timestamp: Option<String>,
    cwd: Option<String>,
    git_branch: Option<String>,
    is_meta: Option<bool>,
    is_api_error_message: Option<bool>,
    request_id: Option<String>,
    message: Option<serde_json::Value>,
    content: Option<serde_json::Value>,
    ai_title: Option<String>,
    custom_title: Option<String>,
    summary: Option<String>,
    agent_name: Option<String>,
    #[serde(rename = "totalCostUSD")]
    total_cost_usd: Option<f64>,
    start_time: Option<i64>,
}

/// Parser incremental: alimente linhas, leia o estado.
#[derive(Default)]
pub struct ClaudeParser {
    pub turns: Vec<Turn>,
    by_key: HashMap<String, usize>,
    calls: HashMap<String, (usize, usize)>,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub ai_title: Option<String>,
    pub custom_title: Option<String>,
    pub summary: Option<String>,
    pub agent_name: Option<String>,
    pub first_user: Option<String>,
    /// `startTime` do processo → último `totalCostUSD` visto.
    pub cost_states: BTreeMap<i64, f64>,
    /// Resultados de tool cujo `tool_use` não está nesta leitura (leitura
    /// incremental do índice).
    pub orphan_results: Vec<(String, ToolStatus)>,
    pub first_ms: Option<i64>,
    pub last_ms: Option<i64>,
    pub lines: u64,
}

fn usage_of(u: &serde_json::Value) -> TokenUsage {
    let output = util::num(u, "output_tokens");
    let reasoning = u
        .get("output_tokens_details")
        .map(|d| util::num(d, "thinking_tokens"))
        .unwrap_or(0)
        .min(output);
    TokenUsage {
        input: util::num(u, "input_tokens"),
        output: output - reasoning,
        cache_read: util::num(u, "cache_read_input_tokens"),
        cache_write: util::num(u, "cache_creation_input_tokens"),
        reasoning,
    }
}

fn max_usage(a: &mut TokenUsage, b: &TokenUsage) {
    a.input = a.input.max(b.input);
    a.output = a.output.max(b.output);
    a.cache_read = a.cache_read.max(b.cache_read);
    a.cache_write = a.cache_write.max(b.cache_write);
    a.reasoning = a.reasoning.max(b.reasoning);
}

fn subagent_of(name: &str, input: &serde_json::Value) -> Option<String> {
    if canonical_tool(name) != "Agent" {
        return None;
    }
    util::str_of(input, "subagent_type")
        .or_else(|| util::str_of(input, "agent_type"))
        .or_else(|| util::str_of(input, "name"))
        .map(|s| s.to_string())
        .or_else(|| Some("general-purpose".into()))
}

impl ClaudeParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, raw: &[u8]) {
        if raw.is_empty() {
            return;
        }
        let Ok(line) = serde_json::from_slice::<Line>(raw) else {
            return;
        };
        self.lines += 1;
        let ts_ms = line.timestamp.as_deref().and_then(util::parse_ts_str);
        if let Some(ms) = ts_ms {
            self.first_ms = Some(self.first_ms.map_or(ms, |f| f.min(ms)));
            self.last_ms = Some(self.last_ms.map_or(ms, |l| l.max(ms)));
        }
        if self.cwd.is_none() {
            if let Some(c) = line.cwd.as_ref().filter(|c| !c.is_empty()) {
                self.cwd = Some(c.clone());
            }
        }
        if let Some(b) = line
            .git_branch
            .as_ref()
            .filter(|b| !b.is_empty() && *b != "HEAD")
        {
            self.branch = Some(b.clone());
        }
        let ts = line.timestamp.clone().unwrap_or_default();
        match line.kind.as_deref() {
            Some("assistant") => self.assistant(line, ts, ts_ms),
            Some("user") => self.user(line, ts, ts_ms),
            Some("system") => {
                if line.subtype.as_deref() == Some("local_command") {
                    let text = line
                        .content
                        .as_ref()
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !text.is_empty() {
                        self.turns.push(Turn {
                            role: Role::System,
                            ts,
                            text,
                            tool_calls: Vec::new(),
                            usage: TokenUsage::default(),
                            cost_usd: None,
                            model: None,
                            message_id: line.uuid,
                        });
                    }
                }
            }
            Some("ai-title") => {
                if let Some(t) = line.ai_title {
                    self.ai_title = Some(t);
                }
            }
            Some("custom-title") => {
                if let Some(t) = line.custom_title {
                    self.custom_title = Some(t);
                }
            }
            Some("summary") => {
                if let Some(t) = line.summary {
                    self.summary = Some(t);
                }
            }
            Some("agent-name") => {
                if let Some(t) = line.agent_name {
                    self.agent_name = Some(t);
                }
            }
            Some("cost-state") => {
                if let Some(c) = line.total_cost_usd {
                    self.cost_states.insert(line.start_time.unwrap_or(0), c);
                }
            }
            _ => {}
        }
    }

    fn assistant(&mut self, mut line: Line, ts: String, _ts_ms: Option<i64>) {
        let Some(mut msg) = line.message.take() else {
            return;
        };
        let msg = &mut msg;
        let model = util::str_of(msg, "model").map(|s| s.to_string());
        let synthetic =
            model.as_deref() == Some("<synthetic>") || line.is_api_error_message == Some(true);
        let key = match (util::str_of(msg, "id"), line.request_id.as_deref()) {
            (Some(id), Some(req)) => format!("{id}:{req}"),
            (Some(id), None) => id.to_string(),
            _ => line
                .uuid
                .clone()
                .unwrap_or_else(|| format!("line-{}", self.lines)),
        };
        let usage = if synthetic {
            TokenUsage::default()
        } else {
            msg.get("usage").map(usage_of).unwrap_or_default()
        };
        let idx = match self.by_key.get(&key) {
            Some(i) => *i,
            None => {
                self.turns.push(Turn {
                    role: Role::Assistant,
                    ts,
                    text: String::new(),
                    tool_calls: Vec::new(),
                    usage: TokenUsage::default(),
                    cost_usd: None,
                    model: if synthetic { None } else { model.clone() },
                    message_id: Some(key.clone()),
                });
                let i = self.turns.len() - 1;
                self.by_key.insert(key, i);
                i
            }
        };
        max_usage(&mut self.turns[idx].usage, &usage);
        let blocks: Vec<serde_json::Value> = match msg.get_mut("content").map(|c| c.take()) {
            Some(serde_json::Value::Array(a)) => a,
            Some(serde_json::Value::String(s)) => {
                vec![serde_json::json!({"type": "text", "text": s})]
            }
            _ => Vec::new(),
        };
        for b in blocks {
            match b.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                        let turn = &mut self.turns[idx];
                        if !turn.text.is_empty() {
                            turn.text.push('\n');
                        }
                        turn.text.push_str(t);
                    }
                }
                Some("tool_use") | Some("server_tool_use") => {
                    let id = util::str_of(&b, "id").unwrap_or("").to_string();
                    let name = util::str_of(&b, "name").unwrap_or("?").to_string();
                    let input = b.get("input").cloned().unwrap_or(serde_json::Value::Null);
                    let turn = &mut self.turns[idx];
                    if !id.is_empty() && turn.tool_calls.iter().any(|c| c.id == id) {
                        continue;
                    }
                    let call = ToolCall {
                        id: id.clone(),
                        name_canonical: canonical_tool(&name).to_string(),
                        subagent: subagent_of(&name, &input),
                        name_raw: name,
                        input,
                        result: None,
                        status: ToolStatus::Pending,
                        ms: None,
                    };
                    turn.tool_calls.push(call);
                    if !id.is_empty() {
                        self.calls.insert(id, (idx, turn.tool_calls.len() - 1));
                    }
                }
                _ => {}
            }
        }
    }

    fn user(&mut self, line: Line, ts: String, ts_ms: Option<i64>) {
        let Some(msg) = line.message.as_ref() else {
            return;
        };
        let mut texts: Vec<String> = Vec::new();
        match msg.get("content") {
            Some(serde_json::Value::String(s)) => texts.push(s.clone()),
            Some(serde_json::Value::Array(blocks)) => {
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                                texts.push(t.to_string());
                            }
                        }
                        Some("image") => texts.push("[image]".into()),
                        Some("tool_result") => {
                            let id = util::str_of(b, "tool_use_id").unwrap_or("").to_string();
                            let is_err = b.get("is_error").and_then(|e| e.as_bool()) == Some(true);
                            let status = if is_err {
                                ToolStatus::Error
                            } else {
                                ToolStatus::Ok
                            };
                            let result = b
                                .get("content")
                                .map(|c| value_text(c, RESULT_MAX))
                                .unwrap_or_default();
                            match self.calls.get(&id) {
                                Some(&(ti, ci)) => {
                                    let turn_ms = util::parse_ts_str(&self.turns[ti].ts);
                                    let call = &mut self.turns[ti].tool_calls[ci];
                                    call.result = Some(result);
                                    call.status = status;
                                    if let (Some(a), Some(b)) = (turn_ms, ts_ms) {
                                        if b >= a {
                                            call.ms = Some((b - a) as u64);
                                        }
                                    }
                                }
                                None => {
                                    if !id.is_empty() {
                                        self.orphan_results.push((id, status));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if line.is_meta == Some(true) {
            return;
        }
        let text = texts.join("\n");
        if text.trim().is_empty() {
            return;
        }
        if self.first_user.is_none()
            && !text.starts_with('<')
            && !text.starts_with("[Request interrupted")
        {
            self.first_user = Some(text.clone());
        }
        self.turns.push(Turn {
            role: Role::User,
            ts,
            text,
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            cost_usd: None,
            model: None,
            message_id: line.uuid,
        });
    }

    /// Custo gravado pelo CLI (soma do último `cost-state` de cada processo).
    pub fn recorded_cost(&self) -> Option<f64> {
        if self.cost_states.is_empty() {
            None
        } else {
            Some(self.cost_states.values().sum())
        }
    }

    /// Só títulos gravados explicitamente (sem cair na primeira mensagem).
    pub fn explicit_title(&self) -> Option<String> {
        self.custom_title
            .clone()
            .or_else(|| self.ai_title.clone())
            .or_else(|| self.summary.clone())
            .or_else(|| self.agent_name.clone())
    }

    pub fn title(&self) -> Option<String> {
        self.custom_title
            .clone()
            .or_else(|| self.ai_title.clone())
            .or_else(|| self.summary.clone())
            .or_else(|| self.agent_name.clone())
            .or_else(|| {
                self.first_user
                    .as_ref()
                    .map(|t| truncate_chars(t.lines().next().unwrap_or(t).trim(), 80))
            })
    }

    pub fn into_session(self, meta_base: SessionMeta) -> Session {
        let mut meta = meta_base;
        meta.title = self.title().or(meta.title);
        if self.cwd.is_some() {
            meta.project_path = self.cwd.clone();
        }
        if self.branch.is_some() {
            meta.git_branch = self.branch.clone();
        }
        let first = self
            .turns
            .iter()
            .filter_map(|t| util::parse_ts_str(&t.ts))
            .min()
            .or(self.first_ms);
        let last = self
            .turns
            .iter()
            .filter_map(|t| util::parse_ts_str(&t.ts))
            .max()
            .or(self.last_ms);
        meta.started = first.map(util::ms_to_rfc3339);
        meta.ended = last.map(util::ms_to_rfc3339);
        let mut models: Vec<String> = Vec::new();
        let mut usage = TokenUsage::default();
        for t in &self.turns {
            if let Some(m) = &t.model {
                if !models.contains(m) {
                    models.push(m.clone());
                }
            }
            usage.add(&t.usage);
        }
        meta.models = models;
        meta.usage = usage;
        meta.turn_count = self.turns.len() as u32;
        meta.cost_usd = self.recorded_cost().unwrap_or(0.0);
        Session {
            meta,
            turns: self.turns,
        }
    }
}

/// Lê o arquivo a partir de `from`; devolve o parser e o offset final.
pub fn parse_from(path: &Path, from: u64) -> (ClaudeParser, u64) {
    let mut p = ClaudeParser::new();
    let end = util::for_each_line(path, from, |l| p.feed(l)).unwrap_or(from);
    (p, end)
}

fn base_meta(path: &Path, account: &str, id: &str, parent: Option<String>) -> SessionMeta {
    SessionMeta {
        tool: TOOL.into(),
        account: Some(account.to_string()),
        id: id.to_string(),
        title: None,
        project_path: None,
        git_branch: None,
        started: None,
        ended: None,
        models: Vec::new(),
        parent_session: parent,
        turn_count: 0,
        usage: TokenUsage::default(),
        cost_usd: 0.0,
        source: path.to_path_buf(),
        mtime_ms: util::file_mtime_ms(path),
    }
}

/// Sessão-mãe de um arquivo `.../<sessão>/subagents/agent-x.jsonl`.
pub fn parent_of(path: &Path) -> Option<String> {
    let dir = path.parent()?;
    if dir.file_name()?.to_str()? != "subagents" {
        return None;
    }
    Some(dir.parent()?.file_name()?.to_string_lossy().to_string())
}

/// Sessão completa de um arquivo.
pub fn load_file(path: &Path, account: &str, id: &str) -> Session {
    let (p, _) = parse_from(path, 0);
    let mut s = p.into_session(base_meta(path, account, id, parent_of(path)));
    decorate_subagent(path, &mut s.meta);
    s
}

pub fn decorate_subagent(path: &Path, meta: &mut SessionMeta) {
    if meta.parent_session.is_none() {
        return;
    }
    if let Some(sm) = subagent_meta(path) {
        let label = match (sm.agent_type, sm.description) {
            (Some(t), Some(d)) => format!("{t}: {d}"),
            (Some(t), None) => t,
            (None, Some(d)) => d,
            _ => return,
        };
        meta.title = Some(label);
    } else if meta.id.starts_with("agent-acompact") {
        meta.title = Some("compact".into());
    }
}

/// Metadados baratos: cabeçalho + rodapé do arquivo.
fn quick_meta(path: &Path, account: &str, id: &str, parent: Option<String>) -> SessionMeta {
    let mut p = ClaudeParser::new();
    for l in util::head_lines(path, 20, 128 * 1024) {
        p.feed(l.as_bytes());
    }
    let head_turns = p.turns.len();
    for l in util::tail_lines(path, 64 * 1024) {
        p.feed(l.as_bytes());
    }
    let mut meta = base_meta(path, account, id, parent);
    meta.title = p.title();
    meta.project_path = p.cwd.clone();
    meta.git_branch = p.branch.clone();
    meta.started = p.first_ms.map(util::ms_to_rfc3339);
    meta.ended = p.last_ms.map(util::ms_to_rfc3339);
    for t in &p.turns {
        if let Some(m) = &t.model {
            if !meta.models.contains(m) {
                meta.models.push(m.clone());
            }
        }
    }
    meta.turn_count = head_turns.max(p.turns.len()) as u32;
    meta.cost_usd = p.recorded_cost().unwrap_or(0.0);
    decorate_subagent(path, &mut meta);
    meta
}

impl SessionSource for ClaudeSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|r| r.projects.clone()).collect()
    }

    fn list(&self) -> Vec<SessionMeta> {
        let files = self.files();
        let mut out = Vec::with_capacity(files.len());
        let mut cache = self.cache.lock().map(|g| g.clone()).unwrap_or_default();
        let mut ids = HashMap::new();
        for (path, account, id, parent) in files {
            let mtime = util::file_mtime_ms(&path);
            let size = util::file_size(&path);
            let meta = match cache.get(&path) {
                Some((m, s, meta)) if *m == mtime && *s == size => meta.clone(),
                _ => {
                    let meta = quick_meta(&path, &account, &id, parent);
                    cache.insert(path.clone(), (mtime, size, meta.clone()));
                    meta
                }
            };
            ids.insert(id, (path, account));
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
        Some(load_file(&path, &account, id))
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        if meta.parent_session.is_some() {
            return None;
        }
        let cfg = meta.account.as_deref().and_then(|a| self.config_dir_of(a));
        let cmd = util::with_env(
            "CLAUDE_CONFIG_DIR",
            cfg.as_deref(),
            format!("claude --resume {}", meta.id),
        );
        Some(util::in_dir(meta.project_path.as_deref(), cmd))
    }
}

/// Estado ao vivo de um arquivo pelo rodapé: último papel e se há tool
/// esperando resultado.
pub fn tail_state(path: &Path) -> (Option<Role>, bool) {
    let mut p = ClaudeParser::new();
    for l in util::tail_lines(path, 128 * 1024) {
        p.feed(l.as_bytes());
    }
    let last = p.turns.iter().rev().find(|t| t.role != Role::System);
    let pending = last
        .map(|t| t.tool_calls.iter().any(|c| c.status == ToolStatus::Pending))
        .unwrap_or(false);
    (last.map(|t| t.role), pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        [
            r#"{"type":"user","uuid":"u1","sessionId":"s1","cwd":"/w/proj","gitBranch":"main","timestamp":"2026-09-20T10:00:00.000Z","message":{"role":"user","content":"conserte o bug"}}"#,
            r#"{"type":"assistant","uuid":"a1","requestId":"req_1","timestamp":"2026-09-20T10:00:05.000Z","message":{"id":"msg_1","model":"claude-opus-5","role":"assistant","content":[{"type":"text","text":"Vou olhar."}],"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"output_tokens":50,"output_tokens_details":{"thinking_tokens":20}}}}"#,
            r#"{"type":"assistant","uuid":"a2","requestId":"req_1","timestamp":"2026-09-20T10:00:06.000Z","message":{"id":"msg_1","model":"claude-opus-5","role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"output_tokens":50,"output_tokens_details":{"thinking_tokens":20}}}}"#,
            r#"{"type":"user","uuid":"u2","timestamp":"2026-09-20T10:00:08.000Z","message":{"role":"user","content":[{"tool_use_id":"toolu_1","type":"tool_result","content":"a.txt","is_error":false}]}}"#,
            r#"{"type":"assistant","uuid":"a3","requestId":"req_2","timestamp":"2026-09-20T10:00:09.000Z","message":{"id":"msg_2","model":"claude-opus-5","role":"assistant","content":[{"type":"tool_use","id":"toolu_2","name":"Agent","input":{"subagent_type":"Explore","description":"achar"}}],"usage":{"input_tokens":5,"cache_creation_input_tokens":0,"cache_read_input_tokens":1100,"output_tokens":30}}}"#,
            r#"{"type":"ai-title","aiTitle":"Conserto do bug","sessionId":"s1"}"#,
            r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":0.10,"startTime":1}"#,
            r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":0.25,"startTime":1}"#,
            r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":0.05,"startTime":2}"#,
        ]
        .join("\n")
            + "\n"
    }

    #[test]
    fn lines_of_one_message_become_one_turn_and_usage_counts_once() {
        let mut p = ClaudeParser::new();
        for l in fixture().lines() {
            p.feed(l.as_bytes());
        }
        let s = p.into_session(base_meta(Path::new("/x/s1.jsonl"), "default", "s1", None));
        assert_eq!(s.turns.len(), 3, "user + 2 assistant messages");
        let a = &s.turns[1];
        assert_eq!(a.tool_calls.len(), 1);
        assert_eq!(a.tool_calls[0].status, ToolStatus::Ok);
        assert_eq!(a.tool_calls[0].result.as_deref(), Some("a.txt"));
        assert_eq!(a.tool_calls[0].ms, Some(3000));
        assert_eq!(a.usage.output, 30);
        assert_eq!(a.usage.reasoning, 20);
        assert_eq!(s.meta.usage.input, 15);
        assert_eq!(s.meta.usage.cache_read, 2100);
        assert_eq!(
            s.turns[2].tool_calls[0].subagent.as_deref(),
            Some("Explore")
        );
        assert_eq!(s.meta.title.as_deref(), Some("Conserto do bug"));
        assert_eq!(s.meta.project_path.as_deref(), Some("/w/proj"));
        assert!((s.meta.cost_usd - 0.30).abs() < 1e-9);
    }

    #[test]
    fn tail_parse_resumes_at_the_offset() {
        let dir = std::env::temp_dir().join(format!("omniget-claude-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s1.jsonl");
        let text = fixture();
        let cut = text.find("\n{\"type\":\"user\",\"uuid\":\"u2\"").unwrap() + 1;
        std::fs::write(&f, &text[..cut]).unwrap();
        let (p1, off) = parse_from(&f, 0);
        assert_eq!(off as usize, cut);
        assert_eq!(p1.turns.len(), 2);
        std::fs::write(&f, &text).unwrap();
        let (p2, _) = parse_from(&f, off);
        assert_eq!(
            p2.orphan_results,
            vec![("toolu_1".to_string(), ToolStatus::Ok)]
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
