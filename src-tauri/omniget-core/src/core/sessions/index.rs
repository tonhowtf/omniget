//! Índice incremental das sessões em `<app_data>/llm/sessions.db` (SQLite).
//!
//! * Uma linha por sessão (`sessions`) com a impressão digital da fonte
//!   (mtime + tamanho do arquivo, ou mtime + contagem + tokens para fontes
//!   em banco) e, no Claude, o offset em bytes já lido: arquivo que só
//!   cresceu é lido a partir do offset, o resto é relido inteiro.
//! * Linhas derivadas para as visões agregadas: `usage` (uma por resposta
//!   com uso), `calls` (tool calls, slash commands) e `msgs` (mensagens de
//!   usuário/assistente). As chaves primárias fazem o dedupe: no Claude a
//!   chave de uso é `message.id:requestId` **global**, então a mesma resposta
//!   copiada para outro arquivo (retomada/fork) conta uma vez só.
//! * Nada roda em repouso: o índice só é atualizado quando uma visão pede
//!   (`ensure_fresh`), com um intervalo mínimo entre varreduras.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::cost::PriceBook;
use super::model::{Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus};
use super::parsers::group_a::claude;
use super::util;

pub const ERR_INDEX: &str = "ERR_SESSIONS_INDEX";
const SCHEMA_VERSION: &str = "1";

pub fn db_path() -> Option<PathBuf> {
    crate::core::llm::roster_store::llm_dir().map(|d| d.join("sessions.db"))
}

pub struct SessionIndex {
    conn: Connection,
    last_refresh: Option<Instant>,
    /// Raízes do Claude para a varredura (`None` = as padrão da máquina).
    claude_roots: Option<Vec<claude::Root>>,
}

static INDEX: Mutex<Option<SessionIndex>> = Mutex::new(None);

/// Roda `f` com o índice global (abre na primeira vez).
pub fn with_index<R>(f: impl FnOnce(&mut SessionIndex) -> R) -> Result<R, String> {
    let mut g = INDEX.lock().map_err(|_| format!("{ERR_INDEX}: lock"))?;
    if g.is_none() {
        let p = db_path().ok_or_else(|| format!("{ERR_INDEX}: sem diretório de dados"))?;
        *g = Some(SessionIndex::open(&p)?);
    }
    Ok(f(g.as_mut().unwrap()))
}

/// Diretórios de projeto que o índice já viu (usado pelo Crush para achar
/// `.crush/crush.db`). Abre uma conexão própria, só leitura: pode ser chamado
/// de dentro de uma varredura sem travar o índice global.
pub fn known_projects() -> Vec<PathBuf> {
    let Some(p) = db_path().filter(|p| p.is_file()) else {
        return Vec::new();
    };
    let Some(c) = util::open_ro(&p) else {
        return Vec::new();
    };
    let Ok(mut st) = c.prepare(
        "SELECT DISTINCT project_path FROM sessions WHERE project_path IS NOT NULL AND project_path <> '' AND tool <> 'crush'",
    ) else {
        return Vec::new();
    };
    let rows = st.query_map([], |r| r.get::<_, String>(0));
    let mut out: Vec<PathBuf> = match rows {
        Ok(it) => it
            .flatten()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out.dedup();
    out
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListFilter {
    pub tool: Option<String>,
    pub account: Option<String>,
    /// Substring do caminho do projeto.
    pub project: Option<String>,
    /// `YYYY-MM-DD` local, inclusivo (sessões que tocam o intervalo).
    pub from: Option<String>,
    pub to: Option<String>,
    /// Substring do título.
    pub query: Option<String>,
    /// Inclui subagentes na lista (padrão: só sessões principais).
    #[serde(default)]
    pub include_subagents: bool,
    /// Só os filhos desta sessão.
    pub parent: Option<String>,
    /// `recent` (padrão) | `tokens` | `cost` | `oldest`.
    pub sort: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// Uma sessão só (por id).
    #[serde(default)]
    pub id: Option<String>,
    /// Só respostas deste modelo (visões de uso; na lista, sessões que
    /// usaram o modelo).
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ListPage {
    pub total: u32,
    pub sessions: Vec<SessionMeta>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RefreshStats {
    pub sources: u32,
    pub sessions_seen: u32,
    pub full_reads: u32,
    pub tail_reads: u32,
    pub removed: u32,
    pub elapsed_ms: u64,
}

// --- linhas derivadas --------------------------------------------------------

struct UsageRow {
    key: String,
    ts_ms: i64,
    model: String,
    u: TokenUsage,
    cost: Option<f64>,
}

struct CallRow {
    key: String,
    ts_ms: i64,
    name: String,
    raw: String,
    subagent: Option<String>,
    detail: Option<String>,
    status: &'static str,
}

struct MsgRow {
    key: String,
    ts_ms: i64,
    role: &'static str,
}

#[derive(Default)]
struct Rows {
    usage: Vec<UsageRow>,
    calls: Vec<CallRow>,
    msgs: Vec<MsgRow>,
}

fn status_str(s: ToolStatus) -> &'static str {
    match s {
        ToolStatus::Ok => "ok",
        ToolStatus::Error => "error",
        ToolStatus::Pending => "pending",
    }
}

/// `/comando` num texto: `<command-name>/x</command-name>` (Claude/Qwen) ou
/// mensagem que começa com `/palavra`.
pub fn slash_commands(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("<command-name>") {
        let after = &rest[i + "<command-name>".len()..];
        if let Some(j) = after.find("</command-name>") {
            let name = after[..j].trim();
            if !name.is_empty() {
                let n = if name.starts_with('/') {
                    name.to_string()
                } else {
                    format!("/{name}")
                };
                out.push(n);
            }
            rest = &after[j..];
        } else {
            break;
        }
    }
    if out.is_empty() {
        let t = text.trim_start();
        if let Some(first) = t.split_whitespace().next() {
            if first.len() > 1
                && first.starts_with('/')
                && first[1..]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_:".contains(c))
                && first[1..]
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_alphabetic())
                    .unwrap_or(false)
            {
                out.push(first.to_string());
            }
        }
    }
    out
}

fn detail_of(name: &str, raw: &str, input: &serde_json::Value) -> Option<String> {
    match name {
        "Agent" => util::str_of(input, "description")
            .or_else(|| util::str_of(input, "prompt"))
            .or_else(|| util::str_of(input, "message"))
            .map(|s| util::truncate_chars(s, 200)),
        "Skill" => util::str_of(input, "skill")
            .or_else(|| util::str_of(input, "command"))
            .or_else(|| util::str_of(input, "name"))
            .map(|s| s.to_string()),
        "Mcp" => {
            let r = raw.trim_start_matches("mcp__").trim_start_matches("mcp_");
            Some(r.split("__").next().unwrap_or(r).to_string())
        }
        _ => None,
    }
}

fn claude_global_key(tool: &str, mid: &str) -> bool {
    tool == "claude" && (mid.starts_with("msg") || mid.contains(':'))
}

fn rows_of(s: &Session) -> Rows {
    let tool = s.meta.tool.as_str();
    let sid = s.meta.id.as_str();
    let mut rows = Rows::default();
    for (i, t) in s.turns.iter().enumerate() {
        let ts_ms = util::turn_ms(&t.ts);
        let mkey = t.message_id.clone().unwrap_or_else(|| format!("#{i}"));
        match t.role {
            Role::User | Role::Assistant => rows.msgs.push(MsgRow {
                key: mkey.clone(),
                ts_ms,
                role: if t.role == Role::User {
                    "user"
                } else {
                    "assistant"
                },
            }),
            _ => {}
        }
        if t.usage.total() > 0 || t.cost_usd.unwrap_or(0.0) > 0.0 {
            let key = if claude_global_key(tool, &mkey) {
                mkey.clone()
            } else {
                format!("{sid}:{mkey}:{i}")
            };
            rows.usage.push(UsageRow {
                key,
                ts_ms,
                model: t.model.clone().unwrap_or_default(),
                u: t.usage.clone(),
                cost: t.cost_usd,
            });
        }
        for (j, c) in t.tool_calls.iter().enumerate() {
            let key = if c.id.is_empty() {
                format!("{i}:{j}")
            } else {
                c.id.clone()
            };
            rows.calls.push(CallRow {
                key,
                ts_ms,
                name: c.name_canonical.clone(),
                raw: c.name_raw.clone(),
                subagent: c.subagent.clone(),
                detail: detail_of(&c.name_canonical, &c.name_raw, &c.input),
                status: status_str(c.status),
            });
        }
        if matches!(t.role, Role::User | Role::System) {
            for (k, cmd) in slash_commands(&t.text).into_iter().enumerate() {
                rows.calls.push(CallRow {
                    key: format!("{mkey}#cmd{k}"),
                    ts_ms,
                    name: "SlashCommand".into(),
                    raw: cmd.clone(),
                    subagent: None,
                    detail: Some(cmd),
                    status: "ok",
                });
            }
        }
    }
    rows
}

/// Metas mínimos do Claude (arquivo, conta, id, mãe, mtime).
fn claude_file_metas(roots: Option<Vec<claude::Root>>) -> Vec<SessionMeta> {
    let src = match roots {
        Some(r) => claude::ClaudeSource::with_roots(r),
        None => claude::ClaudeSource::new(),
    };
    src.files()
        .into_iter()
        .map(|(path, account, id, parent)| SessionMeta {
            tool: claude::TOOL.into(),
            account: Some(account),
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
            mtime_ms: util::file_mtime_ms(&path),
            source: path,
            id,
        })
        .collect()
}

// --- trabalho de uma varredura ----------------------------------------------

struct Stored {
    fp: String,
    offset: i64,
    tail_state: Option<String>,
    source: String,
}

enum Work {
    Full {
        meta: SessionMeta,
        fp: String,
        size: u64,
    },
    ClaudeTail {
        meta: SessionMeta,
        fp: String,
        offset: u64,
        state: BTreeMap<i64, f64>,
        size: u64,
    },
}

struct Done {
    meta: SessionMeta,
    fp: String,
    rows: Rows,
    tail: bool,
    offset: u64,
    cost_states: Option<BTreeMap<i64, f64>>,
    orphans: Vec<(String, ToolStatus)>,
}

fn run_work(w: Work, sources: &[Box<dyn SessionSource>]) -> Option<Done> {
    match w {
        Work::Full { meta, fp, .. } => {
            if meta.tool == claude::TOOL && meta.source.is_file() {
                let (p, end) = claude::parse_from(&meta.source, 0);
                let states = p.cost_states.clone();
                let path = meta.source.clone();
                let mut s = p.into_session(meta);
                claude::decorate_subagent(&path, &mut s.meta);
                return Some(Done {
                    rows: rows_of(&s),
                    meta: s.meta,
                    fp,
                    tail: false,
                    offset: end,
                    cost_states: Some(states),
                    orphans: Vec::new(),
                });
            }
            let src = sources.iter().find(|s| s.tool() == meta.tool)?;
            let mut s = src.load(&meta.id)?;
            // O `list` pode saber coisas que o `load` não reconstrói.
            if s.meta.title.is_none() {
                s.meta.title = meta.title.clone();
            }
            if s.meta.account.is_none() {
                s.meta.account = meta.account.clone();
            }
            s.meta.mtime_ms = meta.mtime_ms;
            Some(Done {
                rows: rows_of(&s),
                meta: s.meta,
                fp,
                tail: false,
                offset: 0,
                cost_states: None,
                orphans: Vec::new(),
            })
        }
        Work::ClaudeTail {
            meta,
            fp,
            offset,
            mut state,
            ..
        } => {
            let (p, end) = claude::parse_from(&meta.source, offset);
            for (k, v) in &p.cost_states {
                state.insert(*k, *v);
            }
            let orphans = p.orphan_results.clone();
            let title = p.explicit_title();
            let branch = p.branch.clone();
            let mut s = Session {
                meta,
                turns: p.turns,
            };
            if s.meta.parent_session.is_none() {
                if let Some(t) = title {
                    // Só títulos explícitos substituem o que já existe.
                    s.meta.title = Some(t);
                }
            }
            if branch.is_some() {
                s.meta.git_branch = branch;
            }
            Some(Done {
                rows: rows_of(&s),
                meta: s.meta,
                fp,
                tail: true,
                offset: end,
                cost_states: Some(state),
                orphans,
            })
        }
    }
}

/// Uma varredura planejada: o trabalho de parse e o que sumiu.
struct Plan {
    stats: RefreshStats,
    work: Vec<Work>,
    gone: Vec<(String, String)>,
}

/// 1. O que as fontes veem agora (lista barata, em paralelo, sem o banco).
fn list_sources(
    sources: &[Box<dyn SessionSource>],
    claude_roots: &Option<Vec<claude::Root>>,
) -> (Vec<SessionMeta>, HashSet<String>) {
    let mut metas: Vec<SessionMeta> = Vec::new();
    let mut listed_tools: HashSet<String> = HashSet::new();
    std::thread::scope(|s| {
        let handles: Vec<_> = sources
            .iter()
            .map(|src| {
                s.spawn(move || {
                    // Claude: só a lista de arquivos (o parse completo de
                    // quem mudou traz os metadados); poupa ler cabeçalho e
                    // rodapé de milhares de arquivos.
                    if src.tool() == claude::TOOL {
                        return (
                            src.tool().to_string(),
                            claude_file_metas(claude_roots.clone()),
                        );
                    }
                    (src.tool().to_string(), src.list())
                })
            })
            .collect();
        for h in handles {
            if let Ok((tool, list)) = h.join() {
                listed_tools.insert(tool);
                metas.extend(list);
            }
        }
    });
    (metas, listed_tools)
}

/// 2-3. Compara com o índice: o que é novo, o que cresceu, o que sumiu.
fn plan_refresh(
    metas: Vec<SessionMeta>,
    listed_tools: HashSet<String>,
    stored: HashMap<(String, String), Stored>,
    n_sources: usize,
) -> Plan {
    let stats = RefreshStats {
        sources: n_sources as u32,
        sessions_seen: metas.len() as u32,
        ..Default::default()
    };
    let mut per_source: HashMap<PathBuf, u32> = HashMap::new();
    for m in &metas {
        *per_source.entry(m.source.clone()).or_default() += 1;
    }
    let mut work: Vec<Work> = Vec::new();
    let mut present: HashSet<(String, String)> = HashSet::new();
    for m in metas {
        let key = (m.tool.clone(), m.id.clone());
        if !present.insert(key.clone()) {
            continue; // mesmo id em duas raízes: vale a primeira
        }
        let shared = per_source.get(&m.source).copied().unwrap_or(1) > 1;
        let is_file = m.source.is_file();
        let size = if is_file {
            util::file_size(&m.source)
        } else {
            0
        };
        let fp = if shared || !is_file {
            format!("{}:{}:{}", m.mtime_ms, m.turn_count, m.usage.total())
        } else {
            format!("{}:{}", util::file_mtime_ms(&m.source), size)
        };
        match stored.get(&key) {
            Some(s) if s.fp == fp => continue,
            Some(s)
                if m.tool == claude::TOOL
                    && s.offset > 0
                    && (s.offset as u64) <= size
                    && s.source == m.source.to_string_lossy() =>
            {
                let state: BTreeMap<i64, f64> = s
                    .tail_state
                    .as_deref()
                    .and_then(|t| serde_json::from_str(t).ok())
                    .unwrap_or_default();
                work.push(Work::ClaudeTail {
                    meta: m,
                    fp,
                    offset: s.offset as u64,
                    state,
                    size,
                });
            }
            _ => work.push(Work::Full { meta: m, fp, size }),
        }
    }
    let gone: Vec<(String, String)> = stored
        .keys()
        .filter(|k| listed_tools.contains(&k.0) && !present.contains(*k))
        .cloned()
        .collect();
    Plan { stats, work, gone }
}

/// 4. Parse em paralelo (maiores primeiro, para equilibrar), sem o banco.
fn parse_work(mut work: Vec<Work>, sources: &[Box<dyn SessionSource>]) -> Vec<Done> {
    work.sort_by_key(|w| {
        std::cmp::Reverse(match w {
            Work::Full { size, .. } | Work::ClaudeTail { size, .. } => *size,
        })
    });
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 12);
    let queue = Mutex::new(work.into_iter());
    let results: Mutex<Vec<Done>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let next = queue.lock().ok().and_then(|mut q| q.next());
                let Some(w) = next else { break };
                if let Some(d) = run_work(w, sources) {
                    if let Ok(mut r) = results.lock() {
                        r.push(d);
                    }
                }
            });
        }
    });
    results.into_inner().unwrap_or_default()
}

// --- varredura compartilhada -------------------------------------------------

/// Portão das varreduras do índice global: uma por vez. Quem chega durante
/// uma varredura espera por ela e usa o resultado (não varre de novo); as
/// leituras (`with_reader`) seguem em paralelo sobre o banco (WAL).
static REFRESH_GATE: Mutex<Option<Instant>> = Mutex::new(None);

/// Varre o índice global se a última varredura tem mais de `max_age`
/// (`None` = varre sempre). O banco só fica travado para ler as impressões
/// digitais e para gravar; listar e parsear correm sem trava.
pub fn ensure_fresh_global(
    max_age: Option<Duration>,
    sources: &[Box<dyn SessionSource>],
) -> Result<Option<RefreshStats>, String> {
    let mut gate = REFRESH_GATE.lock().unwrap_or_else(|e| e.into_inner());
    if let (Some(age), Some(at)) = (max_age, *gate) {
        if at.elapsed() < age {
            return Ok(None);
        }
    }
    let started = Instant::now();
    let (roots, stored) = with_index(|idx| (idx.claude_roots.clone(), idx.stored()))?;
    let (metas, listed) = list_sources(sources, &roots);
    let mut plan = plan_refresh(metas, listed, stored, sources.len());
    let work = std::mem::take(&mut plan.work);
    let done = parse_work(work, sources);
    let stats = with_index(|idx| {
        let st = idx.commit(plan, done, started);
        idx.last_refresh = Some(Instant::now());
        st
    })?;
    *gate = Some(Instant::now());
    Ok(Some(stats))
}

/// A próxima visão varre de novo.
pub fn mark_stale_global() {
    if let Ok(mut g) = REFRESH_GATE.lock() {
        *g = None;
    }
    let _ = with_index(|idx| idx.mark_stale());
}

/// Apaga o índice e varre do zero (com o portão fechado).
pub fn reset_and_refresh(sources: &[Box<dyn SessionSource>]) -> Result<RefreshStats, String> {
    {
        let _gate = REFRESH_GATE.lock().unwrap_or_else(|e| e.into_inner());
        with_index(|idx| idx.reset())??;
    }
    mark_stale_global();
    Ok(ensure_fresh_global(None, sources)?.unwrap_or_default())
}

/// Conexões só de leitura sobre o mesmo arquivo (WAL): as visões leem em
/// paralelo entre si e com a gravação de uma varredura.
static READERS: Mutex<Vec<(PathBuf, SessionIndex)>> = Mutex::new(Vec::new());
const MAX_READERS: usize = 8;

/// Roda `f` com uma conexão de leitura do índice global.
pub fn with_reader<R>(f: impl FnOnce(&SessionIndex) -> R) -> Result<R, String> {
    let p = db_path().ok_or_else(|| format!("{ERR_INDEX}: sem diretório de dados"))?;
    let taken = {
        let mut g = READERS.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|(path, _)| *path == p);
        g.pop()
    };
    let idx = match taken {
        Some((_, idx)) => idx,
        None => {
            // O esquema nasce na conexão de escrita.
            with_index(|_| ())?;
            SessionIndex::open_reader(&p)?
        }
    };
    let out = f(&idx);
    let mut g = READERS.lock().unwrap_or_else(|e| e.into_inner());
    if g.len() < MAX_READERS {
        g.push((p, idx));
    }
    Ok(out)
}

impl SessionIndex {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{ERR_INDEX}: {e}"))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("{ERR_INDEX}: {e}"))?;
        let _ = conn.busy_timeout(Duration::from_secs(5));
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| format!("{ERR_INDEX}: {e}"))?;
        let mut idx = SessionIndex {
            conn,
            last_refresh: None,
            claude_roots: None,
        };
        idx.migrate()?;
        Ok(idx)
    }

    /// Conexão só de leitura (o esquema já existe).
    pub fn open_reader(path: &Path) -> Result<Self, String> {
        use rusqlite::OpenFlags;
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("{ERR_INDEX}: {e}"))?;
        let _ = conn.busy_timeout(Duration::from_secs(5));
        Ok(SessionIndex {
            conn,
            last_refresh: None,
            claude_roots: None,
        })
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("{ERR_INDEX}: {e}"))?;
        let mut idx = SessionIndex {
            conn,
            last_refresh: None,
            claude_roots: None,
        };
        idx.migrate()?;
        Ok(idx)
    }

    fn migrate(&mut self) -> Result<(), String> {
        let c = &self.conn;
        c.execute_batch("CREATE TABLE IF NOT EXISTS meta(k TEXT PRIMARY KEY, v TEXT)")
            .map_err(|e| format!("{ERR_INDEX}: {e}"))?;
        let v: Option<String> = c
            .query_row("SELECT v FROM meta WHERE k='schema'", [], |r| r.get(0))
            .optional()
            .map_err(|e| format!("{ERR_INDEX}: {e}"))?;
        if v.as_deref() != Some(SCHEMA_VERSION) {
            c.execute_batch(
                "DROP TABLE IF EXISTS sessions; DROP TABLE IF EXISTS usage; DROP TABLE IF EXISTS calls; DROP TABLE IF EXISTS msgs; DROP TABLE IF EXISTS search_text;",
            )
            .map_err(|e| format!("{ERR_INDEX}: {e}"))?;
        }
        c.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sessions(
              tool TEXT NOT NULL, id TEXT NOT NULL, account TEXT, title TEXT,
              project_path TEXT, git_branch TEXT, started_ms INTEGER, ended_ms INTEGER,
              parent_session TEXT, source TEXT, mtime_ms INTEGER, fp TEXT,
              byte_offset INTEGER DEFAULT 0, tail_state TEXT, turn_count INTEGER DEFAULT 0,
              models TEXT, recorded_cost REAL,
              PRIMARY KEY(tool, id));
            CREATE INDEX IF NOT EXISTS sessions_ended ON sessions(ended_ms);
            CREATE INDEX IF NOT EXISTS sessions_parent ON sessions(tool, parent_session);
            CREATE TABLE IF NOT EXISTS usage(
              tool TEXT NOT NULL, key TEXT NOT NULL, session_id TEXT NOT NULL,
              ts_ms INTEGER, day TEXT, hour INTEGER, model TEXT,
              input INTEGER, output INTEGER, cache_read INTEGER, cache_write INTEGER,
              reasoning INTEGER, cost REAL,
              PRIMARY KEY(tool, key));
            CREATE INDEX IF NOT EXISTS usage_session ON usage(tool, session_id);
            CREATE INDEX IF NOT EXISTS usage_day ON usage(day);
            CREATE TABLE IF NOT EXISTS calls(
              tool TEXT NOT NULL, session_id TEXT NOT NULL, key TEXT NOT NULL,
              ts_ms INTEGER, day TEXT, hour INTEGER, name TEXT, raw TEXT,
              subagent TEXT, detail TEXT, status TEXT,
              PRIMARY KEY(tool, session_id, key));
            CREATE INDEX IF NOT EXISTS calls_name ON calls(name, day);
            CREATE TABLE IF NOT EXISTS msgs(
              tool TEXT NOT NULL, session_id TEXT NOT NULL, key TEXT NOT NULL,
              ts_ms INTEGER, day TEXT, hour INTEGER, role TEXT,
              PRIMARY KEY(tool, session_id, key));
            CREATE INDEX IF NOT EXISTS msgs_day ON msgs(day);
            CREATE TABLE IF NOT EXISTS search_text(
              tool TEXT NOT NULL, id TEXT NOT NULL, fp TEXT NOT NULL, body BLOB,
              PRIMARY KEY(tool, id));
            "#,
        )
        .map_err(|e| format!("{ERR_INDEX}: {e}"))?;
        c.execute(
            "INSERT OR REPLACE INTO meta(k, v) VALUES('schema', ?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| format!("{ERR_INDEX}: {e}"))?;
        Ok(())
    }

    /// Troca as raízes do Claude usadas na varredura (testes, perfis).
    pub fn set_claude_roots(&mut self, roots: Vec<claude::Root>) {
        self.claude_roots = Some(roots);
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Atualiza se a última varredura tem mais de `max_age`.
    pub fn ensure_fresh(
        &mut self,
        max_age: Duration,
        sources: &[Box<dyn SessionSource>],
    ) -> Option<RefreshStats> {
        if let Some(at) = self.last_refresh {
            if at.elapsed() < max_age {
                return None;
            }
        }
        Some(self.refresh(sources))
    }

    /// Marca o índice como velho (a próxima visão varre de novo).
    pub fn mark_stale(&mut self) {
        self.last_refresh = None;
    }

    /// Custo que a ferramenta gravou para a sessão inteira (Claude
    /// `cost-state`, total do Crush/Goose), se houver.
    pub fn recorded_cost(&self, tool: &str, id: &str) -> Option<f64> {
        self.conn
            .query_row(
                "SELECT recorded_cost FROM sessions WHERE tool=?1 AND id=?2",
                params![tool, id],
                |r| r.get::<_, Option<f64>>(0),
            )
            .ok()
            .flatten()
    }

    pub fn refresh(&mut self, sources: &[Box<dyn SessionSource>]) -> RefreshStats {
        let started = Instant::now();
        let (metas, listed) = list_sources(sources, &self.claude_roots);
        let stored = self.stored();
        let mut plan = plan_refresh(metas, listed, stored, sources.len());
        let done = parse_work(std::mem::take(&mut plan.work), sources);
        let stats = self.commit(plan, done, started);
        self.last_refresh = Some(Instant::now());
        stats
    }

    /// O que já está no índice (impressão digital, offset, estado).
    fn stored(&self) -> HashMap<(String, String), Stored> {
        let mut stored: HashMap<(String, String), Stored> = HashMap::new();
        if let Ok(mut st) = self
            .conn
            .prepare("SELECT tool, id, fp, byte_offset, tail_state, source FROM sessions")
        {
            let rows = st.query_map([], |r| {
                Ok((
                    (r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                    Stored {
                        fp: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        offset: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                        tail_state: r.get(4)?,
                        source: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    },
                ))
            });
            if let Ok(it) = rows {
                for (k, v) in it.flatten() {
                    stored.insert(k, v);
                }
            }
        }
        stored
    }

    /// Grava o resultado de uma varredura numa transação e poda o que sumiu.
    fn commit(&mut self, plan: Plan, done: Vec<Done>, started: Instant) -> RefreshStats {
        let mut stats = plan.stats;
        if let Ok(tx) = self.conn.transaction() {
            for d in done {
                if d.tail {
                    stats.tail_reads += 1;
                } else {
                    stats.full_reads += 1;
                }
                let _ = write_done(&tx, d);
            }
            // Some o que sumiu da fonte (só de ferramentas que listaram).
            for (tool, id) in plan.gone {
                stats.removed += 1;
                let _ = delete_session(&tx, &tool, &id);
            }
            let _ = tx.commit();
        }
        stats.elapsed_ms = started.elapsed().as_millis() as u64;
        stats
    }

    /// Esquece tudo (a próxima varredura relê do zero).
    pub fn reset(&mut self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "DELETE FROM sessions; DELETE FROM usage; DELETE FROM calls; DELETE FROM msgs; DELETE FROM search_text;",
            )
            .map_err(|e| format!("{ERR_INDEX}: {e}"))?;
        self.last_refresh = None;
        Ok(())
    }

    /// Modelos distintos no recorte (para montar o `PriceBook`).
    pub fn models(&self, f: &ListFilter) -> Vec<String> {
        let (w, p) = usage_where(f, "u");
        let sql = format!(
            "SELECT DISTINCT u.model FROM usage u JOIN sessions s ON s.tool=u.tool AND s.id=u.session_id WHERE {w}"
        );
        let mut out = Vec::new();
        if let Ok(mut st) = self.conn.prepare(&sql) {
            if let Ok(rows) = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
                r.get::<_, Option<String>>(0)
            }) {
                out = rows.flatten().flatten().filter(|m| !m.is_empty()).collect();
            }
        }
        out
    }

    pub fn list(&self, f: &ListFilter, book: &PriceBook) -> ListPage {
        let (w, p) = session_where(f);
        let order = match f.sort.as_deref() {
            Some("oldest") => "s.ended_ms ASC",
            Some("tokens") => "tok DESC",
            Some("cost") => "tok DESC",
            _ => "s.ended_ms DESC",
        };
        let total: u32 = self
            .conn
            .query_row(
                &format!("SELECT COUNT(*) FROM sessions s WHERE {w}"),
                rusqlite::params_from_iter(p.iter()),
                |r| r.get(0),
            )
            .unwrap_or(0);
        let limit = f.limit.unwrap_or(200).min(5000);
        let offset = f.offset.unwrap_or(0);
        let sql = format!(
            "SELECT s.tool, s.id, s.account, s.title, s.project_path, s.git_branch, s.started_ms, s.ended_ms,
                    s.parent_session, s.source, s.mtime_ms, s.turn_count, s.models, s.recorded_cost,
                    COALESCE(SUM(u.input),0), COALESCE(SUM(u.output),0), COALESCE(SUM(u.cache_read),0),
                    COALESCE(SUM(u.cache_write),0), COALESCE(SUM(u.reasoning),0),
                    COALESCE(SUM(u.input+u.output+u.cache_read+u.cache_write+u.reasoning),0) AS tok,
                    (SELECT COUNT(*) FROM sessions c WHERE c.tool=s.tool AND c.parent_session=s.id) AS kids
             FROM sessions s LEFT JOIN usage u ON u.tool=s.tool AND u.session_id=s.id
             WHERE {w} GROUP BY s.tool, s.id ORDER BY {order} LIMIT {limit} OFFSET {offset}"
        );
        let mut out: Vec<(SessionMeta, Option<f64>)> = Vec::new();
        if let Ok(mut st) = self.conn.prepare(&sql) {
            let rows = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
                let models: Option<String> = r.get(12)?;
                Ok((
                    SessionMeta {
                        tool: r.get(0)?,
                        id: r.get(1)?,
                        account: r.get(2)?,
                        title: r.get(3)?,
                        project_path: r.get(4)?,
                        git_branch: r.get(5)?,
                        started: r.get::<_, Option<i64>>(6)?.map(util::ms_to_rfc3339),
                        ended: r.get::<_, Option<i64>>(7)?.map(util::ms_to_rfc3339),
                        parent_session: r.get(8)?,
                        source: PathBuf::from(r.get::<_, Option<String>>(9)?.unwrap_or_default()),
                        mtime_ms: r.get::<_, Option<i64>>(10)?.unwrap_or(0),
                        turn_count: r.get::<_, Option<i64>>(11)?.unwrap_or(0) as u32,
                        models: models
                            .and_then(|m| serde_json::from_str(&m).ok())
                            .unwrap_or_default(),
                        usage: TokenUsage {
                            input: r.get::<_, i64>(14)? as u64,
                            output: r.get::<_, i64>(15)? as u64,
                            cache_read: r.get::<_, i64>(16)? as u64,
                            cache_write: r.get::<_, i64>(17)? as u64,
                            reasoning: r.get::<_, i64>(18)? as u64,
                        },
                        cost_usd: 0.0,
                    },
                    r.get::<_, Option<f64>>(13)?,
                    r.get::<_, i64>(20)?,
                ))
            });
            if let Ok(it) = rows {
                for (m, recorded, kids) in it.flatten() {
                    // O `cost-state` do Claude é do processo inteiro: inclui
                    // os subagentes, que têm linhas (e custo) próprios. Numa
                    // sessão com filhos ele não é o custo desta sessão.
                    let recorded = if kids > 0 && m.tool == claude::TOOL {
                        None
                    } else {
                        recorded
                    };
                    out.push((m, recorded));
                }
            }
        }
        let keys: Vec<(String, String)> = out
            .iter()
            .map(|(m, _)| (m.tool.clone(), m.id.clone()))
            .collect();
        let costs = self.session_costs(&keys, book);
        let mut sessions: Vec<SessionMeta> = out
            .into_iter()
            .map(|(mut m, recorded)| {
                let computed = costs
                    .get(&(m.tool.clone(), m.id.clone()))
                    .copied()
                    .unwrap_or(0.0);
                m.cost_usd = recorded.unwrap_or(computed);
                m
            })
            .collect();
        if f.sort.as_deref() == Some("cost") {
            sessions.sort_by(|a, b| {
                b.cost_usd
                    .partial_cmp(&a.cost_usd)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        ListPage { total, sessions }
    }

    /// Custo por sessão: custo gravado nas linhas + tabela de preço no resto.
    pub fn session_costs(
        &self,
        keys: &[(String, String)],
        book: &PriceBook,
    ) -> HashMap<(String, String), f64> {
        let mut out: HashMap<(String, String), f64> = HashMap::new();
        if keys.is_empty() {
            return out;
        }
        let Ok(mut st) = self.conn.prepare(
            "SELECT model, COALESCE(SUM(cost),0),
                    SUM(CASE WHEN cost IS NULL THEN input ELSE 0 END),
                    SUM(CASE WHEN cost IS NULL THEN output ELSE 0 END),
                    SUM(CASE WHEN cost IS NULL THEN cache_read ELSE 0 END),
                    SUM(CASE WHEN cost IS NULL THEN cache_write ELSE 0 END),
                    SUM(CASE WHEN cost IS NULL THEN reasoning ELSE 0 END)
             FROM usage WHERE tool=?1 AND session_id=?2 GROUP BY model",
        ) else {
            return out;
        };
        for (tool, id) in keys {
            let rows = st.query_map(params![tool, id], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    r.get::<_, f64>(1)?,
                    TokenUsage {
                        input: r.get::<_, i64>(2)? as u64,
                        output: r.get::<_, i64>(3)? as u64,
                        cache_read: r.get::<_, i64>(4)? as u64,
                        cache_write: r.get::<_, i64>(5)? as u64,
                        reasoning: r.get::<_, i64>(6)? as u64,
                    },
                ))
            });
            let mut total = 0.0;
            if let Ok(it) = rows {
                for (model, recorded, u) in it.flatten() {
                    total += recorded + book.cost(&model, &u).unwrap_or(0.0);
                }
            }
            out.insert((tool.clone(), id.clone()), total);
        }
        out
    }

    pub fn get_meta(&self, tool: &str, id: &str, book: &PriceBook) -> Option<SessionMeta> {
        let f = ListFilter {
            tool: Some(tool.to_string()),
            id: Some(id.to_string()),
            include_subagents: true,
            limit: Some(1),
            ..Default::default()
        };
        self.list(&f, book).sessions.into_iter().next()
    }

    /// Textos pesquisáveis guardados (fontes sem arquivo de texto).
    pub fn search_texts(&self, keys: &[(String, String)]) -> super::search::SearchTexts {
        let mut out = super::search::SearchTexts::new();
        let Ok(mut st) = self
            .conn
            .prepare("SELECT fp, body FROM search_text WHERE tool=?1 AND id=?2")
        else {
            return out;
        };
        for (tool, id) in keys {
            if let Ok(Some((fp, body))) = st
                .query_row(params![tool, id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<Vec<u8>>>(1)?.unwrap_or_default(),
                    ))
                })
                .optional()
            {
                out.insert((tool.clone(), id.clone()), (fp, body));
            }
        }
        out
    }

    /// Grava textos pesquisáveis novos (uma transação).
    pub fn put_search_texts(&mut self, rows: &[super::search::NewText]) {
        let Ok(tx) = self.conn.transaction() else {
            return;
        };
        if let Ok(mut st) = tx.prepare_cached(
            "INSERT OR REPLACE INTO search_text(tool, id, fp, body) VALUES(?1, ?2, ?3, ?4)",
        ) {
            for ((tool, id), fp, body) in rows {
                let _ = st.execute(params![tool, id, fp, body]);
            }
        }
        let _ = tx.commit();
    }

    /// A sessão tem subagentes no índice?
    pub fn has_children(&self, tool: &str, id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE tool=?1 AND parent_session=?2)",
                params![tool, id],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(false)
    }

    /// Modelos com uso nas sessões dadas.
    pub fn models_of(&self, keys: &[(String, String)]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let Ok(mut st) = self
            .conn
            .prepare("SELECT DISTINCT model FROM usage WHERE tool=?1 AND session_id=?2")
        else {
            return out;
        };
        for (tool, id) in keys {
            if let Ok(rows) = st.query_map(params![tool, id], |r| r.get::<_, Option<String>>(0)) {
                for m in rows.flatten().flatten() {
                    if !m.is_empty() && !out.contains(&m) {
                        out.push(m);
                    }
                }
            }
        }
        out
    }

    /// Filhos (subagentes) de uma sessão.
    pub fn children(&self, tool: &str, id: &str) -> Vec<(String, Option<String>, String)> {
        let mut out = Vec::new();
        if let Ok(mut st) = self
            .conn
            .prepare("SELECT id, title, source FROM sessions WHERE tool=?1 AND parent_session=?2 ORDER BY started_ms")
        {
            if let Ok(rows) = st.query_map(params![tool, id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default()))
            }) {
                out = rows.flatten().collect();
            }
        }
        out
    }

    /// Contagem de sessões por ferramenta e última atividade.
    pub fn counts(&self) -> HashMap<String, (u32, Option<i64>)> {
        let mut out = HashMap::new();
        if let Ok(mut st) = self
            .conn
            .prepare("SELECT tool, COUNT(*), MAX(ended_ms) FROM sessions GROUP BY tool")
        {
            if let Ok(rows) = st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)? as u32,
                    r.get::<_, Option<i64>>(2)?,
                ))
            }) {
                for (t, n, last) in rows.flatten() {
                    out.insert(t, (n, last));
                }
            }
        }
        out
    }
}

fn write_done(tx: &rusqlite::Transaction, d: Done) -> rusqlite::Result<()> {
    let tool = d.meta.tool.clone();
    let id = d.meta.id.clone();
    if !d.tail {
        delete_rows(tx, &tool, &id)?;
    }
    {
        let mut ins_u = tx.prepare_cached(
            "INSERT OR IGNORE INTO usage(tool,key,session_id,ts_ms,day,hour,model,input,output,cache_read,cache_write,reasoning,cost)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        )?;
        for u in &d.rows.usage {
            let (day, hour) = util::local_day_hour(u.ts_ms);
            ins_u.execute(params![
                tool,
                u.key,
                id,
                u.ts_ms,
                day,
                hour,
                u.model,
                u.u.input as i64,
                u.u.output as i64,
                u.u.cache_read as i64,
                u.u.cache_write as i64,
                u.u.reasoning as i64,
                u.cost
            ])?;
        }
        let mut ins_c = tx.prepare_cached(
            "INSERT OR REPLACE INTO calls(tool,session_id,key,ts_ms,day,hour,name,raw,subagent,detail,status)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        )?;
        for c in &d.rows.calls {
            let (day, hour) = util::local_day_hour(c.ts_ms);
            ins_c.execute(params![
                tool, id, c.key, c.ts_ms, day, hour, c.name, c.raw, c.subagent, c.detail, c.status
            ])?;
        }
        let mut ins_m = tx.prepare_cached(
            "INSERT OR IGNORE INTO msgs(tool,session_id,key,ts_ms,day,hour,role) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        )?;
        for m in &d.rows.msgs {
            let (day, hour) = util::local_day_hour(m.ts_ms);
            ins_m.execute(params![tool, id, m.key, m.ts_ms, day, hour, m.role])?;
        }
        let mut upd = tx.prepare_cached(
            "UPDATE calls SET status=?4 WHERE tool=?1 AND session_id=?2 AND key=?3",
        )?;
        for (cid, st) in &d.orphans {
            upd.execute(params![tool, id, cid, status_str(*st)])?;
        }
    }
    let recorded = d
        .cost_states
        .as_ref()
        .filter(|m| !m.is_empty())
        .map(|m| m.values().sum::<f64>());
    let tail_state = d
        .cost_states
        .as_ref()
        .and_then(|m| serde_json::to_string(m).ok());
    let started = d.meta.started.as_deref().and_then(util::parse_ts_str);
    let ended = d.meta.ended.as_deref().and_then(util::parse_ts_str);
    if d.tail {
        let (old_models, old_start, old_end): (Option<String>, Option<i64>, Option<i64>) = tx
            .query_row(
                "SELECT models, started_ms, ended_ms FROM sessions WHERE tool=?1 AND id=?2",
                params![tool, id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .unwrap_or((None, None, None));
        let mut models: Vec<String> = old_models
            .and_then(|m| serde_json::from_str(&m).ok())
            .unwrap_or_default();
        for t in &d.rows.usage {
            if !t.model.is_empty() && !models.contains(&t.model) {
                models.push(t.model.clone());
            }
        }
        let new_start = [
            old_start,
            d.rows.msgs.iter().map(|m| m.ts_ms).filter(|t| *t > 0).min(),
        ]
        .into_iter()
        .flatten()
        .min();
        let new_end = [old_end, d.rows.msgs.iter().map(|m| m.ts_ms).max(), ended]
            .into_iter()
            .flatten()
            .max();
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM msgs WHERE tool=?1 AND session_id=?2",
            params![tool, id],
            |r| r.get(0),
        )?;
        tx.execute(
            "UPDATE sessions SET title=COALESCE(?3,title), git_branch=COALESCE(?4,git_branch), started_ms=?5, ended_ms=?6,
               mtime_ms=?7, fp=?8, byte_offset=?9, tail_state=?10, turn_count=?11, models=?12, recorded_cost=?13
             WHERE tool=?1 AND id=?2",
            params![
                tool,
                id,
                d.meta.title,
                d.meta.git_branch,
                new_start,
                new_end,
                d.meta.mtime_ms,
                d.fp,
                d.offset as i64,
                tail_state,
                count,
                serde_json::to_string(&models).unwrap_or_default(),
                recorded
            ],
        )?;
    } else {
        let recorded = recorded.or_else(|| {
            // Custo de sessão sem linhas com custo (fonte grava só o total).
            let row_cost: f64 = d.rows.usage.iter().filter_map(|u| u.cost).sum();
            (d.meta.cost_usd > 0.0 && row_cost == 0.0 && d.meta.tool != claude::TOOL)
                .then_some(d.meta.cost_usd)
        });
        let count = d.rows.msgs.len().max(d.meta.turn_count as usize) as i64;
        tx.execute(
            "INSERT OR REPLACE INTO sessions(tool,id,account,title,project_path,git_branch,started_ms,ended_ms,parent_session,
               source,mtime_ms,fp,byte_offset,tail_state,turn_count,models,recorded_cost)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![
                tool,
                id,
                d.meta.account,
                d.meta.title,
                d.meta.project_path,
                d.meta.git_branch,
                started,
                ended,
                d.meta.parent_session,
                d.meta.source.to_string_lossy(),
                d.meta.mtime_ms,
                d.fp,
                d.offset as i64,
                tail_state,
                count,
                serde_json::to_string(&d.meta.models).unwrap_or_default(),
                recorded
            ],
        )?;
    }
    Ok(())
}

fn delete_rows(tx: &Connection, tool: &str, id: &str) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM usage WHERE tool=?1 AND session_id=?2",
        params![tool, id],
    )?;
    tx.execute(
        "DELETE FROM calls WHERE tool=?1 AND session_id=?2",
        params![tool, id],
    )?;
    tx.execute(
        "DELETE FROM msgs WHERE tool=?1 AND session_id=?2",
        params![tool, id],
    )?;
    Ok(())
}

fn delete_session(tx: &Connection, tool: &str, id: &str) -> rusqlite::Result<()> {
    delete_rows(tx, tool, id)?;
    tx.execute(
        "DELETE FROM sessions WHERE tool=?1 AND id=?2",
        params![tool, id],
    )?;
    tx.execute(
        "DELETE FROM search_text WHERE tool=?1 AND id=?2",
        params![tool, id],
    )?;
    Ok(())
}

/// `WHERE` sobre `sessions s`.
fn session_where(f: &ListFilter) -> (String, Vec<String>) {
    let mut w = vec!["1=1".to_string()];
    let mut p: Vec<String> = Vec::new();
    if let Some(t) = f.tool.as_ref().filter(|s| !s.is_empty()) {
        w.push("s.tool = ?".into());
        p.push(t.clone());
    }
    if let Some(a) = f.account.as_ref().filter(|s| !s.is_empty()) {
        w.push("s.account = ?".into());
        p.push(a.clone());
    }
    if let Some(pr) = f.project.as_ref().filter(|s| !s.is_empty()) {
        w.push("s.project_path LIKE ?".into());
        p.push(format!("%{pr}%"));
    }
    if let Some(q) = f.query.as_ref().filter(|s| !s.is_empty()) {
        w.push("(s.title LIKE ? OR s.id LIKE ?)".into());
        p.push(format!("%{q}%"));
        p.push(format!("%{q}%"));
    }
    if let Some(from) = f.from.as_ref().and_then(|d| day_start_ms(d)) {
        w.push("s.ended_ms >= ?".into());
        p.push(from.to_string());
    }
    if let Some(to) = f.to.as_ref().and_then(|d| day_start_ms(d)) {
        w.push("s.started_ms < ?".into());
        p.push((to + 86_400_000).to_string());
    }
    if let Some(id) = f.id.as_ref().filter(|s| !s.is_empty()) {
        w.push("s.id = ?".into());
        p.push(id.clone());
    }
    if let Some(m) = f.model.as_ref().filter(|s| !s.is_empty()) {
        w.push("EXISTS (SELECT 1 FROM usage mu WHERE mu.tool=s.tool AND mu.session_id=s.id AND mu.model = ?)".into());
        p.push(m.clone());
    }
    if let Some(parent) = f.parent.as_ref() {
        w.push("s.parent_session = ?".into());
        p.push(parent.clone());
    } else if !f.include_subagents {
        w.push("(s.parent_session IS NULL OR s.parent_session = '')".into());
    }
    (w.join(" AND "), p)
}

/// `WHERE` sobre uma tabela derivada (`usage`/`calls`/`msgs` com alias) já
/// juntada com `sessions s`.
pub(crate) fn usage_where(f: &ListFilter, alias: &str) -> (String, Vec<String>) {
    let mut w = vec!["1=1".to_string()];
    let mut p: Vec<String> = Vec::new();
    if let Some(t) = f.tool.as_ref().filter(|s| !s.is_empty()) {
        w.push(format!("{alias}.tool = ?"));
        p.push(t.clone());
    }
    if let Some(a) = f.account.as_ref().filter(|s| !s.is_empty()) {
        w.push("s.account = ?".into());
        p.push(a.clone());
    }
    if let Some(pr) = f.project.as_ref().filter(|s| !s.is_empty()) {
        w.push("s.project_path LIKE ?".into());
        p.push(format!("%{pr}%"));
    }
    if let Some(from) = f.from.as_ref().filter(|s| !s.is_empty()) {
        w.push(format!("{alias}.day >= ?"));
        p.push(from.clone());
    }
    if let Some(to) = f.to.as_ref().filter(|s| !s.is_empty()) {
        w.push(format!("{alias}.day <= ?"));
        p.push(to.clone());
    }
    // `msgs`/`calls` não têm modelo: o filtro vira "sessão que usou o modelo".
    if let Some(m) = f.model.as_ref().filter(|s| !s.is_empty()) {
        if alias == "u" {
            w.push("u.model = ?".into());
        } else {
            w.push("EXISTS (SELECT 1 FROM usage mu WHERE mu.tool=s.tool AND mu.session_id=s.id AND mu.model = ?)".into());
        }
        p.push(m.clone());
    }
    (w.join(" AND "), p)
}

/// Início do dia local em ms.
pub fn day_start_ms(day: &str) -> Option<i64> {
    use chrono::TimeZone;
    let d = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    chrono::Local
        .from_local_datetime(&d.and_hms_opt(0, 0, 0)?)
        .earliest()
        .map(|t| t.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sessions::model::{ToolCall, Turn};

    struct Fake {
        tool: &'static str,
        sessions: Vec<Session>,
    }

    impl SessionSource for Fake {
        fn tool(&self) -> &'static str {
            self.tool
        }
        fn roots(&self) -> Vec<PathBuf> {
            Vec::new()
        }
        fn list(&self) -> Vec<SessionMeta> {
            self.sessions.iter().map(|s| s.meta.clone()).collect()
        }
        fn load(&self, id: &str) -> Option<Session> {
            self.sessions.iter().find(|s| s.meta.id == id).cloned()
        }
    }

    fn session(id: &str, mid: &str, out: u64) -> Session {
        let turn = |role, text: &str, mid: Option<&str>, usage: TokenUsage| Turn {
            role,
            ts: "2026-09-20T12:00:00.000Z".into(),
            text: text.into(),
            tool_calls: if role == Role::Assistant {
                vec![ToolCall {
                    id: format!("call-{id}"),
                    name_canonical: "Agent".into(),
                    name_raw: "task".into(),
                    input: serde_json::json!({"description": "procurar"}),
                    result: None,
                    status: ToolStatus::Ok,
                    ms: None,
                    subagent: Some("explore".into()),
                }]
            } else {
                Vec::new()
            },
            usage,
            cost_usd: None,
            model: Some("m".into()),
            message_id: mid.map(|s| s.to_string()),
        };
        Session {
            meta: SessionMeta {
                tool: "fake".into(),
                account: None,
                id: id.into(),
                title: Some(format!("sessão {id}")),
                project_path: Some("/w/p".into()),
                git_branch: None,
                started: Some("2026-09-20T12:00:00.000Z".into()),
                ended: Some("2026-09-20T12:00:00.000Z".into()),
                models: vec!["m".into()],
                parent_session: None,
                turn_count: 2,
                usage: TokenUsage::default(),
                cost_usd: 0.0,
                source: PathBuf::from(format!("/nao/existe/{id}")),
                mtime_ms: 1,
            },
            turns: vec![
                turn(
                    Role::User,
                    "/review agora",
                    Some("u"),
                    TokenUsage::default(),
                ),
                turn(
                    Role::Assistant,
                    "ok",
                    Some(mid),
                    TokenUsage {
                        output: out,
                        ..Default::default()
                    },
                ),
            ],
        }
    }

    #[test]
    fn refresh_indexes_then_skips_unchanged_and_prunes() {
        let mut idx = SessionIndex::open_in_memory().unwrap();
        let src: Vec<Box<dyn SessionSource>> = vec![Box::new(Fake {
            tool: "fake",
            sessions: vec![session("a", "msg_1", 10), session("b", "msg_1", 7)],
        })];
        let st = idx.refresh(&src);
        assert_eq!(st.full_reads, 2);
        let page = idx.list(&ListFilter::default(), &PriceBook::empty());
        assert_eq!(page.total, 2);
        // Fora do Claude a chave de uso é por sessão: os dois contam.
        let tok: u64 = page.sessions.iter().map(|m| m.usage.total()).sum();
        assert_eq!(tok, 17);
        let st2 = idx.refresh(&src);
        assert_eq!(st2.full_reads + st2.tail_reads, 0, "nothing changed");
        let calls: i64 = idx
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM calls WHERE name='SlashCommand'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(calls, 2);
        let src2: Vec<Box<dyn SessionSource>> = vec![Box::new(Fake {
            tool: "fake",
            sessions: vec![session("a", "msg_1", 10)],
        })];
        let st3 = idx.refresh(&src2);
        assert_eq!(st3.removed, 1);
    }

    /// Aceite: tokens por dia do Claude desta máquina, soma ingênua (toda
    /// linha `assistant` com `usage`) × índice com dedupe. Rode com
    /// `cargo test -p omniget-core real_claude_dedupe -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_claude_dedupe_by_day() {
        let src = claude::ClaudeSource::new();
        let files = src.files();
        let mut naive: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        let mut per_file: BTreeMap<String, u64> = BTreeMap::new();
        for (path, ..) in &files {
            let mut seen_in_file: HashSet<String> = HashSet::new();
            let _ = util::for_each_line(path, 0, |l| {
                let Ok(v) = serde_json::from_slice::<serde_json::Value>(l) else {
                    return;
                };
                if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                    return;
                }
                let Some(u) = v.get("message").and_then(|m| m.get("usage")) else {
                    return;
                };
                let tok = util::num(u, "input_tokens")
                    + util::num(u, "output_tokens")
                    + util::num(u, "cache_read_input_tokens")
                    + util::num(u, "cache_creation_input_tokens");
                let Some(ms) = util::ts_ms_of(v.get("timestamp")) else {
                    return;
                };
                let day = util::local_day_hour(ms).0;
                let e = naive.entry(day.clone()).or_default();
                e.0 += tok;
                e.1 += 1;
                let key = format!(
                    "{}:{}",
                    v["message"]["id"].as_str().unwrap_or(""),
                    v["requestId"].as_str().unwrap_or("")
                );
                if seen_in_file.insert(key) {
                    *per_file.entry(day).or_default() += tok;
                }
            });
        }
        let dir =
            std::env::temp_dir().join(format!("omniget-sessions-accept-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut idx = SessionIndex::open(&dir.join("sessions.db")).unwrap();
        let sources: Vec<Box<dyn SessionSource>> = vec![Box::new(claude::ClaudeSource::new())];
        let t0 = Instant::now();
        let st = idx.refresh(&sources);
        let cold = t0.elapsed();
        let t1 = Instant::now();
        let st2 = idx.refresh(&sources);
        let warm = t1.elapsed();
        println!(
            "files={} cold={:?} ({:?}) warm={:?} ({:?})",
            files.len(),
            cold,
            st,
            warm,
            st2
        );
        let mut rows: Vec<(String, u64, u64)> = Vec::new();
        {
            let mut q = idx
                .conn()
                .prepare("SELECT day, SUM(input+output+cache_read+cache_write+reasoning), COUNT(*) FROM usage WHERE tool='claude' GROUP BY day")
                .unwrap();
            let it = q
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)? as u64,
                        r.get::<_, i64>(2)? as u64,
                    ))
                })
                .unwrap();
            rows.extend(it.flatten());
        }
        println!("day        | naive lines | naive tokens    | per-file dedupe | global dedupe (index) | index rows | inflation");
        for (day, tok, n) in rows.iter().rev().take(14) {
            let (nv, nl) = naive.get(day).copied().unwrap_or((0, 0));
            let pf = per_file.get(day).copied().unwrap_or(0);
            println!(
                "{day} | {nl:>11} | {nv:>15} | {pf:>15} | {tok:>21} | {n:>10} | {:.2}x",
                if *tok > 0 {
                    nv as f64 / *tok as f64
                } else {
                    0.0
                }
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn claude_file_that_grows_is_read_from_the_offset() {
        let base = std::env::temp_dir().join(format!("omniget-index-tail-{}", std::process::id()));
        let proj = base.join("projects").join("-w-p");
        std::fs::create_dir_all(&proj).unwrap();
        let f = proj.join("s1.jsonl");
        let a = |id: &str, req: &str, out: u64, ts: &str| {
            format!(
                r#"{{"type":"assistant","requestId":"{req}","timestamp":"{ts}","cwd":"/w/p","message":{{"id":"{id}","model":"m","role":"assistant","content":[{{"type":"text","text":"x"}}],"usage":{{"input_tokens":1,"output_tokens":{out},"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
            )
        };
        let u = r#"{"type":"user","uuid":"u1","timestamp":"2026-09-20T10:00:00.000Z","cwd":"/w/p","message":{"role":"user","content":"oi"}}"#;
        std::fs::write(
            &f,
            format!(
                "{u}\n{}\n",
                a("msg_1", "r1", 10, "2026-09-20T10:00:01.000Z")
            ),
        )
        .unwrap();
        let mut idx = SessionIndex::open_in_memory().unwrap();
        idx.set_claude_roots(vec![claude::Root {
            account: "t".into(),
            config_dir: Some(base.clone()),
            projects: base.join("projects"),
        }]);
        let src: Vec<Box<dyn SessionSource>> =
            vec![Box::new(claude::ClaudeSource::with_roots(Vec::new()))];
        assert_eq!(idx.refresh(&src).full_reads, 1);
        // Mesma mensagem repetida (linha de outro bloco) + mensagem nova.
        let mut fh = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
        use std::io::Write as _;
        writeln!(fh, "{}", a("msg_1", "r1", 10, "2026-09-20T10:00:01.500Z")).unwrap();
        writeln!(fh, "{}", a("msg_2", "r2", 5, "2026-09-20T10:00:02.000Z")).unwrap();
        drop(fh);
        let st = idx.refresh(&src);
        assert_eq!((st.full_reads, st.tail_reads), (0, 1));
        let page = idx.list(&ListFilter::default(), &PriceBook::empty());
        assert_eq!(page.sessions[0].usage.output, 15);
        assert_eq!(page.sessions[0].turn_count, 3);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn slash_commands_are_found() {
        assert_eq!(
            slash_commands("<command-name>/model</command-name>"),
            vec!["/model"]
        );
        assert_eq!(slash_commands("/review o PR"), vec!["/review"]);
        assert!(slash_commands("/Users/tonho/x").is_empty());
        assert!(slash_commands("olá").is_empty());
    }
}
