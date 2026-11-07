use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::cost::PriceBook;
use super::index::{self, ListFilter, ListPage};
use super::model::{Session, SessionMeta, SessionSource, Turn};
use super::parsers::{self, group_a::claude, group_a::codex};
use super::util;

pub const ERR_NOT_FOUND: &str = "ERR_SESSION_NOT_FOUND";
pub const ERR_TASK: &str = "ERR_SESSIONS_TASK";
const FRESH: Duration = Duration::from_secs(10);
const SOURCES_TTL: Duration = Duration::from_secs(300);

type Sources = Arc<Vec<Box<dyn SessionSource>>>;
static SOURCES: Mutex<Option<(Instant, Sources)>> = Mutex::new(None);

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

fn fresh() -> Result<(), String> {
    index::ensure_fresh_global(Some(FRESH), &sources()).map(|_| ())
}

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
    pub first_index: usize,
    pub page: u32,
    pub pages: u32,
    pub total: usize,
    pub has_older: bool,
    pub resume_command: Option<String>,
    pub resume_target: Option<ResumeTarget>,
}

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

fn resume_for(meta: &SessionMeta) -> Option<String> {
    let src = sources();
    src.iter()
        .find(|s| s.tool() == meta.tool)?
        .resume_command(meta)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResumeTarget {
    pub driver: String,
    pub instance_id: String,
    pub cursor: serde_json::Value,
    pub session_id: String,
    pub cwd: Option<String>,
    pub account: Option<String>,
    pub config_dir: Option<String>,
}

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
        "kilo" if meta.source.extension().and_then(|e| e.to_str()) == Some("db") => acp("kilo"),
        "gemini" | "qwen" | "goose" | "kimi" => acp(meta.tool.as_str()),
        "cursor" | "copilot" | "cline" if acc == "cli" => acp(meta.tool.as_str()),
        "grok" if acc == "grok-build" => acp("grok"),
        _ => None,
    }
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
    pub found: bool,
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
    pub state: String,
    pub idle_secs: u64,
    pub pending_tool: bool,
}
