use super::Profile;
use crate::limits_strip::AccountTag;
use crate::limits_strip::{
    http, iso_ms, json_or_error, net_err, now_ms, LimitWindow, ReadError, Reading, Secret,
    UsageProvider, DAY_MS, HOUR_MS,
};
use std::path::{Path, PathBuf};

const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

pub struct Claude {
    id: String,
    dir: Option<PathBuf>,
    tag: AccountTag,
}

impl Claude {
    pub fn new(p: Profile) -> Self {
        Self {
            id: p.id,
            dir: p.dir,
            tag: p.tag,
        }
    }

    fn dir(&self) -> Option<PathBuf> {
        self.dir.clone().or_else(config_dir)
    }
}

fn env_config_dir() -> Option<String> {
    std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .filter(|v| !v.is_empty())
}

pub fn keychain_service(config_dir: Option<&str>) -> String {
    use sha2::{Digest, Sha256};
    match config_dir {
        None => KEYCHAIN_SERVICE.to_string(),
        Some(dir) => {
            let hash = hex::encode(Sha256::digest(dir.as_bytes()));
            format!("{KEYCHAIN_SERVICE}-{}", &hash[..8])
        }
    }
}

pub fn keychain_services(dir: Option<&Path>) -> Vec<String> {
    match dir {
        Some(d) => vec![keychain_service(Some(&d.to_string_lossy()))],
        None => match env_config_dir() {
            Some(d) => vec![keychain_service(Some(&d)), keychain_service(None)],
            None => vec![keychain_service(None)],
        },
    }
}

pub fn profile_json(dir: Option<&Path>) -> Option<PathBuf> {
    match dir {
        Some(d) => Some(d.join(".claude.json")),
        None => match env_config_dir() {
            Some(d) => Some(PathBuf::from(d).join(".claude.json")),
            None => crate::limits_strip::home().map(|h| h.join(".claude.json")),
        },
    }
}

pub fn parse_email(v: &serde_json::Value) -> Option<String> {
    v.pointer("/oauthAccount/emailAddress")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn email_of(dir: Option<&Path>) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::SystemTime;
    type Stamp = (Option<SystemTime>, u64);
    type Entries = HashMap<PathBuf, (Stamp, Option<String>)>;
    static CACHE: Mutex<Option<Entries>> = Mutex::new(None);

    let path = profile_json(dir)?;
    let Ok(meta) = std::fs::metadata(&path) else {
        if let Ok(mut g) = CACHE.lock() {
            g.get_or_insert_with(HashMap::new).remove(&path);
        }
        return None;
    };
    let stamp: Stamp = (meta.modified().ok(), meta.len());
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some((seen, email)) = cache.get(&path) {
        if *seen == stamp {
            return email.clone();
        }
    }
    match crate::limits_strip::read_json(&path) {
        Some(v) => {
            let email = parse_email(&v);
            cache.insert(path, (stamp, email.clone()));
            email
        }
        None => cache.get(&path).and_then(|(_, e)| e.clone()),
    }
}

pub fn config_dir() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    crate::limits_strip::home().map(|h| h.join(".claude"))
}

#[derive(Debug, PartialEq)]
pub struct Credential {
    token: Secret,
    pub expires_at: Option<i64>,
    pub plan: Option<String>,
}

pub fn parse_credential(v: &serde_json::Value) -> Option<Credential> {
    let oauth = v.get("claudeAiOauth").unwrap_or(v);
    let token = Secret::new(oauth.get("accessToken")?.as_str()?)?;
    Some(Credential {
        token,
        expires_at: oauth
            .get("expiresAt")
            .and_then(|x| x.as_f64())
            .map(|ms| ms as i64)
            .filter(|ms| *ms > 0),
        plan: oauth
            .get("subscriptionType")
            .and_then(|x| x.as_str())
            .map(str::to_string),
    })
}

#[cfg(target_os = "macos")]
async fn keychain_credential(service: &str) -> Option<Credential> {
    let user = std::env::var("USER").ok()?;
    let (ok, out, _) = super::run_cli(
        std::path::Path::new("/usr/bin/security"),
        &["find-generic-password", "-s", service, "-a", &user, "-w"],
        &[],
        std::time::Duration::from_secs(5),
    )
    .await?;
    if !ok {
        return None;
    }
    parse_credential(&serde_json::from_str(out.trim()).ok()?)
}

#[cfg(not(target_os = "macos"))]
async fn keychain_credential(_service: &str) -> Option<Credential> {
    None
}

#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    Keychain(String),
}

pub fn sources(own_dir: Option<&Path>) -> Vec<Source> {
    let dir = own_dir.map(Path::to_path_buf).or_else(config_dir);
    let mut out: Vec<Source> = dir
        .map(|d| Source::File(d.join(".credentials.json")))
        .into_iter()
        .collect();
    out.extend(keychain_services(own_dir).into_iter().map(Source::Keychain));
    out
}

async fn credential(own_dir: Option<&Path>) -> Option<Credential> {
    for source in sources(own_dir) {
        let found = match &source {
            Source::File(path) => crate::limits_strip::read_json(path)
                .as_ref()
                .and_then(parse_credential),
            Source::Keychain(service) => keychain_credential(service).await,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

fn label_for(kind: &str, scope: Option<&str>) -> String {
    match kind {
        "session" | "five_hour" => "Current session".into(),
        "weekly_all" | "seven_day" | "weekly" => "Weekly (all models)".into(),
        "weekly_opus" | "seven_day_opus" => "Weekly (Opus)".into(),
        "weekly_sonnet" | "seven_day_sonnet" => "Weekly (Sonnet)".into(),
        "weekly_scoped" | "scoped" => format!("Weekly ({})", scope.unwrap_or("scoped")),
        other => {
            let s = other.trim_start_matches("weekly_").replace('_', " ");
            let mut c = s.chars();
            match c.next() {
                Some(f) => format!("Weekly ({}{})", f.to_uppercase(), c.as_str()),
                None => "Limit".into(),
            }
        }
    }
}

fn canonical(kind: &str) -> &str {
    match kind {
        "five_hour" => "session",
        "seven_day" | "weekly" => "weekly_all",
        "seven_day_opus" => "weekly_opus",
        "seven_day_sonnet" => "weekly_sonnet",
        k => k,
    }
}

fn span_of(id: &str) -> i64 {
    if id == "session" {
        5 * HOUR_MS
    } else {
        7 * DAY_MS
    }
}

pub fn parse_usage(v: &serde_json::Value) -> Vec<LimitWindow> {
    let mut out: Vec<LimitWindow> = Vec::new();
    if let Some(arr) = v.get("limits").and_then(|x| x.as_array()) {
        for l in arr {
            let (Some(kind), Some(pct)) = (
                l.get("kind").and_then(|x| x.as_str()),
                l.get("percent").and_then(|x| x.as_f64()),
            ) else {
                continue;
            };
            let Some(resets) = iso_ms(l.get("resets_at")) else {
                continue;
            };
            let id = canonical(kind);
            if out.iter().any(|w| w.id == id) {
                continue;
            }
            let scope = l
                .pointer("/scope/model/display_name")
                .and_then(|x| x.as_str());
            out.push(
                LimitWindow::percent(id, &label_for(kind, scope), pct)
                    .resets(Some(resets))
                    .span(span_of(id)),
            );
        }
    }
    for field in [
        "five_hour",
        "seven_day",
        "seven_day_opus",
        "seven_day_sonnet",
    ] {
        let Some(w) = v.get(field).filter(|w| w.is_object()) else {
            continue;
        };
        let Some(u) = w.get("utilization").and_then(|x| x.as_f64()) else {
            continue;
        };
        let id = canonical(field);
        if out.iter().any(|x| x.id == id) {
            continue;
        }
        out.push(
            LimitWindow::percent(id, &label_for(field, None), u)
                .resets(iso_ms(w.get("resets_at")))
                .span(span_of(id)),
        );
    }
    out.sort_by_key(|w| match w.id.as_str() {
        "session" => (0, String::new()),
        "weekly_all" => (1, String::new()),
        other => (2, other.to_string()),
    });
    out
}

#[async_trait::async_trait]
impl UsageProvider for Claude {
    fn id(&self) -> &str {
        &self.id
    }
    fn label(&self) -> &'static str {
        "Claude Code"
    }
    fn beta(&self) -> bool {
        false
    }
    fn account(&self) -> Option<&AccountTag> {
        Some(&self.tag)
    }

    async fn detect(&self) -> bool {
        self.dir().map(|d| d.is_dir()).unwrap_or(false)
    }

    async fn read(&self) -> Result<Reading, ReadError> {
        let Some(cred) = credential(self.dir.as_deref()).await else {
            return Err(ReadError::NeedsAuth("no Claude Code login found".into()));
        };
        if cred.expires_at.map(|t| t <= now_ms()).unwrap_or(false) {
            return Err(ReadError::NeedsAuth(
                "login expired: run claude once to renew it".into(),
            ));
        }
        let resp = http()
            .get(ENDPOINT)
            .bearer_auth(cred.token.expose())
            .header("anthropic-beta", "oauth-2025-04-20")
            .send()
            .await
            .map_err(net_err)?;
        let v = json_or_error(resp).await?;
        let windows = parse_usage(&v);
        Ok(Reading {
            note: windows
                .is_empty()
                .then(|| "nothing metered on this account".to_string()),
            windows,
            plan: cred.plan,
            ..Default::default()
        })
    }
}
