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
    let mut f = ListFilter::default();
    f.tool = Some(tool.clone());
    f.id = Some(id.clone());
    f.include_subagents = true;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_target_per_tool() {
        let meta = |tool: &str, id: &str, account: Option<&str>, source: &str| SessionMeta {
            tool: tool.into(),
            account: account.map(str::to_string),
            id: id.into(),
            title: None,
            project_path: Some("/w/p".into()),
            git_branch: None,
            started: None,
            ended: None,
            models: Vec::new(),
            parent_session: None,
            turn_count: 0,
            usage: Default::default(),
            cost_usd: 0.0,
            source: PathBuf::from(source),
            mtime_ms: 0,
        };
        let uuid = "58a9b20e-37c6-4fa9-b9f1-516e534f47c4";
        let c = resume_target(&meta("claude", uuid, None, "/x.jsonl")).unwrap();
        assert_eq!(
            (c.driver.as_str(), c.cursor["sessionId"].as_str()),
            ("claude", Some(uuid))
        );
        assert!(resume_target(&meta("claude", "agent-a1", None, "/x.jsonl")).is_none());
        let x = resume_target(&meta("codex", uuid, None, "/r.jsonl")).unwrap();
        assert_eq!(x.cursor["threadId"].as_str(), Some(uuid));
        let g = resume_target(&meta("gemini", uuid, None, "/g.json")).unwrap();
        assert_eq!(
            (g.driver.as_str(), g.instance_id.as_str()),
            ("acp", "acp-gemini")
        );
        assert_eq!(g.cursor["cwd"].as_str(), Some("/w/p"));
        let o = resume_target(&meta("opencode", "ses_1", None, "/o.db")).unwrap();
        assert_eq!(
            (o.driver.as_str(), o.cursor["sessionId"].as_str()),
            ("opencode", Some("ses_1"))
        );
        assert!(resume_target(&meta("cursor", "c1", Some("Cursor"), "/s.vscdb")).is_none());
        assert_eq!(
            resume_target(&meta("cursor", "c1", Some("cli"), "/s.db"))
                .unwrap()
                .instance_id,
            "acp-cursor"
        );
        assert!(resume_target(&meta("kilo", "k", Some("Code"), "/t.json")).is_none());
        let mut sub = meta("claude", uuid, None, "/x.jsonl");
        sub.parent_session = Some("p".into());
        assert!(resume_target(&sub).is_none());
    }

    /// `cargo test --release -p omniget-core --lib real_load_costs -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_load_costs() {
        let dir =
            std::env::temp_dir().join(format!("omniget-sessions-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("OMNIGET_DATA_DIR", &dir);
        fresh().unwrap();
        let metas = index::with_reader(|idx| {
            idx.list(
                &ListFilter {
                    limit: Some(100_000),
                    ..Default::default()
                },
                &PriceBook::empty(),
            )
            .sessions
        })
        .unwrap();
        let mut per: std::collections::BTreeMap<String, (u32, u32, Duration, Duration)> =
            Default::default();
        for m in &metas {
            let raw = matches!(
                m.source.extension().and_then(|e| e.to_str()),
                Some("jsonl") | Some("json")
            ) && m.source.is_file();
            let t = Instant::now();
            let s = load_meta(m);
            let d = t.elapsed();
            let t2 = Instant::now();
            if let Some(s) = &s {
                let _ = search::search_in(s, "zzqx", true, 5);
            }
            let e = per.entry(format!("{} raw={raw}", m.tool)).or_default();
            e.0 += 1;
            if s.is_none() {
                e.1 += 1;
            }
            e.2 += d;
            e.3 += t2.elapsed();
        }
        for (k, v) in per {
            println!(
                "{k:>22} n={} none={} load={:?} search_in={:?}",
                v.0, v.1, v.2, v.3
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Desempenho contra os logs reais (só leitura, índice temporário).
    /// `cargo test --release -p omniget-core --lib real_perf -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_perf() {
        let dir =
            std::env::temp_dir().join(format!("omniget-sessions-perf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("OMNIGET_DATA_DIR", &dir);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(8)
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            // 1. Frio e concorrente: tudo chega junto com o índice vazio.
            let t0 = Instant::now();
            let mut hs: Vec<tokio::task::JoinHandle<(&'static str, Duration, Option<String>)>> = Vec::new();
            macro_rules! go {
                ($hs:ident, $name:expr, $fut:expr) => {{
                    $hs.push(tokio::spawn(async move {
                        let t = Instant::now();
                        let r = $fut.await.map(|_| ());
                        ($name, t.elapsed(), r.err())
                    }));
                }};
            }
            go!(hs, "list", list(ListFilter::default()));
            go!(hs, "usage", usage(ListFilter::default(), "day".into()));
            go!(hs, "heatmap", heatmap(ListFilter::default(), None));
            go!(hs, "sources", sources_info());
            go!(hs, "active", active(600));
            go!(hs, "search", search(SearchQuery { text: "dedupe".into(), limit: Some(200), ..Default::default() }));
            go!(hs, "retro", retro(None, None, None));
            for h in hs {
                let (n, d, e) = h.await.unwrap();
                println!("cold-concurrent {n:>8} {:>8.2?} err={e:?}", d);
            }
            println!("cold-concurrent wall {:?}", t0.elapsed());
            // 2. Busca global quente.
            for text in ["dedupe", "zzqx-nada-aqui-9f3", "função", "cargo test", "a"] {
                let t = Instant::now();
                let r = search(SearchQuery { text: text.into(), limit: Some(200), ..Default::default() }).await.unwrap();
                println!("search {text:?} hits={} scanned={} matched={} truncated={} cancelled={} in {:?}", r.hits.len(), r.sessions_scanned, r.sessions_matched, r.truncated, r.cancelled, t.elapsed());
            }
            let t = Instant::now();
            let st = refresh(false).await.unwrap();
            println!("warm refresh {:?} ({:?})", t.elapsed(), st);
            // Digitando: cada tecla uma busca (cache de sessões + negativos).
            for text in ["wor", "work", "workt", "worktree"] {
                let t = Instant::now();
                let r = search(SearchQuery { text: text.into(), limit: Some(200), ..Default::default() }).await.unwrap();
                println!("typing {text:?} hits={} scanned={} truncated={} in {:?}", r.hits.len(), r.sessions_scanned, r.truncated, t.elapsed());
            }
            // Uma busca nova cancela a anterior.
            let a = tokio::spawn(search(SearchQuery { text: "zzqx-cancel-1".into(), ..Default::default() }));
            tokio::time::sleep(Duration::from_millis(50)).await;
            let b = search(SearchQuery { text: "zzqx-cancel-2".into(), ..Default::default() }).await.unwrap();
            let a = a.await.unwrap().unwrap();
            println!("supersede first.cancelled={} second.cancelled={} second.scanned={}", a.cancelled, b.cancelled, b.sessions_scanned);
            // 3. Quente e concorrente.
            let t0 = Instant::now();
            let mut hs: Vec<tokio::task::JoinHandle<(&'static str, Duration, Option<String>)>> = Vec::new();
            go!(hs, "list", list(ListFilter::default()));
            go!(hs, "usage", usage(ListFilter::default(), "day".into()));
            go!(hs, "search", search(SearchQuery { text: "worktree".into(), keep_previous: true, ..Default::default() }));
            go!(hs, "search2", search(SearchQuery { text: "sqlite".into(), keep_previous: true, ..Default::default() }));
            go!(hs, "active", active(600));
            go!(hs, "heatmap", heatmap(ListFilter::default(), None));
            for h in hs {
                let (n, d, e) = h.await.unwrap();
                println!("warm-concurrent {n:>8} {:>8.2?} err={e:?}", d);
            }
            println!("warm-concurrent wall {:?}", t0.elapsed());
            // 4. Custo da sessão suspeita.
            let id = "58a9b20e-37c6-4fa9-b9f1-516e534f47c4";
            if let Ok(page) = list(ListFilter { tool: Some("claude".into()), id: Some(id.into()), include_subagents: true, ..Default::default() }).await {
                for m in &page.sessions {
                    println!("list cost {} tokens={} cost=${:.2}", m.id, m.usage.total(), m.cost_usd);
                }
                if !page.sessions.is_empty() {
                    let a = session_analysis("claude".into(), id.into()).await.unwrap();
                    println!("analysis cost={:.2} recorded={:.2} computed={:.2} source={} tree={:?}", a.cost.cost_usd, a.cost.recorded_usd, a.cost.computed_usd, a.cost.source, a.cost.tree_recorded_usd);
                    let kids = list(ListFilter { tool: Some("claude".into()), parent: Some(id.into()), limit: Some(1000), ..Default::default() }).await.unwrap();
                    let kc: f64 = kids.sessions.iter().map(|m| m.cost_usd).sum();
                    let kt: u64 = kids.sessions.iter().map(|m| m.usage.total()).sum();
                    println!("children n={} tokens={} computed=${:.2}", kids.sessions.len(), kt, kc);
                    let p = get("claude".into(), id.into(), Some(0), Some(10)).await.unwrap();
                    println!("resume_command={:?} resume_target={:?}", p.resume_command, p.resume_target);
                }
            }
            let by_model = usage(ListFilter { model: Some("claude-opus-5".into()), ..Default::default() }, "model".into()).await.unwrap();
            println!("usage model filter rows={:?}", by_model.rows.iter().map(|r| (&r.key, r.total_tokens)).collect::<Vec<_>>());
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Pipeline inteiro contra os logs reais desta máquina, num índice
    /// temporário (`OMNIGET_DATA_DIR`). Rode com
    /// `cargo test -p omniget-core --lib real_pipeline -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_pipeline() {
        let dir = std::env::temp_dir().join(format!("omniget-sessions-api-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("OMNIGET_DATA_DIR", &dir);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let t = Instant::now();
            let st = refresh(false).await.unwrap();
            println!("refresh {:?} in {:?}", st, t.elapsed());
            for s in sources_info().await.unwrap() {
                println!("source {:>9} found={} beta={} sessions={} last={:?}", s.tool, s.found, s.beta, s.sessions, s.last_activity);
            }
            let page = list(ListFilter { limit: Some(5), ..Default::default() }).await.unwrap();
            println!("list total={} first={:?}", page.total, page.sessions.first().map(|m| (&m.tool, &m.id, &m.title, m.usage.total(), m.cost_usd)));
            let by_tool = usage(ListFilter::default(), "tool".into()).await.unwrap();
            for r in &by_tool.rows {
                println!("usage tool={} tokens={} cost=${:.2} sessions={} unpriced={}", r.key, r.total_tokens, r.cost_usd, r.sessions, r.unpriced_tokens);
            }
            println!("unpriced models: {:?}", by_tool.unpriced_models);
            let by_model = usage(ListFilter::default(), "model".into()).await.unwrap();
            for r in by_model.rows.iter().take(6) {
                println!("usage model={} tokens={} cost=${:.2}", r.key, r.total_tokens, r.cost_usd);
            }
            let hm = heatmap(ListFilter::default(), None).await.unwrap();
            println!("heatmap days={} active={} streak cur={} longest={} peak_hour={:?} busiest={:?}", hm.days.len(), hm.stats.active_days, hm.stats.current_streak, hm.stats.longest_streak, hm.stats.peak_hour, hm.stats.busiest_day);
            let ag = agents(ListFilter::default()).await.unwrap();
            println!("agents total={} types={} top={:?} patterns={:?}", ag.total_invocations, ag.agent_types, ag.top_agent, ag.patterns.iter().take(3).map(|p| (&p.pattern, p.count)).collect::<Vec<_>>());
            println!("subagent sessions {:?}", ag.subagent_sessions);
            let r = retro(None, None, None).await.unwrap();
            println!("retro sessions={} tokens={} cost=${:.2} top_models={:?} milestones={}", r.totals.sessions, r.totals.total_tokens, r.totals.cost_usd, r.top_models.iter().map(|m| &m.name).collect::<Vec<_>>(), r.milestones.len());
            for m in r.milestones.iter().take(8) {
                println!("  milestone {} {} {} {}", m.date, m.kind, m.label, m.value);
            }
            // Uma sessão do Claude com subagentes e uma do Codex.
            let with_kids = index::with_index(|idx| {
                idx.conn()
                    .query_row(
                        "SELECT tool, parent_session FROM sessions WHERE parent_session IS NOT NULL AND parent_session <> '' GROUP BY tool, parent_session ORDER BY COUNT(*) DESC LIMIT 1",
                        [],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                    )
                    .ok()
            })
            .unwrap();
            if let Some((tool, id)) = with_kids {
                let a = session_analysis(tool.clone(), id.clone()).await.unwrap();
                println!("analysis {tool}/{id}: turns={} tools={} agent%={:.0} cache_eff={:.0}% cost=${:.2} ({}) tips={:?}", a.counts.turns, a.counts.tool_calls, a.time.agent_pct, a.cache.efficiency, a.cost.cost_usd, a.cost.source, a.tips.iter().map(|t| &t.code).collect::<Vec<_>>());
                let g = team(tool.clone(), id.clone()).await.unwrap();
                println!("team nodes={} edges={} matched_spawns={} tasks={} is_team={}", g.nodes.len(), g.edges.len(), g.nodes.iter().filter(|n| n.session_id.is_some() && n.spawn_call_id.is_some()).count(), g.tasks.len(), g.is_team);
                let p = get(tool.clone(), id.clone(), Some(0), Some(20)).await.unwrap();
                println!("get page0 turns={} total={} pages={} resume={:?}", p.turns.len(), p.total, p.pages, p.resume_command);
                let hits = search_in(tool.clone(), id.clone(), "agent".into()).await.unwrap();
                println!("search_in hits={} first={:?}", hits.len(), hits.first().map(|h| (&h.snippet, h.turn)));
                let e = export(tool.clone(), id.clone(), "context".into(), Some("gemini".into())).await.unwrap();
                println!("export context {} chars, {}/{} turns", e.content.len(), e.turns_included, e.turns_total);
            }
            let codex = list(ListFilter { tool: Some("codex".into()), limit: Some(1), ..Default::default() }).await.unwrap();
            if let Some(m) = codex.sessions.first() {
                let a = session_analysis(m.tool.clone(), m.id.clone()).await.unwrap();
                println!("codex {} title={:?} tokens={} cost=${:.2} models={:?} tools={:?}", m.id, m.title, a.usage.total(), a.cost.cost_usd, a.models.iter().map(|x| &x.model).collect::<Vec<_>>(), a.tools.iter().take(4).map(|t| (&t.name, t.count)).collect::<Vec<_>>());
                println!("codex resume: {:?}", resume_command(m.tool.clone(), m.id.clone()).await.unwrap());
            }
            let t = Instant::now();
            let res = search(SearchQuery { text: "dedupe".into(), limit: Some(20), ..Default::default() }).await.unwrap();
            println!("search 'dedupe' hits={} scanned={} matched={} in {:?}", res.hits.len(), res.sessions_scanned, res.sessions_matched, t.elapsed());
            let act = active(3600).await.unwrap();
            println!("active(1h)={:?}", act.iter().take(3).map(|a| (&a.meta.tool, &a.meta.id, &a.state, a.idle_secs)).collect::<Vec<_>>());
        });
        std::fs::remove_dir_all(&dir).ok();
    }
}
