use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::detect::Detection;
use super::exec::parse_version;
use super::latest::{self, VersionState};
use super::owner::OwnerMethod;
use super::table;

pub const AUTO_INTERVAL: u64 = 24 * 3600;
const CHANGELOG_LINES: usize = 12;
const CHANGELOG_CHARS: usize = 900;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Changelog {
    pub tag: String,
    pub name: Option<String>,
    pub url: String,
    pub published_at: Option<String>,
    pub excerpt: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUpdate {
    pub id: String,
    pub name: String,
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub state: VersionState,
    pub source: String,
    pub update_available: bool,
    pub auto_updates: bool,
    pub changelog: Option<Changelog>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdatesReport {
    pub checked_at: u64,
    pub next_auto_at: u64,
    pub from_cache: bool,
    pub available: usize,
    pub tools: Vec<ToolUpdate>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn disk_path() -> Option<PathBuf> {
    Some(
        crate::core::paths::app_data_dir()?
            .join("clitools")
            .join("updates.json"),
    )
}

fn store() -> &'static Mutex<Option<UpdatesReport>> {
    use std::sync::OnceLock;
    static S: OnceLock<Mutex<Option<UpdatesReport>>> = OnceLock::new();
    S.get_or_init(|| {
        let loaded = disk_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok());
        Mutex::new(loaded)
    })
}

fn save(report: &UpdatesReport) {
    if let Ok(mut g) = store().lock() {
        *g = Some(report.clone());
    }
    if let Some(p) = disk_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(report) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }
}

pub fn last_report() -> Option<UpdatesReport> {
    store().lock().ok().and_then(|g| g.clone())
}

pub fn due(at: u64) -> bool {
    match last_report() {
        Some(r) => at.saturating_sub(r.checked_at) >= AUTO_INTERVAL,
        None => true,
    }
}

pub fn excerpt(body: &str, max_lines: usize, max_chars: usize) -> (String, bool) {
    let mut out: Vec<String> = Vec::new();
    let mut chars = 0usize;
    let mut truncated = false;
    let mut in_comment = false;
    let mut blank = false;
    for raw in body.lines() {
        let line = raw.trim_end();
        let t = line.trim();
        if in_comment {
            if t.contains("-->") {
                in_comment = false;
            }
            continue;
        }
        if t.starts_with("<!--") {
            if !t.contains("-->") {
                in_comment = true;
            }
            continue;
        }
        if t.starts_with("![") || t.starts_with("<img") {
            continue;
        }
        if t.is_empty() {
            if !out.is_empty() {
                blank = true;
            }
            continue;
        }
        if out.len() >= max_lines || chars + t.len() > max_chars {
            truncated = true;
            break;
        }
        if blank {
            out.push(String::new());
            blank = false;
        }
        chars += t.len();
        out.push(line.to_string());
    }
    (out.join("\n"), truncated)
}

fn client() -> Result<reqwest::Client, String> {
    crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(Duration::from_secs(5))
        .user_agent("OmniGet-clitools")
        .build()
        .map_err(|e| e.to_string())
}

async fn release(client: &reqwest::Client, repo: &str, suffix: &str) -> Option<serde_json::Value> {
    let url = format!("https://api.github.com/repos/{repo}/releases/{suffix}");
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

fn changelog_from(v: &serde_json::Value) -> Option<Changelog> {
    let tag = v["tag_name"].as_str()?.to_string();
    let (excerpt, truncated) = excerpt(
        v["body"].as_str().unwrap_or_default(),
        CHANGELOG_LINES,
        CHANGELOG_CHARS,
    );
    Some(Changelog {
        name: v["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        url: v["html_url"].as_str().unwrap_or_default().to_string(),
        published_at: v["published_at"].as_str().map(str::to_string),
        tag,
        excerpt,
        truncated,
    })
}

pub async fn changelog(repo: &str, version: &str) -> Option<Changelog> {
    let client = client().ok()?;
    if let Some(v) = release(&client, repo, "latest").await {
        let tag = v["tag_name"].as_str().unwrap_or_default();
        if parse_version(tag).as_deref() == Some(version) {
            return changelog_from(&v);
        }
    }
    for tag in [format!("v{version}"), version.to_string()] {
        if let Some(v) = release(&client, repo, &format!("tags/{tag}")).await {
            return changelog_from(&v);
        }
    }
    None
}

pub async fn check(detections: &[Detection], force: bool) -> UpdatesReport {
    let previous: BTreeMap<String, ToolUpdate> = last_report()
        .map(|r| r.tools.into_iter().map(|t| (t.id.clone(), t)).collect())
        .unwrap_or_default();
    let jobs = detections
        .iter()
        .filter(|d| d.installed && d.binary.is_some())
        .filter_map(|d| table::get(&d.id).map(|t| (t, d.clone())))
        .map(|(t, d)| {
            let prev = previous.get(&t.id).cloned();
            async move {
                let brew = (d.owner.method == OwnerMethod::Brew && d.owner.proven)
                    .then(|| t.brew.as_ref().map(|b| (b.name.clone(), b.cask)))
                    .flatten();
                let l = latest::for_tool(t, d.version.clone(), brew, force).await;
                let behind = l.state == VersionState::BehindLatest;
                let mut log = None;
                if behind {
                    if let (Some(repo), Some(ver)) = (t.repo.as_deref(), l.latest.as_deref()) {
                        log = prev
                            .and_then(|p| p.changelog.filter(|_| p.latest.as_deref() == Some(ver)));
                        if log.is_none() {
                            log = changelog(repo, ver).await;
                        }
                    }
                }
                ToolUpdate {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    installed: l.installed.clone(),
                    latest: l.latest.clone(),
                    state: l.state,
                    source: l.source.clone(),
                    update_available: behind,
                    auto_updates: t.auto_updates,
                    changelog: log,
                    error: l.error.clone(),
                }
            }
        });
    let mut tools = futures::future::join_all(jobs).await;
    tools.sort_by(|a, b| {
        b.update_available
            .cmp(&a.update_available)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let at = now();
    let report = UpdatesReport {
        checked_at: at,
        next_auto_at: at + AUTO_INTERVAL,
        from_cache: false,
        available: tools.iter().filter(|t| t.update_available).count(),
        tools,
    };
    save(&report);
    report
}

pub fn cached_report() -> Option<UpdatesReport> {
    last_report().map(|mut r| {
        r.from_cache = true;
        r
    })
}
