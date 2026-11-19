use std::collections::{BTreeMap, HashMap, HashSet};
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
use crate::core::platform_optout;

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
    pub status: String,
    pub total: usize,
    pub done: usize,
    pub skipped: usize,
    pub drm: usize,
    pub failed: usize,
    pub handed_off: usize,
    pub cancelled: usize,
    pub bytes: u64,
    pub started_at: i64,
    pub items: Vec<ItemState>,
}

struct Job {
    state: Mutex<JobState>,
    cancel: CancellationToken,
    last_emit: Mutex<Instant>,
    origin: Option<Origin>,
}

#[derive(Clone)]
struct Origin {
    connector: String,
    dir: Option<String>,
    page: String,
    domains: Vec<String>,
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

pub fn running_count() -> usize {
    jobs()
        .lock()
        .map(|j| {
            j.iter()
                .filter(|x| x.snapshot().status == "running")
                .count()
        })
        .unwrap_or(0)
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
            let mut c = (0, 0, 0, 0, 0, 0, 0u64);
            for it in &s.items {
                match it.status.as_str() {
                    "done" => c.0 += 1,
                    "skipped" => c.1 += 1,
                    "drm" => c.2 += 1,
                    "failed" => c.3 += 1,
                    "handed_off" => c.4 += 1,
                    "cancelled" => c.5 += 1,
                    _ => {}
                }
                c.6 += it.bytes;
            }
            (
                s.done,
                s.skipped,
                s.drm,
                s.failed,
                s.handed_off,
                s.cancelled,
                s.bytes,
            ) = c;
        }
    }
    fn finish(&self) {
        let cancelled = self.cancel.is_cancelled();
        if let Ok(mut s) = self.state.lock() {
            if cancelled {
                for it in &mut s.items {
                    if matches!(it.status.as_str(), "queued" | "active") {
                        it.status = "cancelled".into();
                    }
                }
            }
        }
        self.update(usize::MAX, |_| {});
        if let Ok(mut s) = self.state.lock() {
            s.status = final_status(cancelled, &s).into();
        }
    }
}

fn final_status(cancelled: bool, s: &JobState) -> &'static str {
    if cancelled {
        "cancelled"
    } else if s.failed == 0 {
        "done"
    } else if s.done + s.skipped + s.handed_off == 0 {
        "failed"
    } else {
        "partial"
    }
}

struct Planned {
    node: Node,
    path: PathBuf,
    base: String,
    number: usize,
    who: String,
}

const SCRIPT_EXTS: &[&str] = &["php", "asp", "aspx", "jsp", "cgi", "ashx", "pl"];

fn known_ext(src: &Source) -> Option<String> {
    if src.convert.as_deref() == Some("srt") {
        return Some("srt".into());
    }
    if let Some(e) = &src.ext {
        let e = e.trim_start_matches('.');
        if (1..=8).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(e.to_string());
        }
    }
    match src.format.as_str() {
        "hls" => return Some("mp4".into()),
        "text" => return Some("html".into()),
        "link" => return Some("url".into()),
        _ => {}
    }
    let u = src.url.as_ref()?;
    let path = u.split(['?', '#']).next().unwrap_or(u);
    let (_, e) = path.rsplit('/').next().unwrap_or("").rsplit_once('.')?;
    let e = e.to_ascii_lowercase();
    ((1..=5).contains(&e.len())
        && e.chars().all(|c| c.is_ascii_alphanumeric())
        && !SCRIPT_EXTS.contains(&e.as_str()))
    .then_some(e)
}

fn ext_for(node: &Node, src: &Source) -> String {
    known_ext(src).unwrap_or_else(|| {
        match node.kind.as_str() {
            "video" => "mp4",
            "audio" => "m4a",
            "subtitle" => "vtt",
            "link" => "url",
            "text" => "html",
            _ => "bin",
        }
        .into()
    })
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

fn name_ext(name: &str) -> Option<&str> {
    let (stem, e) = name.rsplit_once('.')?;
    (!stem.is_empty()
        && (1..=8).contains(&e.len())
        && e.chars().all(|c| c.is_ascii_alphanumeric())
        && e.chars().any(|c| c.is_ascii_alphabetic()))
    .then_some(e)
}

fn clip_file(name: &str) -> String {
    let s = sanitize_path_component(name.trim());
    if s.chars().count() <= 120 {
        return clip(&s);
    }
    match name_ext(&s).map(|e| e.len() + 1) {
        Some(keep) => {
            let stem: String = s[..s.len() - keep].chars().take(120 - keep).collect();
            format!("{}{}", stem.trim_end(), &s[s.len() - keep..])
        }
        None => clip(&s),
    }
}

fn file_name(number: usize, title: &str, node: &Node, src: &Source) -> String {
    let mut name = match &src.filename {
        Some(f) => {
            let mut name = clip_file(f);
            if name_ext(&name).is_none() {
                if let Some(e) = known_ext(src) {
                    name = format!("{name}.{e}");
                }
            }
            name
        }
        None => {
            let title = clip(title);
            let ext = ext_for(node, src);
            if title
                .to_ascii_lowercase()
                .ends_with(&format!(".{}", ext.to_ascii_lowercase()))
            {
                format!("{number:02} - {title}")
            } else {
                format!("{number:02} - {title}.{ext}")
            }
        }
    };
    if src.format == "link" && !name.to_ascii_lowercase().ends_with(".url") {
        name.push_str(".url");
    }
    name
}

pub(crate) fn numbered(path: &Path, n: usize) -> PathBuf {
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let numbered = match name_ext(&name) {
        Some(e) => format!("{} ({n}).{e}", &name[..name.len() - e.len() - 1]),
        None => format!("{name} ({n})"),
    };
    path.with_file_name(numbered)
}

fn owners() -> &'static Mutex<HashMap<String, String>> {
    static O: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    O.get_or_init(Mutex::default)
}

fn claim(wanted: &Path, who: &str, resumes: bool) -> PathBuf {
    let mut o = owners().lock().unwrap_or_else(|e| e.into_inner());
    if o.len() > 200_000 {
        o.clear();
    }
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            wanted.to_path_buf()
        } else {
            numbered(wanted, n)
        };
        let key = candidate.display().to_string().to_lowercase();
        let free = match o.get(&key) {
            Some(owner) => owner == who,
            None => resumes || !exists_with_content(&candidate),
        };
        if free {
            o.insert(key, who.to_string());
            return candidate;
        }
        n += 1;
    }
}

fn identity(node: &Node, origin: Option<&Origin>) -> String {
    match origin {
        Some(o) => format!(
            "{}|{}|{}",
            o.connector,
            o.page,
            node.key.as_deref().unwrap_or(&node.id)
        ),
        None => node
            .source
            .as_ref()
            .and_then(|s| s.url.clone())
            .unwrap_or_else(|| format!("{}|{}", node.id, node.title)),
    }
}

fn hands_off(src: Option<&Source>) -> bool {
    src.is_some_and(|s| matches!(s.format.as_str(), "ytdlp" | "dash"))
}

fn plan(node: &Node, dir: &Path, origin: Option<&Origin>, out: &mut Vec<Planned>) {
    let mut n_folder = 0;
    let mut n_item = 0;
    for child in &node.children {
        if child.is_folder() {
            n_folder += 1;
            let sub = dir.join(format!("{:02} - {}", n_folder, clip(&child.title)));
            plan(child, &sub, origin, out);
        } else {
            n_item += 1;
            let src = child.source.clone().unwrap_or_default();
            let base = file_name(n_item, &child.title, child, &src);
            let who = identity(child, origin);
            let path = if hands_off(Some(&src)) {
                dir.join(&base)
            } else {
                claim(&dir.join(&base), &who, origin.is_some())
            };
            out.push(Planned {
                node: child.clone(),
                path,
                base,
                number: n_item,
                who,
            });
        }
    }
}

fn final_name(p: &Planned, title: &str, src: &Source) -> String {
    let name = file_name(p.number, title, &p.node, src);
    let planned = p
        .path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name == p.base {
        return planned;
    }
    match name.strip_prefix(&p.base) {
        Some(ext) if ext.starts_with('.') && name_ext(&p.base).is_none() => {
            format!("{planned}{ext}")
        }
        _ => name,
    }
}

fn is_local_host(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(h)) => {
            let h = h.to_ascii_lowercase();
            h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local")
        }
        Some(url::Host::Ipv4(ip)) => crate::core::components::is_private_ip(ip.into()),
        Some(url::Host::Ipv6(ip)) => crate::core::components::is_private_ip(ip.into()),
        None => true,
    }
}

fn is_declared(host: &str, domains: &[String]) -> bool {
    domains.iter().any(|d| {
        let d = d
            .trim()
            .trim_start_matches("*.")
            .trim_start_matches('.')
            .to_ascii_lowercase();
        (d.contains('.') || d == "localhost") && super::host_matches(host, &d)
    })
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Reach {
    Credentials,
    Anonymous,
}

// WHY: connectors take media from CDNs they cannot list (branas gets Panda Video, courseflix gets
// signed storage links), so an undeclared host stays reachable, without any credential.
fn reach_for(url: &str, domains: Option<&[String]>) -> Result<Reach, String> {
    let parsed = url::Url::parse(url).map_err(|_| "this item has an invalid link".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("{} links are not allowed", parsed.scheme()));
    }
    platform_optout::ensure_allowed(url)?;
    let Some(domains) = domains else {
        return Ok(Reach::Credentials);
    };
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if is_declared(&host, domains) {
        Ok(Reach::Credentials)
    } else if is_local_host(&parsed) {
        Err(format!(
            "{host} is a local address outside this connector's domains"
        ))
    } else {
        Ok(Reach::Anonymous)
    }
}

fn check_source(src: &Source, domains: Option<&[String]>) -> Result<(), String> {
    src.urls()
        .try_for_each(|u| reach_for(u, domains).map(|_| ()))
}

fn keep_anonymous_headers(headers: &mut BTreeMap<String, String>) {
    headers.retain(|k, _| {
        matches!(
            k.to_ascii_lowercase().as_str(),
            "referer" | "origin" | "user-agent" | "accept" | "accept-language"
        )
    });
}

// WHY: without a connector there is no declared list, so only the playlist's own host is trusted.
fn hls_credentials(
    headers: &BTreeMap<String, String>,
    playlist: &str,
    domains: Option<&[String]>,
) -> crate::core::hls_downloader::RequestHeaders {
    let mut credentials = headers.clone();
    let anonymous = {
        let mut a = headers.clone();
        keep_anonymous_headers(&mut a);
        a
    };
    credentials.retain(|k, _| !anonymous.contains_key(k));
    let credentials = header_map(&credentials);
    let domains = domains.map(<[String]>::to_vec);
    let host_of = |u: &str| {
        url::Url::parse(u)
            .ok()
            .and_then(|p| p.host_str().map(str::to_ascii_lowercase))
    };
    let playlist_host = host_of(playlist);
    Arc::new(move |url: &str| {
        let trusted = match &domains {
            Some(d) => matches!(reach_for(url, Some(d)), Ok(Reach::Credentials)),
            None => playlist_host.is_some() && host_of(url) == playlist_host,
        };
        if trusted {
            credentials.clone()
        } else {
            reqwest::header::HeaderMap::new()
        }
    })
}

struct JarCookie {
    name: String,
    value: String,
    domain: String,
    secure: bool,
}

#[derive(Default)]
struct ScopedCookies {
    domains: Option<Vec<String>>,
    jar: Vec<JarCookie>,
    fixed: Option<(String, String)>,
}

impl ScopedCookies {
    fn header_for(&self, url: &url::Url) -> Option<String> {
        let host = url.host_str()?.to_ascii_lowercase();
        if let Some((fixed_host, value)) = &self.fixed {
            return (*fixed_host == host).then(|| value.clone());
        }
        if self
            .domains
            .as_ref()
            .is_some_and(|d| !is_declared(&host, d))
        {
            return None;
        }
        let parts: Vec<String> = self
            .jar
            .iter()
            .filter(|c| super::host_matches(&host, &c.domain))
            .filter(|c| !c.secure || url.scheme() == "https")
            .map(|c| format!("{}={}", c.name, c.value))
            .collect();
        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

impl reqwest::cookie::CookieStore for ScopedCookies {
    fn set_cookies(
        &self,
        _: &mut dyn Iterator<Item = &reqwest::header::HeaderValue>,
        _: &url::Url,
    ) {
    }

    fn cookies(&self, url: &url::Url) -> Option<reqwest::header::HeaderValue> {
        reqwest::header::HeaderValue::from_str(&self.header_for(url)?).ok()
    }
}

fn job_client(
    builder: reqwest::ClientBuilder,
    domains: Option<&[String]>,
    cookies: Arc<ScopedCookies>,
) -> Result<reqwest::Client, String> {
    let domains = domains.map(<[String]>::to_vec);
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many redirects");
        }
        match reach_for(attempt.url().as_str(), domains.as_deref()) {
            Ok(_) => attempt.follow(),
            Err(e) => attempt.error(e),
        }
    });
    builder
        .use_rustls_tls()
        .redirect(policy)
        .cookie_provider(cookies)
        .build()
        .map_err(|e| e.to_string())
}

fn connector_domains(tree: &Node, id: &str) -> Result<Vec<String>, String> {
    let dir = tree
        .connector_dir
        .as_deref()
        .map(PathBuf::from)
        .filter(|d| d.is_dir());
    let manifest = match dir {
        Some(d) => super::connector::load(&d)?,
        None => {
            super::connector::find_by_id(id)
                .ok_or_else(|| format!("connector {id} is not installed"))?
                .manifest
        }
    };
    if manifest.id != id {
        return Err(format!("connector {id} does not match its folder"));
    }
    Ok(manifest.domains)
}

pub fn start(
    tree: Node,
    selected: Option<Vec<String>>,
    out_dir: PathBuf,
    max_height: Option<u32>,
) -> Result<JobState, String> {
    if let Some(page) = tree.origin.as_deref() {
        platform_optout::ensure_allowed(page)?;
    }
    let origin = match tree.connector.clone() {
        Some(connector) => Some(Origin {
            domains: connector_domains(&tree, &connector)?,
            connector,
            dir: tree.connector_dir.clone(),
            page: tree.origin.clone().unwrap_or_default(),
        }),
        None => None,
    };
    let root = out_dir.join(clip(&tree.title));
    let mut planned = Vec::new();
    plan(&tree, &root, origin.as_ref(), &mut planned);
    if let Some(selected) = selected {
        let selected = selected.into_iter().collect::<HashSet<_>>();
        planned.retain(|p| selected.contains(&p.node.id));
    }
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
            cancelled: 0,
            bytes: 0,
            started_at: chrono::Utc::now().timestamp_millis(),
            items,
        }),
        cancel: CancellationToken::new(),
        last_emit: Mutex::new(Instant::now()),
        origin,
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
    if let Some(o) = &job.origin {
        tokio::select! {
            _ = super::browser::warm_up(&o.page, false) => {}
            _ = job.cancel.cancelled() => {}
        }
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
                    Err(_) if job.cancel.is_cancelled() => it.status = "cancelled".into(),
                    Err(e) => {
                        it.status = "failed".into();
                        it.error = Some(e);
                    }
                });
                job.emit(true);
            }
        })
        .await;
    job.finish();
    job.emit(true);
}

fn fetched_url(src: &Source, max_height: Option<u32>) -> Option<String> {
    if src.format == "hls" {
        src.url.clone()
    } else {
        src.url_for(max_height)
    }
}

async fn with_browser_session(
    src: &mut Source,
    max_height: Option<u32>,
    domains: Option<&[String]>,
) -> Arc<ScopedCookies> {
    let mut store = ScopedCookies {
        domains: domains.map(<[String]>::to_vec),
        ..Default::default()
    };
    let url = fetched_url(src, max_height).unwrap_or_default();
    let trusted = reach_for(&url, domains) == Ok(Reach::Credentials);
    let given = src
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        .map(|(_, v)| v.clone());
    src.headers.retain(|k, _| !k.eq_ignore_ascii_case("cookie"));
    if !trusted {
        keep_anonymous_headers(&mut src.headers);
    }
    let host = super::host_of(&url);
    if trusted {
        store.fixed = given.zip(host.clone()).map(|(value, host)| (host, value));
    }
    let Some(b) = super::browser::current() else {
        return Arc::new(store);
    };
    if store.fixed.is_none() {
        let mut hosts: Vec<String> = Vec::new();
        for u in [Some(url.as_str()), src.audio.as_deref()]
            .into_iter()
            .flatten()
        {
            if reach_for(u, domains) == Ok(Reach::Credentials) {
                hosts.extend(super::host_of(u));
            }
        }
        hosts.dedup();
        for host in hosts {
            for c in b.cookies_for(&host).await.unwrap_or_default() {
                if let (Some(name), Some(value)) = (c["name"].as_str(), c["value"].as_str()) {
                    store.jar.push(JarCookie {
                        name: name.to_string(),
                        value: value.to_string(),
                        domain: c["domain"]
                            .as_str()
                            .unwrap_or_default()
                            .trim_start_matches('.')
                            .to_ascii_lowercase(),
                        secure: c["secure"].as_bool().unwrap_or(false),
                    });
                }
            }
        }
    }
    if !src
        .headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("user-agent"))
    {
        if let Some(ua) = b.user_agent().await {
            src.headers.insert("User-Agent".into(), ua);
        }
    }
    Arc::new(store)
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
    let mut target = p.path.clone();
    let domains = job.origin.as_ref().map(|o| o.domains.clone());
    let domains = domains.as_deref();
    for attempt in 0..2 {
        if node.is_lazy() || attempt == 1 {
            let key = node.key.clone().ok_or("item has no source")?;
            let o = job
                .origin
                .clone()
                .ok_or("this tree has no connector to resolve items")?;
            let fresh = tokio::select! {
                r = super::connector::resolve_item(&o.connector, o.dir.as_deref(), &o.page, &key) => r?,
                _ = job.cancel.cancelled() => return Err("cancelled".into()),
            };
            if fresh.drm {
                return Ok("drm");
            }
            let mut src = fresh.source.ok_or("resolveItem returned no source")?;
            let old = node.source.clone().unwrap_or_default();
            src.filename = src.filename.or(old.filename);
            src.convert = src.convert.or(old.convert);
            if !hands_off(Some(&src)) {
                let title = Some(fresh.title.as_str())
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or(&p.node.title);
                let wanted = p.path.with_file_name(final_name(p, title, &src));
                if wanted != target {
                    let claimed = claim(&wanted, &p.who, job.origin.is_some());
                    if claimed != target {
                        target = claimed;
                        job.update(idx, |it| it.path = target.display().to_string());
                        if exists_with_content(&target) {
                            return Ok("skipped");
                        }
                    }
                }
            }
            node.source = Some(src);
        }
        let mut src = node.source.clone().ok_or("item has no source")?;
        check_source(&src, domains)?;
        let cookies = if matches!(src.format.as_str(), "text" | "link" | "ytdlp" | "dash") {
            Arc::default()
        } else {
            with_browser_session(&mut src, max_height, domains).await
        };
        match fetch(job, idx, &target, src, max_height, cookies).await {
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

async fn download_track(
    job: &Arc<Job>,
    idx: usize,
    client: &reqwest::Client,
    url: &str,
    target: &Path,
    headers: reqwest::header::HeaderMap,
    before: u64,
) -> Result<u64, String> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let j = job.clone();
    let counter = tokio::spawn(async move {
        while let Some(u) = rx.recv().await {
            let u: crate::models::progress::ProgressUpdate = u;
            if let Some(b) = u.downloaded_bytes {
                j.update(idx, |it| it.bytes = before + b);
                j.emit(false);
            }
        }
    });
    let r = stop_on_cancel(job, async {
        crate::core::direct_downloader::download_direct_with_headers(
            client,
            url,
            target,
            tx,
            Some(headers),
            Some(&job.cancel),
        )
        .await
        .map_err(|e| e.to_string())
    })
    .await;
    let _ = counter.await;
    r
}

async fn stop_on_cancel<T>(
    job: &Arc<Job>,
    work: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {
        r = work => r,
        _ = async {
            job.cancel.cancelled().await;
            tokio::time::sleep(Duration::from_secs(3)).await;
        } => Err("cancelled".into()),
    }
}

fn converts_in_place(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
}

async fn settle_video(job: &Arc<Job>, idx: usize, path: &Path, bytes: u64) -> Result<(), String> {
    let size = if converts_in_place(path) {
        stop_on_cancel(job, async {
            Ok(crate::core::ytdlp::finish_direct_video(path.to_path_buf(), bytes, None).await)
        })
        .await?
        .1
    } else {
        bytes
    };
    job.update(idx, |it| it.bytes = size);
    Ok(())
}

async fn fetch(
    job: &Arc<Job>,
    idx: usize,
    path: &Path,
    src: Source,
    max_height: Option<u32>,
    cookies: Arc<ScopedCookies>,
) -> Result<&'static str, String> {
    let domains = job.origin.as_ref().map(|o| o.domains.as_slice());
    match src.format.as_str() {
        "text" => {
            let body = src.content.unwrap_or_default();
            std::fs::write(path, body.as_bytes()).map_err(|e| e.to_string())?;
            job.update(idx, |it| it.bytes = body.len() as u64);
            Ok("done")
        }
        "link" => {
            let url = src.url.unwrap_or_default().replace(['\r', '\n'], "");
            let body = format!("[InternetShortcut]\nURL={url}\n");
            std::fs::write(path, body).map_err(|e| e.to_string())?;
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
            let mut anonymous = src.headers.clone();
            keep_anonymous_headers(&mut anonymous);
            let request_headers = hls_credentials(&src.headers, &url, domains);
            let client = job_client(
                reqwest::Client::builder()
                    .default_headers(header_map(&anonymous))
                    .timeout(Duration::from_secs(120)),
                domains,
                cookies,
            )?;
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
            let out = path.display().to_string();
            let ua = src
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| format!("OmniGet/{}", env!("CARGO_PKG_VERSION")));
            let r = stop_on_cancel(job, async {
                crate::core::hls_downloader::HlsDownloader::with_client(client)
                    .with_request_headers(request_headers)
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
                    .await
                    .map_err(|e| e.to_string())
            })
            .await;
            let _ = counter.await;
            let r = r?;
            settle_video(job, idx, path, r.file_size).await?;
            Ok("done")
        }
        _ => {
            let url = src.url_for(max_height).ok_or("item has no url")?;
            let client = job_client(reqwest::Client::builder(), domains, cookies)?;
            let headers = header_map(&src.headers);
            if let Some(audio) = src.audio.as_deref() {
                if !crate::core::media_processor::check_ffmpeg() {
                    return Err(crate::core::hls_downloader::FFMPEG_NEEDED_TO_JOIN.into());
                }
                let mut audio_headers = src.headers.clone();
                if reach_for(audio, domains) != Ok(Reach::Credentials) {
                    keep_anonymous_headers(&mut audio_headers);
                }
                let video_tmp = PathBuf::from(format!("{}.video.mp4", path.display()));
                let audio_tmp = PathBuf::from(format!("{}.audio.m4a", path.display()));
                let joined = async {
                    let v = download_track(job, idx, &client, &url, &video_tmp, headers, 0).await?;
                    let a = download_track(
                        job,
                        idx,
                        &client,
                        audio,
                        &audio_tmp,
                        header_map(&audio_headers),
                        v,
                    )
                    .await?;
                    crate::core::hls_downloader::join_tracks(&video_tmp, &audio_tmp, path)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok::<u64, String>(v + a)
                }
                .await;
                let _ = std::fs::remove_file(&video_tmp);
                let _ = std::fs::remove_file(&audio_tmp);
                joined?;
                let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                settle_video(job, idx, path, size).await?;
                return Ok("done");
            }
            let srt = src.convert.as_deref() == Some("srt");
            let target = if srt {
                path.with_extension("vtt.part")
            } else {
                path.to_path_buf()
            };
            let n = download_track(job, idx, &client, &url, &target, headers, 0).await?;
            if srt {
                let vtt = std::fs::read_to_string(&target).map_err(|e| e.to_string())?;
                std::fs::write(path, vtt_to_srt(&vtt)).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_file(&target);
            }
            settle_video(job, idx, path, n).await?;
            Ok("done")
        }
    }
}

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

#[cfg(test)]
mod ext_tests {
    use super::*;

    #[test]
    fn a_connector_extension_cannot_carry_a_path() {
        let node = Node::default();
        let unsafe_ext = Source {
            ext: Some("x/../../probe.txt".into()),
            ..Source::default()
        };
        assert!(!ext_for(&node, &unsafe_ext).contains('/'));
        let plain = Source {
            ext: Some(".mp4".into()),
            ..Source::default()
        };
        assert_eq!(ext_for(&node, &plain), "mp4");
    }
}

#[cfg(test)]
mod job_tests {
    use super::*;
    use crate::core::http_fetcher::test_server;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "omniget-gd-{tag}-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn item(title: &str, url: &str) -> Node {
        Node {
            title: title.into(),
            kind: "file".into(),
            source: Some(Source {
                format: "direct".into(),
                url: Some(url.into()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn tree(children: Vec<Node>) -> Node {
        let mut t = Node {
            title: "Course".into(),
            kind: "folder".into(),
            children,
            ..Default::default()
        };
        t.assign_ids("0");
        t
    }

    fn named(mut n: Node, filename: &str) -> Node {
        n.source.as_mut().unwrap().filename = Some(filename.into());
        n
    }

    fn origin(domains: &[&str]) -> Origin {
        Origin {
            connector: "test".into(),
            dir: None,
            page: "https://a.test/".into(),
            domains: domains.iter().map(|d| d.to_string()).collect(),
        }
    }

    fn names(planned: &[Planned]) -> Vec<String> {
        planned
            .iter()
            .map(|p| p.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    async fn finished(id: u64) -> JobState {
        for _ in 0..600 {
            let j = list().into_iter().find(|j| j.id == id).unwrap();
            if j.status != "running" {
                return j;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("job {id} did not finish");
    }

    fn state(done: usize, skipped: usize, handed: usize, failed: usize) -> JobState {
        JobState {
            id: 0,
            title: String::new(),
            out_dir: String::new(),
            status: "running".into(),
            total: 0,
            done,
            skipped,
            drm: 0,
            failed,
            handed_off: handed,
            cancelled: 0,
            bytes: 0,
            started_at: 0,
            items: Vec::new(),
        }
    }

    #[test]
    fn only_a_file_that_keeps_its_name_is_made_compatible() {
        assert!(converts_in_place(Path::new("/c/01 Lesson.mp4")));
        assert!(converts_in_place(Path::new("/c/01 Lesson.MP4")));
        for name in [
            "lesson.webm",
            "lesson.mkv",
            "lesson.mov",
            "slides.pdf",
            "files.zip",
            "captions.srt",
            "track.m4a",
            "page.html",
            "shortcut.url",
            "no_extension",
        ] {
            assert!(!converts_in_place(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn a_job_is_only_finished_when_nothing_failed() {
        assert_eq!(final_status(false, &state(3, 0, 0, 0)), "done");
        assert_eq!(final_status(false, &state(0, 0, 0, 3)), "failed");
        assert_eq!(final_status(false, &state(2, 0, 0, 1)), "partial");
        assert_eq!(final_status(false, &state(0, 1, 0, 1)), "partial");
        assert_eq!(final_status(true, &state(0, 0, 0, 3)), "cancelled");
    }

    #[tokio::test]
    async fn job_with_failures_says_how_many_and_which() {
        let (base, _) = test_server::spawn(|_, r| {
            if r.path.contains("gone") {
                test_server::Reply::new(404, Vec::new())
            } else {
                test_server::serve_bytes(b"some file body", r)
            }
        })
        .await;
        let dir = temp_dir("failures");
        let t = tree(vec![
            item("Good", &format!("{base}/good.pdf")),
            item("Broken", &format!("{base}/gone.pdf")),
        ]);
        let j = finished(start(t, None, dir.clone(), None).unwrap().id).await;
        assert_eq!(j.status, "partial");
        assert_eq!((j.done, j.failed), (1, 1));
        let failed: Vec<&str> = j
            .items
            .iter()
            .filter(|i| i.status == "failed")
            .map(|i| i.title.as_str())
            .collect();
        assert_eq!(failed, vec!["Broken"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn job_where_every_item_failed_is_not_finished() {
        let (base, _) = test_server::spawn(|_, _| test_server::Reply::new(404, Vec::new())).await;
        let dir = temp_dir("allfail");
        let t = tree(vec![
            item("A", &format!("{base}/a.pdf")),
            item("B", &format!("{base}/b.pdf")),
        ]);
        let j = finished(start(t, None, dir.clone(), None).unwrap().id).await;
        assert_eq!(j.status, "failed");
        assert_eq!(j.failed, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_name_files_are_numbered_not_skipped() {
        let dir = temp_dir("names");
        let t = tree(vec![
            named(item("a", "https://a.test/1"), "Notes.pdf"),
            named(item("b", "https://a.test/2"), "notes.PDF"),
            named(item("c", "https://a.test/3"), "Notes.pdf"),
        ]);
        let mut planned = Vec::new();
        plan(&t, &dir, Some(&origin(&["a.test"])), &mut planned);
        assert_eq!(
            names(&planned),
            vec!["Notes.pdf", "notes (2).PDF", "Notes (3).pdf"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retrying_one_item_keeps_its_number() {
        let dir = temp_dir("retry");
        let t = tree(vec![
            named(item("a", "https://a.test/1"), "Notes.pdf"),
            named(item("b", "https://a.test/2"), "Notes.pdf"),
        ]);
        let o = origin(&["a.test"]);
        let mut first = Vec::new();
        plan(&t, &dir, Some(&o), &mut first);
        std::fs::write(&first[0].path, b"first").unwrap();
        let mut again = Vec::new();
        plan(&t, &dir, Some(&o), &mut again);
        assert_eq!(names(&again), vec!["Notes.pdf", "Notes (2).pdf"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_connector_rerun_still_skips_what_it_already_saved() {
        let (base, log) = test_server::spawn(|_, r| test_server::serve_bytes(b"body", r)).await;
        let dir = temp_dir("resume");
        let conn = temp_connector(&["127.0.0.1"], "function resolve(){return {}}");
        let mut t = tree(vec![named(item("a", &format!("{base}/a.pdf")), "A.pdf")]);
        t.connector = Some("test-conn".into());
        t.connector_dir = Some(conn.display().to_string());
        t.origin = Some(format!("{base}/"));
        let j = finished(start(t.clone(), None, dir.clone(), None).unwrap().id).await;
        assert_eq!((j.status.as_str(), j.done), ("done", 1));
        let seen = log.lock().unwrap().len();
        let j = finished(start(t, None, dir.clone(), None).unwrap().id).await;
        assert_eq!((j.status.as_str(), j.skipped), ("done", 1));
        assert_eq!(log.lock().unwrap().len(), seen);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&conn);
    }

    #[tokio::test]
    async fn another_video_with_the_same_title_is_numbered_when_there_is_no_connector() {
        let (base, _) = test_server::spawn(|_, r| test_server::serve_bytes(b"body", r)).await;
        let dir = temp_dir("sniffed");
        for u in ["one.mp4", "two.mp4"] {
            let t = tree(vec![item("Page", &format!("{base}/{u}"))]);
            let j = finished(start(t, None, dir.clone(), None).unwrap().id).await;
            assert_eq!(j.done, 1, "{u}: {:?}", j.items);
        }
        assert!(dir.join("Course/01 - Page.mp4").exists());
        assert!(dir.join("Course/01 - Page (2).mp4").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn attachments_keep_their_extension() {
        let node = Node::default();
        let by_url = Source {
            format: "direct".into(),
            url: Some("https://a.test/f/report.PDF?sig=1".into()),
            filename: Some("Material".into()),
            ..Default::default()
        };
        assert_eq!(file_name(1, "x", &node, &by_url), "Material.pdf");
        let by_ext = Source {
            ext: Some("zip".into()),
            filename: Some("Archive".into()),
            ..Default::default()
        };
        assert_eq!(file_name(1, "x", &node, &by_ext), "Archive.zip");
        let dotted = Source {
            url: Some("https://a.test/f.pdf".into()),
            filename: Some("Aula 1.5".into()),
            ..Default::default()
        };
        assert_eq!(file_name(1, "x", &node, &dotted), "Aula 1.5.pdf");
        let script = Source {
            url: Some("https://a.test/get.php?id=1".into()),
            filename: Some("Material".into()),
            ..Default::default()
        };
        assert_eq!(file_name(1, "x", &node, &script), "Material");
        let long = format!("{}.pdf", "n".repeat(200));
        let clipped = clip_file(&long);
        assert!(clipped.ends_with(".pdf"));
        assert_eq!(clipped.chars().count(), 120);
    }

    #[test]
    fn a_shortcut_is_never_saved_as_the_file_it_points_at() {
        let src = Source {
            format: "link".into(),
            url: Some("https://a.test/book.pdf".into()),
            filename: Some("Book.pdf".into()),
            ..Default::default()
        };
        assert_eq!(file_name(1, "x", &Node::default(), &src), "Book.pdf.url");
    }

    fn temp_connector(domains: &[&str], script: &str) -> PathBuf {
        let d = temp_dir("conn");
        let manifest = json!({
            "id": "test-conn", "name": "Test", "domains": domains,
            "match": ["https://example.test/*"],
        });
        std::fs::write(d.join("manifest.json"), manifest.to_string()).unwrap();
        std::fs::write(d.join("connector.js"), script).unwrap();
        d
    }

    fn lazy(title: &str, key: &str, filename: Option<&str>) -> Node {
        Node {
            title: title.into(),
            kind: "file".into(),
            key: Some(key.into()),
            source: Some(Source {
                format: "lazy".into(),
                filename: filename.map(String::from),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn resolve_item_name_and_extension_are_honoured() {
        let (base, _) = test_server::spawn(|_, r| test_server::serve_bytes(b"body", r)).await;
        let script = format!(
            r#"function resolve(){{return {{}}}}
            function resolveItem(key, ctx) {{
              if (key === "ext") return {{ format: "direct", url: "{base}/x", ext: "pdf" }};
              if (key === "url") return {{ format: "direct", url: "{base}/archive.zip" }};
              return {{ format: "direct", url: "{base}/y", filename: "Real name.epub" }};
            }}"#
        );
        let conn = temp_connector(&["127.0.0.1"], &script);
        let dir = temp_dir("resolved");
        let mut t = tree(vec![
            lazy("Material", "ext", Some("Material")),
            lazy("Material", "ext2", Some("Material")),
            lazy("Archive", "url", None),
        ]);
        t.children.push(lazy("Book", "name", None));
        t.assign_ids("0");
        t.connector = Some("test-conn".into());
        t.connector_dir = Some(conn.display().to_string());
        t.origin = Some(format!("{base}/"));
        let script2 = format!(
            r#"function resolve(){{return {{}}}}
            function resolveItem(key, ctx) {{
              if (key === "ext" || key === "ext2") return {{ format: "direct", url: "{base}/x", ext: "pdf" }};
              if (key === "url") return {{ format: "direct", url: "{base}/archive.zip" }};
              return {{ format: "direct", url: "{base}/y", filename: "Real name.epub" }};
            }}"#
        );
        std::fs::write(conn.join("connector.js"), script2).unwrap();
        let j = finished(start(t, None, dir.clone(), None).unwrap().id).await;
        assert_eq!(j.status, "done", "{:?}", j.items);
        let root = dir.join("Course");
        for f in [
            "Material.pdf",
            "Material (2).pdf",
            "03 - Archive.zip",
            "Real name.epub",
        ] {
            assert!(root.join(f).exists(), "{f} missing: {:?}", j.items);
        }
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&conn);
    }

    #[tokio::test]
    async fn cancelling_leaves_no_item_waiting_forever() {
        let (base, _) = test_server::spawn(|_, r| {
            let mut reply = test_server::serve_bytes(&[7u8; 4096], r);
            reply.cut_after = Some(1024);
            reply.stall = true;
            reply
        })
        .await;
        let dir = temp_dir("cancel");
        let t = tree(
            (0..8)
                .map(|i| item(&format!("I{i}"), &format!("{base}/{i}.bin")))
                .collect(),
        );
        let id = start(t, None, dir.clone(), None).unwrap().id;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(cancel(id));
        let j = finished(id).await;
        assert_eq!(j.status, "cancelled");
        assert!(
            j.items
                .iter()
                .all(|i| !matches!(i.status.as_str(), "queued" | "active" | "failed")),
            "{:?}",
            j.items
        );
        assert_eq!(j.cancelled, 8);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_connectors_reach_their_media_hosts() {
        use Reach::{Anonymous, Credentials};
        let cases: &[(&[&str], &str, Reach)] = &[
            (
                &[
                    "branas.io",
                    "launchlab.app",
                    "launchlms.com.br",
                    "launchlms.io",
                ],
                "https://b-vz-1.tv.pandavideo.com.br/x/playlist.m3u8",
                Anonymous,
            ),
            (
                &["branas.io", "launchlab.app"],
                "https://x.launchlab.app/api/files/1/",
                Credentials,
            ),
            (
                &["bunkr.cr", "scdn.st", "cdn.cr"],
                "https://media-files9.scdn.st/a.mp4",
                Credentials,
            ),
            (
                &["bunkr.cr", "scdn.st", "cdn.cr"],
                "https://other-host.example/a.mp4?token=1",
                Anonymous,
            ),
            (
                &["courseflix.net"],
                "https://courseflix.net/f.pdf",
                Credentials,
            ),
            (
                &["courseflix.net"],
                "https://storage.example.com/l.mp4?sig=1",
                Anonymous,
            ),
            (
                &["entregadigital.app.br", "pandavideo.com.br"],
                "https://b-vz-1.tv.pandavideo.com.br/x.m3u8",
                Credentials,
            ),
            (
                &["entregadigital.app.br", "pandavideo.com.br"],
                "https://s3.amazonaws.com/a.pdf",
                Anonymous,
            ),
            (
                &[
                    "fisicacomrenatobrito.com.br",
                    "videotecaead.com.br",
                    "azionedge.net",
                ],
                "https://x.azionedge.net/master.m3u8",
                Credentials,
            ),
            (
                &["fisicacomrenatobrito.com.br"],
                "https://www.youtube.com/watch?v=1",
                Anonymous,
            ),
            (
                &[
                    "tle-eliminators.com",
                    "cloudfront.net",
                    "digitaloceanspaces.com",
                ],
                "https://d1.cloudfront.net/s.pdf",
                Credentials,
            ),
            (
                &["tle-eliminators.com"],
                "https://codeforces.com/contest/1",
                Anonymous,
            ),
            (
                &["udemy.com", "udemycdn.com"],
                "https://mp4-c.udemycdn.com/2024/a.mp4",
                Credentials,
            ),
            (
                &["udemy.com", "udemycdn.com"],
                "https://att-c.s3.amazonaws.com/a.pdf",
                Anonymous,
            ),
        ];
        for (domains, url, want) in cases {
            let domains: Vec<String> = domains.iter().map(|d| d.to_string()).collect();
            assert_eq!(reach_for(url, Some(&domains)), Ok(*want), "{url}");
        }
    }

    #[test]
    fn a_tree_url_on_an_opted_out_site_is_refused() {
        let domains = vec!["kiwify.com.br".to_string()];
        for url in ["https://kiwify.com.br/x", "https://CDN.Kiwify.app/x.mp4"] {
            assert_eq!(
                reach_for(url, None),
                Err(platform_optout::OPTED_OUT_ERROR.to_string())
            );
            assert_eq!(
                reach_for(url, Some(&domains)),
                Err(platform_optout::OPTED_OUT_ERROR.to_string())
            );
        }
        let hidden = Source {
            url: Some("https://ok.test/a.mp4".into()),
            renditions: vec![crate::core::global_dl::Rendition {
                height: 1080,
                url: "https://kiwify.com/a.mp4".into(),
            }],
            ..Default::default()
        };
        assert!(check_source(&hidden, None).is_err());
        let audio = Source {
            url: Some("https://ok.test/a.mp4".into()),
            audio: Some("https://kiwify.com/a.m4a".into()),
            ..Default::default()
        };
        assert!(check_source(&audio, None).is_err());
    }

    #[tokio::test]
    async fn a_redirect_to_an_opted_out_site_is_not_followed() {
        let (base, _) = test_server::spawn(|_, _| {
            test_server::Reply::new(302, Vec::new()).header("Location", "https://kiwify.com/a.mp4")
        })
        .await;
        let client = job_client(reqwest::Client::builder(), None, Arc::default()).unwrap();
        let err = client
            .get(format!("{base}/a.mp4"))
            .send()
            .await
            .unwrap_err();
        assert!(
            format!("{err:?}").contains(platform_optout::OPTED_OUT_ERROR),
            "{err:?}"
        );
    }

    #[test]
    fn connectors_cannot_reach_other_schemes_or_local_addresses() {
        let domains = vec!["a.test".to_string()];
        for url in [
            "file:///etc/passwd",
            "http://127.0.0.1:8080/admin",
            "http://localhost/x",
            "http://192.168.0.1/x",
            "http://[::1]/x",
            "http://printer.local/x",
        ] {
            assert!(reach_for(url, Some(&domains)).is_err(), "{url}");
        }
    }

    #[test]
    fn a_bare_tld_in_domains_declares_nothing() {
        let domains = vec!["com".to_string(), String::new(), "*.".to_string()];
        assert_eq!(
            reach_for("https://bank.com/", Some(&domains)),
            Ok(Reach::Anonymous)
        );
        assert_eq!(
            reach_for("https://bank.com./", Some(&domains)),
            Ok(Reach::Anonymous)
        );
    }

    #[test]
    fn credentials_are_dropped_for_an_undeclared_host() {
        let mut h = BTreeMap::new();
        for k in [
            "Cookie",
            "Authorization",
            "X-Udemy-Authorization",
            "Referer",
            "Origin",
        ] {
            h.insert(k.to_string(), "v".to_string());
        }
        keep_anonymous_headers(&mut h);
        let kept: Vec<&str> = h.keys().map(String::as_str).collect();
        assert_eq!(kept, vec!["Origin", "Referer"]);
    }

    fn jar_cookie(domain: &str, secure: bool) -> JarCookie {
        JarCookie {
            name: "s".into(),
            value: "1".into(),
            domain: domain.into(),
            secure,
        }
    }

    #[test]
    fn browser_cookies_only_go_to_declared_hosts_they_belong_to() {
        let store = ScopedCookies {
            domains: Some(vec!["udemy.com".into()]),
            jar: vec![jar_cookie("udemy.com", false)],
            fixed: None,
        };
        let at = |u: &str| store.header_for(&url::Url::parse(u).unwrap());
        assert_eq!(at("https://www.udemy.com/x").as_deref(), Some("s=1"));
        assert_eq!(at("https://evil.test/x"), None);
        assert_eq!(at("https://udemy.com.evil.test/x"), None);
        let secure = ScopedCookies {
            domains: None,
            jar: vec![jar_cookie("a.test", true)],
            fixed: None,
        };
        let at = |u: &str| secure.header_for(&url::Url::parse(u).unwrap());
        assert_eq!(at("https://a.test/x").as_deref(), Some("s=1"));
        assert_eq!(at("http://a.test/x"), None);
    }

    #[tokio::test]
    async fn hls_cookies_do_not_follow_segments_to_another_host() {
        let (cdn, cdn_log) =
            test_server::spawn(|_, _| test_server::Reply::new(200, vec![0x47; 188])).await;
        let cdn_host = cdn.replace("127.0.0.1", "localhost");
        let playlist = format!(
            "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\n{cdn_host}/seg.ts\n#EXT-X-ENDLIST\n"
        );
        let (site, site_log) = test_server::spawn(move |_, _| {
            test_server::Reply::new(200, playlist.clone().into_bytes())
        })
        .await;
        let dir = temp_dir("hlscookie");
        let store = ScopedCookies {
            domains: Some(vec!["127.0.0.1".into()]),
            jar: Vec::new(),
            fixed: Some(("127.0.0.1".into(), "s=secret".into())),
        };
        let domains = vec!["127.0.0.1".to_string()];
        let client =
            job_client(reqwest::Client::builder(), Some(&domains), Arc::new(store)).unwrap();
        let out = dir.join("o.ts");
        crate::core::hls_downloader::HlsDownloader::with_client(client)
            .download(
                &format!("{site}/p.m3u8"),
                out.to_str().unwrap(),
                "",
                None,
                CancellationToken::new(),
                2,
                1,
            )
            .await
            .unwrap();
        assert_eq!(
            site_log.lock().unwrap()[0].header("cookie"),
            Some("s=secret")
        );
        let segs = cdn_log.lock().unwrap().clone();
        assert!(!segs.is_empty());
        assert!(
            segs.iter().all(|r| r.header("cookie").is_none()),
            "{segs:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn hls_credentials_only_reach_declared_hosts() {
        let (cdn, cdn_log) =
            test_server::spawn(|_, _| test_server::Reply::new(200, vec![0x47; 188])).await;
        let cdn_host = cdn.replace("127.0.0.1", "localhost");
        let playlist = format!(
            "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\n{cdn_host}/seg.ts\n#EXT-X-ENDLIST\n"
        );
        let (site, site_log) = test_server::spawn(move |_, _| {
            test_server::Reply::new(200, playlist.clone().into_bytes())
        })
        .await;
        let dir = temp_dir("hlsauth");
        let mut headers = BTreeMap::new();
        for k in ["Authorization", "X-Udemy-Authorization"] {
            headers.insert(k.to_string(), "Bearer t".to_string());
        }
        headers.insert("Referer".to_string(), "https://site/".to_string());
        let domains = vec!["127.0.0.1".to_string()];
        let url = format!("{site}/p.m3u8");
        let request_headers = hls_credentials(&headers, &url, Some(&domains));
        let client = job_client(
            reqwest::Client::builder(),
            Some(&domains),
            Arc::new(ScopedCookies {
                domains: Some(domains.clone()),
                jar: Vec::new(),
                fixed: None,
            }),
        )
        .unwrap();
        let out = dir.join("o.ts");
        crate::core::hls_downloader::HlsDownloader::with_client(client)
            .with_request_headers(request_headers)
            .download(
                &url,
                out.to_str().unwrap(),
                "",
                None,
                CancellationToken::new(),
                2,
                1,
            )
            .await
            .unwrap();
        let first = site_log.lock().unwrap()[0].clone();
        assert_eq!(first.header("authorization"), Some("Bearer t"));
        assert_eq!(first.header("x-udemy-authorization"), Some("Bearer t"));
        let segs = cdn_log.lock().unwrap().clone();
        assert!(!segs.is_empty());
        assert!(
            segs.iter().all(|r| r.header("authorization").is_none()
                && r.header("x-udemy-authorization").is_none()),
            "{segs:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hls_without_a_connector_only_trusts_the_playlist_host() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".to_string(), "Bearer t".to_string());
        let h = hls_credentials(&headers, "https://a.test/p.m3u8", None);
        assert!(h("https://a.test/s.ts").contains_key("authorization"));
        assert!(h("https://cdn.test/s.ts").is_empty());
    }

    #[cfg_attr(
        windows,
        ignore = "needs *.localhost to resolve, which Windows does not do"
    )]
    #[tokio::test]
    async fn undeclared_cdn_still_downloads_without_the_connectors_credentials() {
        let (base, log) = test_server::spawn(|_, r| test_server::serve_bytes(b"body", r)).await;
        let cdn = base.replace("127.0.0.1", "cdn.localhost");
        let domains = vec!["localhost".to_string()];
        let mut src = Source {
            format: "direct".into(),
            url: Some(format!("{base}/a.pdf")),
            ..Default::default()
        };
        src.headers.insert("Cookie".into(), "s=secret".into());
        src.headers
            .insert("Authorization".into(), "Bearer t".into());
        src.headers.insert("Referer".into(), "https://site/".into());
        let domains_other = vec!["declared.test".to_string()];
        assert!(reach_for(&src.url.clone().unwrap(), Some(&domains_other)).is_err());
        let store = with_browser_session(&mut src, None, Some(&domains_other)).await;
        assert!(store.fixed.is_none());
        let kept: Vec<&str> = src.headers.keys().map(String::as_str).collect();
        assert_eq!(kept, vec!["Referer"]);
        let client = job_client(reqwest::Client::builder(), Some(&domains), store).unwrap();
        client
            .get(format!("{cdn}/a.pdf"))
            .headers(header_map(&src.headers))
            .send()
            .await
            .unwrap();
        let reqs = log.lock().unwrap().clone();
        assert_eq!(reqs[0].header("referer"), Some("https://site/"));
        assert_eq!(reqs[0].header("cookie"), None);
        assert_eq!(reqs[0].header("authorization"), None);
    }

    #[tokio::test]
    async fn a_connector_tree_needs_its_connector_installed() {
        let mut t = tree(vec![item("a", "https://a.test/1.pdf")]);
        t.connector = Some("not-installed-connector".into());
        assert!(start(t, None, temp_dir("gone"), None).is_err());
    }
}
