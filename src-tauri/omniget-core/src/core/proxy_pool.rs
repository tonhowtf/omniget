use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const MAX_PROXIES: usize = 500;
pub const SCHEMES: &[&str] = &["http", "https", "socks4", "socks4a", "socks5", "socks5h"];
const BASE_COOLDOWN: Duration = Duration::from_secs(30);
const MAX_COOLDOWN: Duration = Duration::from_secs(600);
const CHECK_TIMEOUT: Duration = Duration::from_secs(8);
pub const CHECK_BATCH: usize = 24;
const CHECK_CONCURRENCY: usize = 8;
const TRACE_URL: &str = "https://www.cloudflare.com/cdn-cgi/trace";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProxyError {
    #[error("empty proxy address")]
    Empty,
    #[error("unknown proxy type \"{0}\"; use http, https, socks4, socks4a, socks5 or socks5h")]
    Scheme(String),
    #[error("a proxy needs a host and a port, like socks5h://127.0.0.1:7890")]
    HostPort,
    #[error("the pool holds at most {MAX_PROXIES} proxies")]
    Full,
    #[error("cannot save the proxy pool: {0}")]
    Save(String),
    #[error("cannot read {path}: {message}")]
    Read { path: PathBuf, message: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    #[default]
    Sticky,
    RoundRobin,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub rotation: Rotation,
    pub proxies: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Health {
    fails: u32,
    cooldown_until: Option<Instant>,
    checked: Option<Checked>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checked {
    pub ok: bool,
    pub ms: Option<u64>,
    pub ip: Option<String>,
    pub country: Option<String>,
    pub status: Option<u16>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyView {
    pub id: String,
    pub proxy: String,
    pub current: bool,
    pub fails: u32,
    pub resting_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<Checked>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolView {
    pub enabled: bool,
    pub rotation: Rotation,
    pub proxies: Vec<ProxyView>,
    pub total: usize,
    pub truncated: bool,
    pub available: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Rejected {
    pub input: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Added {
    pub added: usize,
    pub duplicates: usize,
    pub rejected: Vec<Rejected>,
}

pub fn parse(raw: &str) -> Result<String, ProxyError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ProxyError::Empty);
    }
    let (scheme, rest) = match raw.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("http".to_string(), raw),
    };
    if !SCHEMES.contains(&scheme.as_str()) {
        return Err(ProxyError::Scheme(scheme));
    }
    let scheme = if scheme == "socks5" {
        "socks5h"
    } else {
        scheme.as_str()
    };
    let rest = rest.trim_end_matches('/');
    let (auth, host_port) = match rest.rsplit_once('@') {
        Some((auth, host_port)) => (Some(auth.to_string()), host_port.to_string()),
        None => {
            let parts: Vec<&str> = rest.split(':').collect();
            if parts.len() == 4 && parts[1].parse::<u16>().is_ok() {
                (
                    Some(format!("{}:{}", parts[2], parts[3])),
                    format!("{}:{}", parts[0], parts[1]),
                )
            } else {
                (None, rest.to_string())
            }
        }
    };
    let explicit_port = host_port
        .rsplit_once(':')
        .is_some_and(|(host, port)| !host.is_empty() && port.parse::<u16>().is_ok_and(|p| p > 0));
    if !explicit_port {
        return Err(ProxyError::HostPort);
    }
    let mut url =
        url::Url::parse(&format!("http://{host_port}")).map_err(|_| ProxyError::HostPort)?;
    if url.host_str().is_none_or(str::is_empty) || url.path() != "/" || url.query().is_some() {
        return Err(ProxyError::HostPort);
    }
    if let Some(auth) = auth {
        let (user, password) = auth.split_once(':').unwrap_or((auth.as_str(), ""));
        url.set_username(user).map_err(|_| ProxyError::HostPort)?;
        if !password.is_empty() {
            url.set_password(Some(password))
                .map_err(|_| ProxyError::HostPort)?;
        }
    }
    let port = host_port.rsplit_once(':').map_or("", |(_, port)| port);
    let host = url.host_str().unwrap_or_default();
    let auth = match (url.username(), url.password()) {
        ("", _) => String::new(),
        (user, None) => format!("{user}@"),
        (user, Some(password)) => format!("{user}:{password}@"),
    };
    Ok(format!("{scheme}://{auth}{host}:{port}"))
}

pub fn redact(proxy: &str) -> String {
    let Some((scheme, rest)) = proxy.split_once("://") else {
        return proxy.to_string();
    };
    match rest.rsplit_once('@') {
        Some((auth, host)) if auth.contains(':') => {
            let user = auth.split(':').next().unwrap_or_default();
            format!("{scheme}://{user}:***@{host}")
        }
        _ => proxy.to_string(),
    }
}

fn redact_input(raw: &str) -> String {
    let raw = raw.trim();
    let shown = match raw.rsplit_once('@') {
        Some(_) => redact(&if raw.contains("://") {
            raw.to_string()
        } else {
            format!("http://{raw}")
        }),
        None => {
            let parts: Vec<&str> = raw.split(':').collect();
            if parts.len() >= 4 {
                format!("{}:***", parts[..parts.len() - 1].join(":"))
            } else {
                raw.to_string()
            }
        }
    };
    shown.chars().take(120).collect()
}

pub fn id_of(proxy: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("px_{}", hex::encode(&Sha256::digest(proxy.as_bytes())[..4]))
}

fn cooldown(fails: u32) -> Duration {
    BASE_COOLDOWN
        .saturating_mul(1u32 << fails.saturating_sub(1).min(8))
        .min(MAX_COOLDOWN)
}

#[derive(Debug, Default)]
pub struct Pool {
    config: Config,
    health: Vec<Health>,
    cursor: usize,
}

impl Pool {
    pub fn new(config: Config) -> Self {
        let health = vec![Health::default(); config.proxies.len()];
        Self {
            config,
            health,
            cursor: 0,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn active(&self) -> bool {
        self.config.enabled && !self.config.proxies.is_empty()
    }

    fn resting(&self, index: usize, now: Instant) -> bool {
        self.health[index]
            .cooldown_until
            .is_some_and(|until| until > now)
    }

    fn next_available(&self, from: usize, now: Instant) -> usize {
        let n = self.config.proxies.len();
        (0..n)
            .map(|step| (from + step) % n)
            .find(|i| !self.resting(*i, now))
            .or_else(|| (0..n).min_by_key(|i| self.health[*i].cooldown_until))
            .unwrap_or(0)
    }

    pub fn pick(&mut self, now: Instant) -> Option<&str> {
        if !self.active() {
            return None;
        }
        let chosen = self.next_available(self.cursor, now);
        self.cursor = match self.config.rotation {
            Rotation::Sticky => chosen,
            Rotation::RoundRobin => (chosen + 1) % self.config.proxies.len(),
        };
        Some(&self.config.proxies[chosen])
    }

    pub fn report(&mut self, proxy: &str, ok: bool, now: Instant) {
        let Some(index) = self.config.proxies.iter().position(|p| p == proxy) else {
            return;
        };
        let health = &mut self.health[index];
        if ok {
            health.fails = 0;
            health.cooldown_until = None;
            return;
        }
        health.fails = health.fails.saturating_add(1);
        health.cooldown_until = Some(now + cooldown(health.fails));
        if self.config.rotation == Rotation::Sticky && self.cursor == index {
            self.cursor = self.next_available(index + 1, now);
        }
    }

    pub fn rotate(&mut self, now: Instant) -> Option<&str> {
        if !self.active() {
            return None;
        }
        self.cursor = self.next_available(self.cursor + 1, now);
        Some(&self.config.proxies[self.cursor])
    }

    pub fn add(&mut self, inputs: &[String]) -> Added {
        let mut out = Added::default();
        for input in inputs {
            let line = input.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let reject = |error: ProxyError| Rejected {
                input: redact_input(line),
                reason: error.to_string(),
            };
            match parse(line) {
                Ok(proxy) if self.config.proxies.contains(&proxy) => out.duplicates += 1,
                Ok(_) if self.config.proxies.len() >= MAX_PROXIES => {
                    out.rejected.push(reject(ProxyError::Full))
                }
                Ok(proxy) => {
                    self.config.proxies.push(proxy);
                    self.health.push(Health::default());
                    out.added += 1;
                }
                Err(error) => out.rejected.push(reject(error)),
            }
        }
        out
    }

    pub fn remove(&mut self, ids: &[String]) -> usize {
        let before = self.config.proxies.len();
        let current = self.config.proxies.get(self.cursor).cloned();
        let keep: Vec<bool> = self
            .config
            .proxies
            .iter()
            .map(|proxy| !ids.contains(&id_of(proxy)))
            .collect();
        let mut flags = keep.iter();
        self.config
            .proxies
            .retain(|_| flags.next().copied().unwrap_or(true));
        let mut flags = keep.iter();
        self.health
            .retain(|_| flags.next().copied().unwrap_or(true));
        self.cursor = current
            .and_then(|proxy| self.config.proxies.iter().position(|p| *p == proxy))
            .unwrap_or(0);
        before - self.config.proxies.len()
    }

    pub fn failing_ids(&self) -> Vec<String> {
        self.config
            .proxies
            .iter()
            .zip(&self.health)
            .filter(|(_, health)| health.checked.as_ref().is_some_and(|c| !c.ok))
            .map(|(proxy, _)| id_of(proxy))
            .collect()
    }

    pub fn configure(&mut self, enabled: Option<bool>, rotation: Option<Rotation>) {
        if let Some(enabled) = enabled {
            self.config.enabled = enabled;
        }
        if let Some(rotation) = rotation {
            self.config.rotation = rotation;
        }
    }

    fn record_check(&mut self, proxy: &str, checked: Checked, now: Instant) {
        let ok = checked.ok;
        if let Some(index) = self.config.proxies.iter().position(|p| p == proxy) {
            self.health[index].checked = Some(checked);
        }
        self.report(proxy, ok, now);
    }

    pub fn view(&self, now: Instant, limit: usize) -> PoolView {
        let total = self.config.proxies.len();
        let available = (0..total).filter(|i| !self.resting(*i, now)).count();
        let proxies = self
            .config
            .proxies
            .iter()
            .zip(&self.health)
            .enumerate()
            .take(limit)
            .map(|(index, (proxy, health))| ProxyView {
                id: id_of(proxy),
                proxy: redact(proxy),
                current: self.active() && index == self.cursor,
                fails: health.fails,
                resting_secs: health
                    .cooldown_until
                    .map_or(0, |until| until.saturating_duration_since(now).as_secs()),
                checked: health.checked.clone(),
            })
            .collect();
        let reason = if total == 0 {
            Some("the pool is empty; add proxies with op=add or op=import, or in Settings, Network")
        } else if !self.config.enabled {
            Some("the pool is off, so the app uses the single proxy from Settings or a direct connection; turn it on with op=set enabled=true")
        } else if available == 0 {
            Some("every proxy is resting after failures; the one that rests the least is still used, run op=check or add working proxies")
        } else {
            None
        };
        PoolView {
            enabled: self.config.enabled,
            rotation: self.config.rotation,
            proxies,
            total,
            truncated: total > limit,
            available,
            reason,
        }
    }
}

fn store_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|dir| dir.join("network").join("proxies.json"))
}

fn load() -> Pool {
    let config = store_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Config>(&bytes).ok())
        .unwrap_or_default();
    Pool::new(config)
}

fn save(config: &Config) -> Result<(), ProxyError> {
    let failed = |e: std::io::Error| ProxyError::Save(e.to_string());
    let path = store_path().ok_or_else(|| ProxyError::Save("no data folder".into()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    let bytes = serde_json::to_vec_pretty(config).map_err(|e| ProxyError::Save(e.to_string()))?;
    let fresh = path.with_extension("json.new");
    std::fs::write(&fresh, bytes).map_err(failed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o600)).map_err(failed)?;
    }
    std::fs::rename(&fresh, &path).map_err(failed)
}

static POOL: LazyLock<Mutex<Pool>> = LazyLock::new(|| Mutex::new(load()));

fn with<T>(work: impl FnOnce(&mut Pool) -> T) -> T {
    let mut guard = POOL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    work(&mut guard)
}

fn change<T>(work: impl FnOnce(&mut Pool) -> T) -> Result<T, ProxyError> {
    let (out, config) = with(|pool| {
        let before = pool.config.clone();
        let out = work(pool);
        (out, (pool.config != before).then(|| pool.config.clone()))
    });
    if let Some(config) = config {
        save(&config)?;
    }
    Ok(out)
}

pub fn active() -> bool {
    with(|pool| pool.active())
}

pub fn pick() -> Option<String> {
    with(|pool| pool.pick(Instant::now()).map(str::to_string))
}

pub fn pick_id(id: &str) -> Option<String> {
    with(|pool| pool.config.proxies.iter().find(|p| id_of(p) == id).cloned())
}

pub fn report(proxy: &str, ok: bool) {
    with(|pool| pool.report(proxy, ok, Instant::now()));
}

pub fn rotate() -> Option<String> {
    with(|pool| pool.rotate(Instant::now()).map(redact))
}

pub fn view(limit: usize) -> PoolView {
    with(|pool| pool.view(Instant::now(), limit))
}

pub fn add(inputs: &[String]) -> Result<Added, ProxyError> {
    change(|pool| pool.add(inputs))
}

pub fn import(path: &std::path::Path) -> Result<Added, ProxyError> {
    use std::io::{BufRead, BufReader};
    let unreadable = |e: std::io::Error| ProxyError::Read {
        path: path.to_path_buf(),
        message: e.to_string(),
    };
    let file = std::fs::File::open(path).map_err(unreadable)?;
    let mut lines = Vec::new();
    for line in BufReader::new(file).lines().take(MAX_PROXIES * 4) {
        let line = line.map_err(unreadable)?;
        if line.len() <= 2048 {
            lines.push(line);
        }
    }
    add(&lines)
}

pub fn remove(ids: &[String]) -> Result<usize, ProxyError> {
    change(|pool| pool.remove(ids))
}

pub fn remove_failing() -> Result<usize, ProxyError> {
    change(|pool| {
        let ids = pool.failing_ids();
        pool.remove(&ids)
    })
}

pub fn configure(enabled: Option<bool>, rotation: Option<Rotation>) -> Result<(), ProxyError> {
    change(|pool| pool.configure(enabled, rotation))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Detected {
    pub proxy: String,
    pub app: String,
    pub in_pool: bool,
}

pub async fn detect() -> Vec<Detected> {
    use crate::core::telegram::net::{detect, route::RouteKind};
    let found = detect::detect_all().await;
    let known = with(|pool| pool.config.proxies.clone());
    let mut out: Vec<Detected> = Vec::new();
    for item in found {
        let scheme = match item.route.kind {
            RouteKind::Socks5 => "socks5h",
            RouteKind::Http => "http",
            _ => continue,
        };
        if item.route.username.is_some() {
            continue;
        }
        let Ok(proxy) = parse(&format!(
            "{scheme}://{}:{}",
            item.route.host, item.route.port
        )) else {
            continue;
        };
        if out.iter().all(|seen| seen.proxy != proxy) {
            out.push(Detected {
                in_pool: known.contains(&proxy),
                proxy,
                app: item.hint,
            });
        }
    }
    out
}

pub fn parse_trace(body: &str) -> (Option<String>, Option<String>) {
    let field = |key: &str| {
        body.lines()
            .find_map(|line| line.strip_prefix(key))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    (field("ip="), field("loc="))
}

async fn check_one(proxy: &str, target: Option<&url::Url>) -> Checked {
    let failed = |error: String| Checked {
        error: Some(error),
        ..Checked::default()
    };
    let route = match reqwest::Proxy::all(proxy) {
        Ok(route) => route,
        Err(e) => return failed(e.to_string()),
    };
    let client = match reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(CHECK_TIMEOUT)
        .proxy(route)
        .build()
    {
        Ok(client) => client,
        Err(e) => return failed(e.to_string()),
    };
    let started = Instant::now();
    let request = client.get(target.map_or(TRACE_URL, url::Url::as_str));
    let mut response = match request.send().await {
        Ok(response) => response,
        Err(e) if e.is_timeout() => return failed("no answer in 8 s".into()),
        Err(e) if e.is_connect() => return failed("could not connect through the proxy".into()),
        Err(e) => return failed(e.without_url().to_string()),
    };
    let ms = Some(started.elapsed().as_millis() as u64);
    let status = response.status().as_u16();
    if target.is_some() {
        return Checked {
            ok: status < 400,
            ms,
            status: Some(status),
            error: (status >= 400).then(|| format!("the site answered HTTP {status}")),
            ..Checked::default()
        };
    }
    let mut body = Vec::new();
    while body.len() < 4096 {
        match response.chunk().await {
            Ok(Some(chunk)) => body.extend_from_slice(&chunk),
            _ => break,
        }
    }
    let (ip, country) = parse_trace(&String::from_utf8_lossy(&body));
    Checked {
        ok: ip.is_some(),
        ms,
        error: ip
            .is_none()
            .then(|| "the proxy answered, but not with the test page".to_string()),
        ip,
        country,
        status: Some(status),
    }
}

impl Pool {
    fn check_batch(&self, ids: &[String]) -> Vec<String> {
        let mut chosen: Vec<(bool, &String)> = self
            .config
            .proxies
            .iter()
            .zip(&self.health)
            .filter(|(proxy, _)| ids.is_empty() || ids.contains(&id_of(proxy)))
            .map(|(proxy, health)| (health.checked.is_some(), proxy))
            .collect();
        chosen.sort_by_key(|(checked, _)| *checked);
        chosen
            .into_iter()
            .take(CHECK_BATCH)
            .map(|(_, proxy)| proxy.clone())
            .collect()
    }

    fn unchecked(&self) -> usize {
        self.health.iter().filter(|h| h.checked.is_none()).count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CheckRun {
    pub checked: usize,
    pub unchecked: usize,
}

pub async fn check(ids: &[String], target: Option<url::Url>) -> CheckRun {
    use futures::StreamExt;
    let proxies = with(|pool| pool.check_batch(ids));
    let target = target.as_ref();
    let results: Vec<(String, Checked)> = futures::stream::iter(proxies)
        .map(|proxy| async move {
            let checked = check_one(&proxy, target).await;
            (proxy, checked)
        })
        .buffer_unordered(CHECK_CONCURRENCY)
        .collect()
        .await;
    let checked = results.len();
    with(|pool| {
        let now = Instant::now();
        for (proxy, result) in results {
            pool.record_check(&proxy, result, now);
        }
        CheckRun {
            checked,
            unchecked: pool.unchecked(),
        }
    })
}

pub fn holds_secret(raw: &str) -> bool {
    parse(raw).is_ok_and(|proxy| redact(&proxy) != proxy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(rotation: Rotation, n: usize) -> Pool {
        let mut pool = Pool::new(Config {
            enabled: true,
            rotation,
            proxies: Vec::new(),
        });
        let inputs: Vec<String> = (1..=n).map(|i| format!("10.0.0.{i}:8080")).collect();
        assert_eq!(pool.add(&inputs).added, n);
        pool
    }

    fn picked(pool: &mut Pool, now: Instant) -> String {
        pool.pick(now).unwrap().to_string()
    }

    #[test]
    fn every_common_list_format_becomes_one_url() {
        assert_eq!(parse("1.2.3.4:8080").unwrap(), "http://1.2.3.4:8080");
        assert_eq!(
            parse(" socks5h://127.0.0.1:7890/ ").unwrap(),
            "socks5h://127.0.0.1:7890"
        );
        assert_eq!(
            parse("1.2.3.4:8080:bob:s3cret").unwrap(),
            "http://bob:s3cret@1.2.3.4:8080"
        );
        assert_eq!(
            parse("bob:p@ss@proxy.example:3128").unwrap(),
            "http://bob:p%40ss@proxy.example:3128"
        );
        assert_eq!(
            parse("SOCKS4://bob:x@[2001:db8::1]:1080").unwrap(),
            "socks4://bob:x@[2001:db8::1]:1080"
        );
    }

    #[test]
    fn socks5_resolves_names_at_the_proxy_so_dns_does_not_leak() {
        assert_eq!(
            parse("socks5://127.0.0.1:7890").unwrap(),
            "socks5h://127.0.0.1:7890"
        );
    }

    #[test]
    fn a_bad_address_is_refused_with_the_reason() {
        assert_eq!(parse("  "), Err(ProxyError::Empty));
        assert_eq!(
            parse("ftp://1.2.3.4:21"),
            Err(ProxyError::Scheme("ftp".into()))
        );
        assert_eq!(parse("proxy.example"), Err(ProxyError::HostPort));
        assert_eq!(parse("socks5://:1080"), Err(ProxyError::HostPort));
        assert_eq!(parse("1.2.3.4:0"), Err(ProxyError::HostPort));
        assert_eq!(parse("1.2.3.4:99999"), Err(ProxyError::HostPort));
        assert_eq!(
            parse("http://1.2.3.4:80/path?x=1"),
            Err(ProxyError::HostPort)
        );
    }

    #[test]
    fn a_password_never_shows_in_a_view_or_a_rejection() {
        let mut pool = pool(Rotation::Sticky, 0);
        let out = pool.add(&[
            "socks5://bob:s3cret@1.2.3.4:1080".into(),
            "ftp://bob:hunter2@1.2.3.4:21".into(),
            "1.2.3.4:x:bob:hunter3".into(),
        ]);
        assert_eq!(out.added, 1);
        let shown = serde_json::to_string(&(pool.view(Instant::now(), 10), &out)).unwrap();
        assert!(shown.contains("socks5h://bob:***@1.2.3.4:1080"), "{shown}");
        for secret in ["s3cret", "hunter2", "hunter3"] {
            assert!(!shown.contains(secret), "{shown}");
        }
        assert_eq!(redact("http://1.2.3.4:80"), "http://1.2.3.4:80");
    }

    #[test]
    fn duplicates_comments_and_the_size_limit_are_counted_not_stored() {
        let mut pool = pool(Rotation::Sticky, 2);
        let out = pool.add(&[
            "# my list".into(),
            String::new(),
            "http://10.0.0.1:8080".into(),
            "10.0.0.9:8080".into(),
        ]);
        assert_eq!((out.added, out.duplicates), (1, 1));
        assert!(out.rejected.is_empty());
        let many: Vec<String> = (0..MAX_PROXIES)
            .map(|i| format!("10.1.{}.{}:80", i / 250, i % 250))
            .collect();
        let out = pool.add(&many);
        assert_eq!(pool.config().proxies.len(), MAX_PROXIES);
        assert_eq!(out.rejected.len(), 3);
    }

    #[test]
    fn an_off_or_empty_pool_gives_no_proxy_and_says_why() {
        let now = Instant::now();
        let mut empty = pool(Rotation::Sticky, 0);
        assert_eq!(empty.pick(now), None);
        assert!(empty.view(now, 10).reason.unwrap().contains("empty"));
        let mut off = pool(Rotation::Sticky, 2);
        off.configure(Some(false), None);
        assert_eq!(off.pick(now), None);
        assert_eq!(off.rotate(now), None);
        assert!(off.view(now, 10).reason.unwrap().contains("off"));
    }

    #[test]
    fn sticky_keeps_one_proxy_until_it_fails() {
        let now = Instant::now();
        let mut pool = pool(Rotation::Sticky, 3);
        let first = picked(&mut pool, now);
        assert_eq!(picked(&mut pool, now), first);
        pool.report(&first, false, now);
        let second = picked(&mut pool, now);
        assert_ne!(second, first);
        assert_eq!(picked(&mut pool, now), second);
        pool.report(&second, true, now);
        assert_eq!(picked(&mut pool, now), second);
    }

    #[test]
    fn round_robin_walks_the_pool_and_skips_a_resting_proxy() {
        let now = Instant::now();
        let mut pool = pool(Rotation::RoundRobin, 3);
        let round: Vec<String> = (0..3).map(|_| picked(&mut pool, now)).collect();
        assert_eq!(
            round,
            [
                "http://10.0.0.1:8080",
                "http://10.0.0.2:8080",
                "http://10.0.0.3:8080"
            ]
        );
        pool.report("http://10.0.0.2:8080", false, now);
        let round: Vec<String> = (0..4).map(|_| picked(&mut pool, now)).collect();
        assert!(!round.contains(&"http://10.0.0.2:8080".to_string()));
        let later = now + BASE_COOLDOWN + Duration::from_secs(1);
        let round: Vec<String> = (0..3).map(|_| picked(&mut pool, later)).collect();
        assert!(round.contains(&"http://10.0.0.2:8080".to_string()));
    }

    #[test]
    fn rest_grows_with_each_failure_and_ends_on_success() {
        let now = Instant::now();
        let mut pool = pool(Rotation::Sticky, 2);
        let proxy = "http://10.0.0.1:8080";
        pool.report(proxy, false, now);
        assert_eq!(pool.view(now, 10).proxies[0].resting_secs, 30);
        pool.report(proxy, false, now);
        assert_eq!(pool.view(now, 10).proxies[0].resting_secs, 60);
        for _ in 0..20 {
            pool.report(proxy, false, now);
        }
        assert_eq!(pool.view(now, 10).proxies[0].resting_secs, 600);
        pool.report(proxy, true, now);
        let view = pool.view(now, 10);
        assert_eq!(
            (view.proxies[0].fails, view.proxies[0].resting_secs),
            (0, 0)
        );
    }

    #[test]
    fn when_every_proxy_rests_the_pool_never_falls_back_to_direct() {
        let now = Instant::now();
        let mut pool = pool(Rotation::Sticky, 2);
        pool.report("http://10.0.0.1:8080", false, now);
        pool.report("http://10.0.0.1:8080", false, now);
        pool.report("http://10.0.0.2:8080", false, now);
        assert_eq!(picked(&mut pool, now), "http://10.0.0.2:8080");
        let view = pool.view(now, 10);
        assert_eq!(view.available, 0);
        assert!(view.reason.unwrap().contains("resting"));
    }

    #[test]
    fn rotate_moves_on_without_punishing_the_current_proxy() {
        let now = Instant::now();
        let mut pool = pool(Rotation::Sticky, 3);
        assert_eq!(picked(&mut pool, now), "http://10.0.0.1:8080");
        assert_eq!(pool.rotate(now), Some("http://10.0.0.2:8080"));
        assert_eq!(picked(&mut pool, now), "http://10.0.0.2:8080");
        assert_eq!(pool.view(now, 10).proxies[0].fails, 0);
        pool.rotate(now);
        assert_eq!(pool.rotate(now), Some("http://10.0.0.1:8080"));
    }

    #[test]
    fn removing_keeps_the_current_proxy_and_the_health_of_the_rest() {
        let now = Instant::now();
        let mut pool = pool(Rotation::Sticky, 3);
        pool.rotate(now);
        pool.rotate(now);
        pool.report("http://10.0.0.2:8080", false, now);
        assert_eq!(
            pool.remove(&[id_of("http://10.0.0.1:8080"), "px_nope".into()]),
            1
        );
        let view = pool.view(now, 10);
        assert_eq!(view.total, 2);
        assert_eq!(view.proxies[0].fails, 1);
        assert!(view.proxies[1].current);
        assert_eq!(picked(&mut pool, now), "http://10.0.0.3:8080");
        assert_eq!(pool.remove(&[id_of("http://10.0.0.3:8080")]), 1);
        assert_eq!(picked(&mut pool, now), "http://10.0.0.2:8080");
    }

    #[test]
    fn a_failed_check_rests_the_proxy_and_marks_it_for_cleanup() {
        let now = Instant::now();
        let mut pool = pool(Rotation::Sticky, 2);
        let dead = Checked {
            error: Some("could not connect through the proxy".into()),
            ..Checked::default()
        };
        pool.record_check("http://10.0.0.1:8080", dead, now);
        let alive = Checked {
            ok: true,
            ip: Some("203.0.113.9".into()),
            ..Checked::default()
        };
        pool.record_check("http://10.0.0.2:8080", alive, now);
        assert_eq!(picked(&mut pool, now), "http://10.0.0.2:8080");
        assert_eq!(pool.failing_ids(), [id_of("http://10.0.0.1:8080")]);
        let ids = pool.failing_ids();
        assert_eq!(pool.remove(&ids), 1);
    }

    #[test]
    fn a_check_takes_one_batch_and_starts_with_the_never_checked() {
        let now = Instant::now();
        let mut pool = pool(Rotation::Sticky, CHECK_BATCH + 3);
        let ok = Checked {
            ok: true,
            ..Checked::default()
        };
        for proxy in pool.check_batch(&[]) {
            pool.record_check(&proxy, ok.clone(), now);
        }
        assert_eq!(pool.unchecked(), 3);
        let next = pool.check_batch(&[]);
        assert_eq!(next.len(), CHECK_BATCH);
        assert_eq!(next[0], format!("http://10.0.0.{}:8080", CHECK_BATCH + 1));
        let one = pool.check_batch(&[id_of("http://10.0.0.2:8080")]);
        assert_eq!(one, ["http://10.0.0.2:8080"]);
    }

    #[test]
    fn an_address_with_a_password_is_recognised_in_every_format() {
        assert!(holds_secret("socks5://bob:pw@1.2.3.4:1080"));
        assert!(holds_secret("1.2.3.4:8080:bob:pw"));
        assert!(!holds_secret("socks5://127.0.0.1:7890"));
        assert!(!holds_secret("bob@1.2.3.4:8080"));
    }

    #[test]
    fn a_long_pool_is_listed_up_to_the_limit_and_says_so() {
        let pool = pool(Rotation::Sticky, 5);
        let view = pool.view(Instant::now(), 2);
        assert_eq!(
            (view.proxies.len(), view.total, view.truncated),
            (2, 5, true)
        );
    }

    #[tokio::test]
    async fn a_request_really_goes_through_the_proxy_and_a_dead_proxy_fails() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 2048];
            let n = stream.read(&mut buffer).await.unwrap();
            let head = String::from_utf8_lossy(&buffer[..n]).into_owned();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
            head
        });
        let target = url::Url::parse("http://site.invalid/page").unwrap();
        let proxy = parse(&format!("127.0.0.1:{port}:bob:pw")).unwrap();
        let alive = check_one(&proxy, Some(&target)).await;
        assert!(alive.ok, "{alive:?}");
        assert_eq!(alive.status, Some(200));
        let head = server.await.unwrap();
        assert!(head.starts_with("GET http://site.invalid/page"), "{head}");
        assert!(
            head.to_lowercase().contains("proxy-authorization: basic"),
            "{head}"
        );

        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_port = closed.local_addr().unwrap().port();
        drop(closed);
        let dead = check_one(&format!("http://127.0.0.1:{dead_port}"), Some(&target)).await;
        assert!(!dead.ok);
        assert!(dead.error.is_some());
    }

    #[test]
    fn the_trace_page_gives_the_exit_address_and_country() {
        let body = "fl=1\nip=203.0.113.9\nts=1\nloc=BR\n";
        assert_eq!(
            parse_trace(body),
            (Some("203.0.113.9".into()), Some("BR".into()))
        );
        assert_eq!(parse_trace("<html>"), (None, None));
    }
}
