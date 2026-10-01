//! Operações de sessão prontas para os comandos Tauri (`sessions_*`).
//!
//! Tudo que lê disco roda em `spawn_blocking`; a tabela de preços é montada
//! de forma assíncrona só com os modelos do recorte. O índice é varrido sob
//! demanda, no máximo a cada `FRESH_SECS`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::analysis::{
    self, AgentsReport, Child, Heatmap, Retro, SessionAnalysis, TeamGraph, UsageReport,
};
use super::cost::PriceBook;
use super::export::{self, ExportOut};
use super::import::{self, ImportOutcome};
use super::index::{self, ListFilter, ListPage, RefreshStats};
use super::model::{Role, Session, SessionMeta, SessionSource, ToolStatus, Turn};
use super::parsers::{self, group_a::claude, group_a::codex};
use super::search::{self, Hit, SearchQuery, SearchResult};
use super::util;

pub const ERR_NOT_FOUND: &str = "ERR_SESSION_NOT_FOUND";
pub const ERR_TASK: &str = "ERR_SESSIONS_TASK";
const FRESH: Duration = Duration::from_secs(10);
const SOURCES_TTL: Duration = Duration::from_secs(300);

type Sources = Arc<Vec<Box<dyn SessionSource>>>;
static SOURCES: Mutex<Option<(Instant, Sources)>> = Mutex::new(None);

/// Fontes vivas (os caches de lista sobrevivem entre chamadas); refeitas a
/// cada 5 min para pegar contas novas.
pub fn sources() -> Sources {
    if let Ok(mut g) = SOURCES.lock() {
        if let Some((at, s)) = g.as_ref() {
            if at.elapsed() < SOURCES_TTL {
                return s.clone();
            }
        }
        let s: Sources = Arc::new(parsers::all_sources());
        *g = Some((Instant::now(), s.clone()));
        return s;
    }
    Arc::new(parsers::all_sources())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("{ERR_TASK}: {e}"))?
}

/// Índice em dia (no máximo `FRESH` de idade). Uma varredura por vez: quem
/// chega durante uma espera por ela em vez de varrer de novo.
fn fresh() -> Result<(), String> {
    index::ensure_fresh_global(Some(FRESH), &sources()).map(|_| ())
}

/// Carrega a sessão completa de um meta do índice, pelo caminho mais curto.
pub fn load_meta(meta: &SessionMeta) -> Option<Session> {
    let account = meta.account.clone().unwrap_or_else(|| "default".into());
    match meta.tool.as_str() {
        "claude" if meta.source.is_file() => {
            let mut s = claude::load_file(&meta.source, &account, &meta.id);
            s.meta.account = meta.account.clone();
            Some(s)
        }
        "codex" if meta.source.is_file() => {
            let titles = codex_titles(&meta.source);
            let mut s = codex::load_file(&meta.source, &account, &titles);
            s.meta.id = meta.id.clone();
            Some(s)
        }
        _ => {
            let src = sources();
            let found = src
                .iter()
                .filter(|s| s.tool() == meta.tool)
                .find_map(|s| s.load(&meta.id));
            found
        }
    }
}

/// `session_index.jsonl` do `CODEX_HOME` dono do arquivo.
fn codex_titles(file: &Path) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    let home = file
        .ancestors()
        .find(|a| {
            a.file_name()
                .map(|n| n == "sessions" || n == "archived_sessions")
                .unwrap_or(false)
        })
        .and_then(|a| a.parent());
    if let Some(h) = home {
        let _ = util::for_each_line(&h.join("session_index.jsonl"), 0, |l| {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(l) {
                if let (Some(id), Some(n)) =
                    (util::str_of(&v, "id"), util::str_of(&v, "thread_name"))
                {
                    m.insert(id.to_string(), n.to_string());
                }
            }
        });
    }
    m
}

fn find_meta(tool: &str, id: &str) -> Result<SessionMeta, String> {
    let (t, i) = (tool.to_string(), id.to_string());
    fresh()?;
    let found = index::with_reader(|idx| idx.get_meta(&t, &i, &PriceBook::empty()))?;
    if let Some(m) = found {
        return Ok(m);
    }
    // Fora do índice (ferramenta nova, sessão recém-criada): pede à fonte.
    // Mais de uma fonte pode ter o mesmo id de ferramenta (`kilo`).
    let src = sources();
    let s = src
        .iter()
        .filter(|s| s.tool() == tool)
        .find_map(|s| s.load(id))
        .ok_or_else(|| format!("{ERR_NOT_FOUND}: {tool}/{id}"))?;
    Ok(s.meta)
}

fn load(tool: &str, id: &str) -> Result<(SessionMeta, Session), String> {
    let meta = find_meta(tool, id)?;
    let s = load_meta(&meta).ok_or_else(|| format!("{ERR_NOT_FOUND}: {tool}/{id}"))?;
    Ok((meta, s))
}

async fn book_for(f: &ListFilter) -> Result<PriceBook, String> {
    let f2 = f.clone();
    let models = blocking(move || {
        fresh()?;
        index::with_reader(|idx| idx.models(&f2))
    })
    .await?;
    Ok(PriceBook::load(models).await)
}

// ---------------------------------------------------------------------------

pub async fn refresh(full: bool) -> Result<RefreshStats, String> {
    blocking(move || {
        let src = sources();
        if full {
            index::reset_and_refresh(&src)
        } else {
            Ok(index::ensure_fresh_global(None, &src)?.unwrap_or_default())
        }
    })
    .await
}

pub async fn list(filter: ListFilter) -> Result<ListPage, String> {
    let mut all = filter.clone();
    all.from = None;
    all.to = None;
    let book = book_for(&all).await?;
    blocking(move || index::with_reader(|idx| idx.list(&filter, &book))).await
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionPage {
    pub meta: SessionMeta,
    pub turns: Vec<Turn>,
    /// Índice (na sessão inteira) da primeira `Turn` desta página.
    pub first_index: usize,
    pub page: u32,
    pub pages: u32,
    pub total: usize,
    pub has_older: bool,
    pub resume_command: Option<String>,
    /// Como um driver da Central continua esta sessão (ver [`resume_target`]).
    pub resume_target: Option<ResumeTarget>,
}

/// Paginação reversa: página 0 = as `page_size` mensagens mais recentes.
pub async fn get(
    tool: String,
    id: String,
    page: Option<u32>,
    page_size: Option<u32>,
) -> Result<SessionPage, String> {
    let f = ListFilter {
        tool: Some(tool.clone()),
        id: Some(id.clone()),
        include_subagents: true,
        ..ListFilter::default()
    };
    let book = book_for(&f).await?;
    blocking(move || {
        let (_, s) = load(&tool, &id)?;
        let meta = index::with_reader(|idx| idx.get_meta(&tool, &id, &book))?
            .unwrap_or_else(|| s.meta.clone());
        let resume = resume_for(&meta);
        let target = resume_target(&meta);
        let total = s.turns.len();
        let (turns, first, page_n, pages) = match page {
            None => (s.turns, 0, 0, 1),
            Some(p) => {
                let size = page_size.unwrap_or(50).clamp(1, 1000) as usize;
                let pages = total.div_ceil(size).max(1) as u32;
                let p = p.min(pages - 1);
                let end = total.saturating_sub(p as usize * size);
                let start = end.saturating_sub(size);
                (s.turns[start..end].to_vec(), start, p, pages)
            }
        };
        Ok(SessionPage {
            meta,
            has_older: first > 0,
            turns,
            first_index: first,
            page: page_n,
            pages,
            total,
            resume_command: resume,
            resume_target: target,
        })
    })
    .await
}

pub async fn search(q: SearchQuery) -> Result<SearchResult, String> {
    blocking(move || {
        let f = ListFilter {
            tool: q.tool.clone(),
            account: q.account.clone(),
            project: q.project.clone(),
            from: q.from.clone(),
            to: q.to.clone(),
            include_subagents: q.include_subagents,
            limit: Some(100_000),
            ..Default::default()
        };
        fresh()?;
        let (candidates, texts) = index::with_reader(|idx| {
            let c = idx.list(&f, &PriceBook::empty()).sessions;
            let keys: Vec<(String, String)> = c
                .iter()
                .filter(|m| search::needs_text(m))
                .map(|m| (m.tool.clone(), m.id.clone()))
                .collect();
            let t = idx.search_texts(&keys);
            (c, t)
        })?;
        let (res, new_texts) = search::search_global(candidates, &q, &load_meta, &texts);
        if !new_texts.is_empty() {
            let _ = index::with_index(|idx| idx.put_search_texts(&new_texts));
        }
        Ok(res)
    })
    .await
}

pub async fn search_in(tool: String, id: String, query: String) -> Result<Vec<Hit>, String> {
    blocking(move || {
        let (_, s) = load(&tool, &id)?;
        Ok(search::search_in(&s, query.trim(), true, 10_000))
    })
    .await
}

pub async fn session_analysis(tool: String, id: String) -> Result<SessionAnalysis, String> {
    let (t, i) = (tool.clone(), id.clone());
    let (s, recorded, kids) = blocking(move || {
        let (_, s) = load(&t, &i)?;
        let (recorded, kids) = index::with_reader(|idx| {
            let kids: Vec<(String, String)> = if t == claude::TOOL {
                idx.children(&t, &i)
                    .into_iter()
                    .map(|(cid, ..)| (t.clone(), cid))
                    .collect()
            } else {
                Vec::new()
            };
            (idx.recorded_cost(&t, &i), kids)
        })?;
        Ok((s, recorded, kids))
    })
    .await?;
    let mut models: Vec<String> = s.turns.iter().filter_map(|t| t.model.clone()).collect();
    if kids.is_empty() {
        let book = PriceBook::load(models).await;
        return blocking(move || Ok(analysis::analyze(&s, &book, recorded))).await;
    }
    // Claude com subagentes: o `cost-state` é do processo inteiro (sessão +
    // subagentes). O custo desta sessão sai da tabela; o gravado vai à parte,
    // ao lado da soma calculada da árvore, para comparar.
    let k2 = kids.clone();
    models.extend(blocking(move || index::with_reader(|idx| idx.models_of(&k2))).await?);
    let book = PriceBook::load(models).await;
    blocking(move || {
        let mut a = analysis::analyze(&s, &book, None);
        let kids_cost: f64 =
            index::with_reader(|idx| idx.session_costs(&kids, &book).values().sum())?;
        a.cost.tree_recorded_usd = recorded;
        a.cost.tree_computed_usd = Some(a.cost.computed_usd + kids_cost);
        Ok(a)
    })
    .await
}

pub async fn usage(range: ListFilter, group_by: String) -> Result<UsageReport, String> {
    let book = book_for(&range).await?;
    blocking(move || index::with_reader(|idx| analysis::usage(idx, &range, &group_by, &book))).await
}

pub async fn heatmap(mut range: ListFilter, tool: Option<String>) -> Result<Heatmap, String> {
    if tool.is_some() {
        range.tool = tool;
    }
    blocking(move || {
        fresh()?;
        index::with_reader(|idx| analysis::heatmap(idx, &range))
    })
    .await
}

pub async fn agents(range: ListFilter) -> Result<AgentsReport, String> {
    blocking(move || {
        fresh()?;
        index::with_reader(|idx| analysis::agents(idx, &range))
    })
    .await
}

pub async fn team(tool: String, id: String) -> Result<TeamGraph, String> {
    blocking(move || {
        let (_, lead) = load(&tool, &id)?;
        let kids = index::with_reader(|idx| idx.children(&tool, &id))?;
        let mut children = Vec::new();
        for (cid, _title, source) in kids {
            let meta = match find_meta(&tool, &cid) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let Some(session) = load_meta(&meta) else {
                continue;
            };
            let sm = (tool == "claude")
                .then(|| claude::subagent_meta(Path::new(&source)))
                .flatten();
            children.push(Child {
                session,
                spawn_call_id: sm.as_ref().and_then(|m| m.tool_use_id.clone()),
                agent_type: sm.as_ref().and_then(|m| m.agent_type.clone()),
                description: sm.as_ref().and_then(|m| m.description.clone()),
            });
        }
        Ok(analysis::team(&lead, children))
    })
    .await
}

pub async fn retro(
    from: Option<String>,
    to: Option<String>,
    tool: Option<String>,
) -> Result<Retro, String> {
    let range = ListFilter {
        from,
        to,
        tool,
        ..Default::default()
    };
    let book = book_for(&range).await?;
    blocking(move || index::with_reader(|idx| analysis::retro(idx, &range, &book))).await
}

/// `format`: `json` | `markdown` | `context` (Markdown para continuar em
/// `target`).
pub async fn export(
    tool: String,
    id: String,
    format: String,
    target: Option<String>,
) -> Result<ExportOut, String> {
    blocking(move || {
        let (_, s) = load(&tool, &id)?;
        Ok(match format.as_str() {
            "json" => export::to_json(&s),
            "context" => export::to_context(&s, target.as_deref(), 120_000),
            _ => export::to_markdown(&s),
        })
    })
    .await
}

pub async fn import(path: String, target: Option<String>) -> Result<ImportOutcome, String> {
    blocking(move || {
        let out = import::import(&PathBuf::from(&path), target.as_deref())?;
        index::mark_stale_global();
        Ok(out)
    })
    .await
}

fn resume_for(meta: &SessionMeta) -> Option<String> {
    let src = sources();
    src.iter()
        .find(|s| s.tool() == meta.tool)?
        .resume_command(meta)
}

/// O que um driver da Central precisa para continuar uma sessão externa:
/// `driver`/`instance_id` padrão e o `resume_cursor` no formato que aquele
/// driver lê (`SessionStart.resume_cursor`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResumeTarget {
    /// `claude` | `codex` | `opencode` | `acp`.
    pub driver: String,
    /// Instância padrão (`claude`, `codex`, `opencode`, `acp-<agente>`).
    pub instance_id: String,
    pub cursor: serde_json::Value,
    /// Id nativo que a ferramenta aceita no `--resume`/`session/load`.
    pub session_id: String,
    pub cwd: Option<String>,
    /// Conta/perfil de onde a sessão saiu (`CLAUDE_CONFIG_DIR`/`CODEX_HOME`
    /// de outra conta só retoma numa instância com o mesmo diretório).
    pub account: Option<String>,
    /// Diretório de configuração da conta, quando não é o padrão.
    pub config_dir: Option<String>,
}

/// Cursor de retomada por ferramenta. `None` quando a sessão não é
/// retomável por um driver (subagente, IDE sem CLI, id sintético).
pub fn resume_target(meta: &SessionMeta) -> Option<ResumeTarget> {
    if meta
        .parent_session
        .as_deref()
        .is_some_and(|p| !p.is_empty())
    {
        return None;
    }
    let id = meta.id.clone();
    let cwd = meta.project_path.clone().filter(|p| !p.is_empty());
    let account = meta.account.clone();
    let acp = |agent: &str| {
        Some(ResumeTarget {
            driver: "acp".into(),
            instance_id: format!("acp-{agent}"),
            cursor: serde_json::json!({ "sessionId": id, "agent": agent, "cwd": cwd }),
            session_id: id.clone(),
            cwd: cwd.clone(),
            account: account.clone(),
            config_dir: None,
        })
    };
    let acc = account.as_deref().unwrap_or("default");
    match meta.tool.as_str() {
        "claude" => {
            uuid::Uuid::parse_str(&id).ok()?;
            let cfg = claude::ClaudeSource::new()
                .config_dir_of(acc)
                .map(|p| p.to_string_lossy().into_owned());
            Some(ResumeTarget {
                driver: "claude".into(),
                instance_id: "claude".into(),
                cursor: serde_json::json!({ "sessionId": id }),
                session_id: id.clone(),
                cwd,
                account,
                config_dir: cfg,
            })
        }
        "codex" => {
            let cfg = codex::CodexSource::new()
                .home_of(acc)
                .filter(|r| !r.is_default)
                .map(|r| r.home.to_string_lossy().into_owned());
            Some(ResumeTarget {
                driver: "codex".into(),
                instance_id: "codex".into(),
                cursor: serde_json::json!({ "threadId": id }),
                session_id: id.clone(),
                cwd,
                account,
                config_dir: cfg,
            })
        }
        "opencode" => Some(ResumeTarget {
            driver: "opencode".into(),
            instance_id: "opencode".into(),
            cursor: serde_json::json!({ "sessionId": id, "cwd": cwd }),
            session_id: id.clone(),
            cwd: cwd.clone(),
            account: account.clone(),
            config_dir: None,
        }),
        // Kilo atual (kilo.db) fala ACP; o legado da extensão não tem CLI.
        "kilo" if meta.source.extension().and_then(|e| e.to_str()) == Some("db") => acp("kilo"),
        "gemini" | "qwen" | "goose" | "kimi" => acp(meta.tool.as_str()),
        // Só as sessões do CLI dessas ferramentas.
        "cursor" | "copilot" | "cline" if acc == "cli" => acp(meta.tool.as_str()),
        "grok" if acc == "grok-build" => acp("grok"),
        _ => None,
    }
}

pub async fn resume_command(tool: String, id: String) -> Result<Option<String>, String> {
    blocking(move || {
        let meta = find_meta(&tool, &id)?;
        Ok(resume_for(&meta))
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
pub struct RootInfo {
    pub path: PathBuf,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub tool: String,
    pub roots: Vec<RootInfo>,
    /// Alguma raiz existe nesta máquina.
    pub found: bool,
    /// Sem dados nesta máquina: o parser foi escrito pela documentação.
    pub beta: bool,
    pub sessions: u32,
    pub last_activity: Option<String>,
}

pub async fn sources_info() -> Result<Vec<SourceInfo>, String> {
    blocking(move || {
        fresh()?;
        let counts = index::with_reader(|idx| idx.counts())?;
        let src = sources();
        let mut out = Vec::new();
        for s in src.iter() {
            let roots: Vec<RootInfo> = s
                .roots()
                .into_iter()
                .map(|p| RootInfo {
                    exists: p.exists(),
                    path: p,
                })
                .collect();
            let found = roots.iter().any(|r| r.exists);
            let (n, last) = counts.get(s.tool()).copied().unwrap_or((0, None));
            if out.iter().any(|o: &SourceInfo| o.tool == s.tool()) {
                continue;
            }
            out.push(SourceInfo {
                tool: s.tool().to_string(),
                found,
                beta: !found || n == 0,
                roots,
                sessions: n,
                last_activity: last.map(util::ms_to_rfc3339),
            });
        }
        Ok(out)
    })
    .await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveSession {
    pub meta: SessionMeta,
    /// `working` (a vez é do agente) | `waiting` (a vez é sua) | `idle`.
    pub state: String,
    pub idle_secs: u64,
    pub pending_tool: bool,
}

/// Sessões com atividade nos últimos `window_secs` (pelo mtime da fonte).
pub async fn active(window_secs: u64) -> Result<Vec<ActiveSession>, String> {
    blocking(move || {
        let now = util::now_ms();
        let since = now - (window_secs.max(1) as i64) * 1000;
        index::ensure_fresh_global(Some(Duration::from_secs(3)), &sources())?;
        let metas = index::with_reader(|idx| {
            idx.list(
                &ListFilter {
                    include_subagents: true,
                    limit: Some(500),
                    ..Default::default()
                },
                &PriceBook::empty(),
            )
            .sessions
        })?;
        let mut out = Vec::new();
        for mut m in metas {
            // mtime atual do arquivo (o índice pode ter alguns segundos).
            if m.source.is_file() && per_session_file(&m.source) {
                m.mtime_ms = m.mtime_ms.max(util::file_mtime_ms(&m.source));
            }
            if m.mtime_ms < since {
                continue;
            }
            let (last_role, pending) = if m.tool == "claude" && m.source.is_file() {
                claude::tail_state(&m.source)
            } else {
                match load_meta(&m) {
                    Some(s) => last_state(&s.turns),
                    None => (None, false),
                }
            };
            let idle = ((now - m.mtime_ms).max(0) / 1000) as u64;
            let state = match (last_role, pending) {
                (_, true) if idle < 600 => "working",
                (Some(Role::User), _) if idle < 600 => "working",
                (Some(Role::Assistant), _) => "waiting",
                _ => "idle",
            };
            out.push(ActiveSession {
                meta: m,
                state: state.into(),
                idle_secs: idle,
                pending_tool: pending,
            });
        }
        out.sort_by_key(|a| a.idle_secs);
        Ok(out)
    })
    .await
}

fn per_session_file(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()),
        Some("jsonl") | Some("json") | Some("zst")
    )
}

fn last_state(turns: &[Turn]) -> (Option<Role>, bool) {
    let last = turns.iter().rev().find(|t| t.role != Role::System);
    (
        last.map(|t| t.role),
        last.map(|t| t.tool_calls.iter().any(|c| c.status == ToolStatus::Pending))
            .unwrap_or(false),
    )
}
