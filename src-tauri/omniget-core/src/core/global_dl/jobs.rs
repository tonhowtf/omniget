//! Downloads a resolved tree: folders become `NN - title` directories, items
//! become files, DRM items are refused, files that already exist are skipped
//! (so a job can be run again to fetch only what is missing).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures::StreamExt;
use serde::Serialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::{emit, Node, Source};
use crate::core::filename::sanitize_path_component;

/// The app plugs the Downloads queue in here, for `ytdlp` and `dash` items.
pub type Handoff = Box<dyn Fn(String) + Send + Sync>;

fn handoff() -> &'static OnceLock<Handoff> {
    static H: OnceLock<Handoff> = OnceLock::new();
    &H
}

pub fn set_handoff(f: Handoff) {
    let _ = handoff().set(f);
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemState {
    pub id: String,
    pub title: String,
    pub path: String,
    /// queued | active | done | skipped | drm | handed_off | failed
    pub status: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobState {
    pub id: u64,
    pub title: String,
    pub out_dir: String,
    /// running | done | cancelled
    pub status: String,
    pub total: usize,
    pub done: usize,
    pub skipped: usize,
    pub drm: usize,
    pub failed: usize,
    pub handed_off: usize,
    pub bytes: u64,
    pub started_at: i64,
    pub items: Vec<ItemState>,
}

struct Job {
    state: Mutex<JobState>,
    cancel: CancellationToken,
    last_emit: Mutex<Instant>,
    /// The connector that made the tree, for lazy items and expired links.
    origin: Option<Origin>,
}

#[derive(Clone)]
struct Origin {
    connector: String,
    dir: Option<String>,
    page: String,
}

fn jobs() -> &'static Mutex<Vec<Arc<Job>>> {
    static J: OnceLock<Mutex<Vec<Arc<Job>>>> = OnceLock::new();
    J.get_or_init(Mutex::default)
}

static NEXT: AtomicU64 = AtomicU64::new(1);

pub fn list() -> Vec<JobState> {
    jobs()
        .lock()
        .map(|j| j.iter().map(|x| x.snapshot()).collect())
        .unwrap_or_default()
}

pub fn cancel(id: u64) -> bool {
    let j = jobs()
        .lock()
        .ok()
        .and_then(|j| j.iter().find(|x| x.id() == id).cloned());
    match j {
        Some(j) => {
            j.cancel.cancel();
            true
        }
        None => false,
    }
}

pub fn clear_finished() {
    if let Ok(mut j) = jobs().lock() {
        j.retain(|x| x.snapshot().status == "running");
    }
}

impl Job {
    fn id(&self) -> u64 {
        self.state.lock().map(|s| s.id).unwrap_or(0)
    }
    fn snapshot(&self) -> JobState {
        self.state
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }
    fn emit(&self, force: bool) {
        let mut last = self.last_emit.lock().unwrap_or_else(|e| e.into_inner());
        if !force && last.elapsed() < Duration::from_millis(400) {
            return;
        }
        *last = Instant::now();
        emit(json!({ "kind": "job", "job": self.snapshot() }));
    }
    fn update(&self, idx: usize, f: impl FnOnce(&mut ItemState)) {
        if let Ok(mut s) = self.state.lock() {
            if let Some(it) = s.items.get_mut(idx) {
                f(it);
            }
            let mut c = (0, 0, 0, 0, 0, 0u64);
            for it in &s.items {
                match it.status.as_str() {
                    "done" => c.0 += 1,
                    "skipped" => c.1 += 1,
                    "drm" => c.2 += 1,
                    "failed" => c.3 += 1,
                    "handed_off" => c.4 += 1,
                    _ => {}
                }
                c.5 += it.bytes;
            }
            (s.done, s.skipped, s.drm, s.failed, s.handed_off, s.bytes) = c;
        }
    }
}

struct Planned {
    node: Node,
    path: PathBuf,
}

fn ext_for(node: &Node, src: &Source) -> String {
    if let Some(e) = &src.ext {
        return e.trim_start_matches('.').to_string();
    }
    if src.format == "hls" {
        return "mp4".into();
    }
    if src.format == "text" {
        return "html".into();
    }
    if let Some(u) = &src.url {
        let path = u.split(['?', '#']).next().unwrap_or(u);
        if let Some((_, e)) = path.rsplit('/').next().unwrap_or("").rsplit_once('.') {
            if (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()) {
                return e.to_ascii_lowercase();
            }
        }
    }
    match node.kind.as_str() {
        "video" => "mp4",
        "audio" => "m4a",
        "subtitle" => "vtt",
        "link" => "url",
        "text" => "html",
        _ => "bin",
    }
    .into()
}

fn clip(name: &str) -> String {
    let s = sanitize_path_component(name.trim());
    let s: String = s.chars().take(120).collect();
    if s.trim().is_empty() {
        "untitled".into()
    } else {
        s.trim().to_string()
    }
}

/// Flattens the selected part of the tree into files with their paths.
fn plan(node: &Node, dir: &Path, selected: &Option<HashSet<String>>, out: &mut Vec<Planned>) {
    let picked = |n: &Node| {
        selected.as_ref().is_none_or(|s| {
            s.contains(&n.id) || s.iter().any(|x| x.starts_with(&format!("{}.", n.id)))
        })
    };
    let mut n_folder = 0;
    let mut n_item = 0;
    for child in &node.children {
        if child.is_folder() {
            n_folder += 1;
            if picked(child) {
                let sub = dir.join(format!("{:02} - {}", n_folder, clip(&child.title)));
                plan(child, &sub, selected, out);
            }
        } else {
            n_item += 1;
            let wanted = selected.as_ref().is_none_or(|s| s.contains(&child.id));
            if !wanted {
                continue;
            }
            let src = child.source.clone().unwrap_or_default();
            let name = match &src.filename {
                Some(f) => clip(f),
                None => {
                    let title = clip(&child.title);
                    let ext = ext_for(child, &src);
                    // A title that already ends in the extension
                    // (`dummy.pdf`) does not get it twice.
                    if title
                        .to_ascii_lowercase()
                        .ends_with(&format!(".{}", ext.to_ascii_lowercase()))
                    {
                        format!("{:02} - {}", n_item, title)
                    } else {
                        format!("{:02} - {}.{}", n_item, title, ext)
                    }
                }
            };
            out.push(Planned {
                node: child.clone(),
                path: dir.join(name),
            });
        }
    }
}

/// Starts downloading `tree` into `out_dir/<tree title>`. `selected` holds
/// node ids; `None` means everything.
pub fn start(
    tree: Node,
    selected: Option<Vec<String>>,
    out_dir: PathBuf,
    max_height: Option<u32>,
) -> Result<JobState, String> {
    let root = out_dir.join(clip(&tree.title));
    let selected = selected.map(|v| v.into_iter().collect::<HashSet<_>>());
    let mut planned = Vec::new();
    plan(&tree, &root, &selected, &mut planned);
    if planned.is_empty() {
        return Err("nothing selected to download".into());
    }
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    let items = planned
        .iter()
        .map(|p| ItemState {
            id: p.node.id.clone(),
            title: p.node.title.clone(),
            path: p.path.display().to_string(),
            status: "queued".into(),
            bytes: 0,
            error: None,
        })
        .collect::<Vec<_>>();
    let job = Arc::new(Job {
        state: Mutex::new(JobState {
            id,
            title: tree.title.clone(),
            out_dir: root.display().to_string(),
            status: "running".into(),
            total: items.len(),
            done: 0,
            skipped: 0,
            drm: 0,
            failed: 0,
            handed_off: 0,
            bytes: 0,
            started_at: chrono::Utc::now().timestamp_millis(),
            items,
        }),
        cancel: CancellationToken::new(),
        last_emit: Mutex::new(Instant::now()),
        origin: tree.connector.clone().map(|connector| Origin {
            connector,
            dir: tree.connector_dir.clone(),
            page: tree.origin.clone().unwrap_or_default(),
        }),
    });
    if let Ok(mut j) = jobs().lock() {
        j.push(job.clone());
    }
    let snapshot = job.snapshot();
    job.emit(true);
    tokio::spawn(run(job, planned, max_height));
    Ok(snapshot)
}

async fn run(job: Arc<Job>, planned: Vec<Planned>, max_height: Option<u32>) {
    // A tab of the site keeps its bot-check cookies fresh for the whole job.
    if let Some(o) = &job.origin {
        super::browser::warm_up(&o.page, false).await;
    }
    let job2 = job.clone();
    futures::stream::iter(planned.into_iter().enumerate())
        .for_each_concurrent(3, |(idx, p)| {
            let job = job2.clone();
            async move {
                if job.cancel.is_cancelled() {
                    return;
                }
                let r = one(&job, idx, &p, max_height).await;
                job.update(idx, |it| match r {
                    Ok(status) => it.status = status.into(),
                    Err(e) => {
                        it.status = "failed".into();
                        it.error = Some(e);
                    }
                });
                job.emit(true);
            }
        })
        .await;
    if let Ok(mut s) = job.state.lock() {
        s.status = if job.cancel.is_cancelled() {
            "cancelled"
        } else {
            "done"
        }
        .into();
    }
    job.emit(true);
}

/// Downloads go out with the browser's session for that host: its exact
/// User-Agent and its cookies (`cf_clearance` among them). Cloudflare answers
/// 403 to a Chrome UA over another TLS stack, and 200 once the clearance the
/// browser earned comes along. Headers the connector set win.
async fn with_browser_session(src: &mut Source) {
    let Some(b) = super::browser::current() else {
        return;
    };
    let Some(host) = src.url.as_deref().and_then(super::host_of) else {
        return;
    };
    let has = |h: &BTreeMap<String, String>, k: &str| h.keys().any(|x| x.eq_ignore_ascii_case(k));
    if !has(&src.headers, "cookie") {
        let cookies = b.cookies_for(&host).await.unwrap_or_default();
        let jar: Vec<String> = cookies
            .iter()
            .filter_map(|c| Some(format!("{}={}", c["name"].as_str()?, c["value"].as_str()?)))
            .collect();
        if !jar.is_empty() {
            src.headers.insert("Cookie".into(), jar.join("; "));
        }
    }
    if !has(&src.headers, "user-agent") {
        if let Some(ua) = b.user_agent().await {
            src.headers.insert("User-Agent".into(), ua);
        }
    }
}

fn exists_with_content(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)
}

fn header_map(h: &BTreeMap<String, String>) -> reqwest::header::HeaderMap {
    let mut m = reqwest::header::HeaderMap::new();
    for (k, v) in h {
        if k.eq_ignore_ascii_case("range") || k.eq_ignore_ascii_case("content-length") {
            continue;
        }
        if let (Ok(k), Ok(v)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            m.insert(k, v);
        }
    }
    m
}

/// A signed link that stopped working: worth asking the connector for a
/// fresh one.
fn looks_expired(e: &str) -> bool {
    ["401", "403", "410", "Forbidden", "Gone", "expired"]
        .iter()
        .any(|m| e.contains(m))
}

async fn one(
    job: &Arc<Job>,
    idx: usize,
    p: &Planned,
    max_height: Option<u32>,
) -> Result<&'static str, String> {
    if p.node.drm {
        return Ok("drm");
    }
    let known = p
        .node
        .source
        .as_ref()
        .map(|s| s.format.as_str())
        .unwrap_or("");
    if known != "ytdlp" && known != "dash" && exists_with_content(&p.path) {
        return Ok("skipped");
    }
    job.update(idx, |it| it.status = "active".into());
    job.emit(true);
    if let Some(parent) = p.path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut node = p.node.clone();
    for attempt in 0..2 {
        if node.is_lazy() || attempt == 1 {
            let key = node.key.clone().ok_or("item has no source")?;
            let o = job
                .origin
                .clone()
                .ok_or("this tree has no connector to resolve items")?;
            let fresh =
                super::connector::resolve_item(&o.connector, o.dir.as_deref(), &o.page, &key)
                    .await?;
            if fresh.drm {
                return Ok("drm");
            }
            let mut src = fresh.source.ok_or("resolveItem returned no source")?;
            let old = node.source.clone().unwrap_or_default();
            src.filename = src.filename.or(old.filename);
            src.convert = src.convert.or(old.convert);
            node.source = Some(src);
        }
        let mut src = node.source.clone().ok_or("item has no source")?;
        if matches!(src.format.as_str(), "hls" | "direct" | "") {
            with_browser_session(&mut src).await;
        }
        match fetch(job, idx, p, src, max_height).await {
            Err(e)
                if attempt == 0
                    && node.key.is_some()
                    && job.origin.is_some()
                    && looks_expired(&e) =>
            {
                tracing::info!(
                    "[global-dl] {} looks expired ({e}), resolving again",
                    p.node.title
                );
                if let Some(o) = &job.origin {
                    super::browser::warm_up(&o.page, true).await;
                }
                continue;
            }
            r => return r,
        }
    }
    Err("the link kept failing after a fresh resolve".into())
}

async fn fetch(
    job: &Arc<Job>,
    idx: usize,
    p: &Planned,
    src: Source,
    max_height: Option<u32>,
) -> Result<&'static str, String> {
    match src.format.as_str() {
        "text" => {
            let body = src.content.unwrap_or_default();
            std::fs::write(&p.path, body.as_bytes()).map_err(|e| e.to_string())?;
            job.update(idx, |it| it.bytes = body.len() as u64);
            Ok("done")
        }
        "link" => {
            let body = format!("[InternetShortcut]\nURL={}\n", src.url.unwrap_or_default());
            std::fs::write(&p.path, body).map_err(|e| e.to_string())?;
            Ok("done")
        }
        "ytdlp" | "dash" => {
            let url = src.url.ok_or("item has no url")?;
            let f = handoff()
                .get()
                .ok_or("the Downloads queue is not available here")?;
            f(url);
            Ok("handed_off")
        }
        "hls" => {
            let url = src.url.ok_or("item has no url")?;
            let referer = src
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("referer"))
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let client = reqwest::Client::builder()
                .use_rustls_tls()
                .default_headers(header_map(&src.headers))
                .timeout(Duration::from_secs(120))
                .build()
                .map_err(|e| e.to_string())?;
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
            let j = job.clone();
            let counter = tokio::spawn(async move {
                let mut total = 0u64;
                while let Some(n) = rx.recv().await {
                    total += n;
                    j.update(idx, |it| it.bytes = total);
                    j.emit(false);
                }
            });
            let out = p.path.display().to_string();
            // The HLS downloader defaults to a Chrome User-Agent of its own;
            // send the browser's (set by `with_browser_session`) or ours.
            let ua = src
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| format!("OmniGet/{}", env!("CARGO_PKG_VERSION")));
            let r = crate::core::hls_downloader::HlsDownloader::with_client(client)
                .with_user_agent_override(Some(ua))
                .download_with_quality(
                    &url,
                    &out,
                    &referer,
                    Some(tx),
                    job.cancel.child_token(),
                    8,
                    5,
                    max_height,
                )
                .await;
            let _ = counter.await;
            let r = r.map_err(|e| e.to_string())?;
            job.update(idx, |it| it.bytes = r.file_size);
            Ok("done")
        }
        _ => {
            let url = src.url_for(max_height).ok_or("item has no url")?;
            let client = reqwest::Client::builder()
                .use_rustls_tls()
                .build()
                .map_err(|e| e.to_string())?;
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);
            let j = job.clone();
            let counter = tokio::spawn(async move {
                while let Some(u) = rx.recv().await {
                    let u: crate::models::progress::ProgressUpdate = u;
                    if let Some(b) = u.downloaded_bytes {
                        j.update(idx, |it| it.bytes = b);
                        j.emit(false);
                    }
                }
            });
            // `convert: srt` downloads the WebVTT next to the target first.
            let srt = src.convert.as_deref() == Some("srt");
            let target = if srt {
                p.path.with_extension("vtt.part")
            } else {
                p.path.clone()
            };
            let r = crate::core::direct_downloader::download_direct_with_headers(
                &client,
                &url,
                &target,
                tx,
                Some(header_map(&src.headers)),
                Some(&job.cancel),
            )
            .await;
            let _ = counter.await;
            let n = r.map_err(|e| e.to_string())?;
            if srt {
                let vtt = std::fs::read_to_string(&target).map_err(|e| e.to_string())?;
                std::fs::write(&p.path, vtt_to_srt(&vtt)).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_file(&target);
            }
            job.update(idx, |it| it.bytes = n);
            Ok("done")
        }
    }
}

/// WebVTT → SubRip: drops the header, NOTE/STYLE blocks and cue settings,
/// numbers the cues and uses a comma before the milliseconds.
pub fn vtt_to_srt(vtt: &str) -> String {
    let text = vtt.replace("\r\n", "\n");
    let mut out = String::new();
    let mut n = 0;
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let mut parts = lines[ti].split("-->");
        let (Some(a), Some(b)) = (parts.next(), parts.next()) else {
            continue;
        };
        let fix = |t: &str| {
            let t = t.split_whitespace().next().unwrap_or("").replace('.', ",");
            if t.matches(':').count() == 1 {
                format!("00:{t}")
            } else {
                t
            }
        };
        n += 1;
        out.push_str(&format!("{n}\n{} --> {}\n", fix(a), fix(b)));
        for l in &lines[ti + 1..] {
            out.push_str(l);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}
