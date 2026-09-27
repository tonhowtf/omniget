//! Claude Code.
//!
//! Credential (read only, never refreshed, never rewritten):
//!   - Linux and Windows: `<config>/.credentials.json`, where `<config>` is
//!     `$CLAUDE_CONFIG_DIR` or `~/.claude` (`%USERPROFILE%\.claude`).
//!   - macOS: the same file when it exists, otherwise the login keychain item
//!     `Claude Code-credentials` (account = the OS user). It is asked through
//!     `/usr/bin/security`, the tool Claude Code itself files the item with, so
//!     no keychain prompt is raised for OmniGet.
//!   - Shape: `{"claudeAiOauth":{"accessToken","expiresAt"(epoch ms),"subscriptionType"}}`.
//!   - Other profiles (one per account in `/llm` → Accounts, each with its own
//!     `CLAUDE_CONFIG_DIR`): `<dir>/.credentials.json`, otherwise the keychain
//!     item `Claude Code-credentials-<8 hex>`, the first 8 hex digits of
//!     sha256 of the directory string exactly as it is handed to the CLI.
//!
//! Identity (not a credential): `oauthAccount.emailAddress` in
//! `~/.claude.json` for the default profile, `<dir>/.claude.json` otherwise.
//!
//! Endpoint: `GET https://api.anthropic.com/api/oauth/usage` with
//! `Authorization: Bearer` and `anthropic-beta: oauth-2025-04-20`, the same one
//! Claude Code's own `/usage` asks. An expired token is never sent: the
//! endpoint answers it with a long 429, not a 401.

use super::Profile;
use crate::limits_strip::AccountTag;
use crate::limits_strip::{
    http, iso_ms, json_or_error, net_err, now_ms, LimitWindow, ReadError, Reading, Secret,
    UsageProvider, DAY_MS, HOUR_MS,
};
use std::path::{Path, PathBuf};

const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// One Claude Code login. The default profile has `dir: None`.
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

    /// The directory this login lives in.
    fn dir(&self) -> Option<PathBuf> {
        self.dir.clone().or_else(config_dir)
    }
}

/// `$CLAUDE_CONFIG_DIR` when the OmniGet process itself carries one.
fn env_config_dir() -> Option<String> {
    std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .filter(|v| !v.is_empty())
}

/// The keychain service Claude Code files a login under: plain for the
/// default `~/.claude`, suffixed with the first 8 hex digits of sha256 of the
/// config dir string for any other `CLAUDE_CONFIG_DIR`.
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

/// The keychain services to try for one profile, in order.
pub fn keychain_services(dir: Option<&Path>) -> Vec<String> {
    match dir {
        Some(d) => vec![keychain_service(Some(&d.to_string_lossy()))],
        None => match env_config_dir() {
            // Moved by the environment: the CLI files it suffixed; older
            // builds filed it plain.
            Some(d) => vec![keychain_service(Some(&d)), keychain_service(None)],
            None => vec![keychain_service(None)],
        },
    }
}

/// Where the CLI keeps its account profile (`oauthAccount`), no token inside.
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

/// The e-mail a profile is signed in with, from `.claude.json`.
///
/// Claude Code rewrites that file all the time, so a read can land mid-write
/// and see half a JSON. Forgetting the e-mail then would un-merge two twin
/// logins (see `profiles`), flip the ring list every few seconds and make the
/// strip re-place itself and the usage icon rebuild its native menu. So the
/// last good answer is kept per file, and the file is only parsed again when
/// its size or mtime moved.
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
        // Half-written: keep what we knew, and try again on the next change.
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
    // An emptied credential is how the owner signs out.
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

/// Where a profile's credential is looked for, in order.
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

/// `limits[]` is the forward-compatible shape; `five_hour` / `seven_day` are
/// merged in because a window that just rolled over vanishes from `limits`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_and_named_windows_merge_without_twins() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":30.0,"resets_at":"2099-01-01T05:00:00.000000+00:00"},
                "seven_day":{"utilization":74.0,"resets_at":"2099-01-05T00:00:00.000000+00:00"},
                "limits":[{"kind":"weekly_all","percent":74,"resets_at":"2099-01-05T00:00:00Z"},
                          {"kind":"weekly_scoped","percent":55,"resets_at":"2099-01-05T00:00:00Z",
                           "scope":{"model":{"display_name":"Opus"}}},
                          {"kind":"no_reset","percent":10}]}"#,
        )
        .unwrap();
        let ws = parse_usage(&v);
        let ids: Vec<_> = ws.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["session", "weekly_all", "weekly_scoped"]);
        assert!((ws[0].used.unwrap() - 0.30).abs() < 1e-6);
        assert_eq!(ws[0].span_ms, Some(5 * HOUR_MS));
        assert_eq!(ws[2].label, "Weekly (Opus)");
    }

    #[test]
    fn an_emptied_credential_is_signed_out() {
        let v = serde_json::json!({"claudeAiOauth":{"accessToken":"","expiresAt":0}});
        assert_eq!(parse_credential(&v), None);
        let v = serde_json::json!({"claudeAiOauth":{"accessToken":"t","expiresAt":1.7e12,"subscriptionType":"max"}});
        let c = parse_credential(&v).unwrap();
        assert_eq!(c.token.expose(), "t");
        assert_eq!(c.plan.as_deref(), Some("max"));
        assert_eq!(c.expires_at, Some(1_700_000_000_000));
    }

    #[test]
    fn the_token_never_shows_in_debug_output() {
        let v = serde_json::json!({"claudeAiOauth":{"accessToken":"sk-ant-oat01-SECRETVALUE","expiresAt":1.7e12}});
        let c = parse_credential(&v).unwrap();
        assert!(!format!("{c:?}").contains("SECRETVALUE"));
    }

    #[test]
    fn the_keychain_service_carries_eight_hex_of_the_dir_hash() {
        assert_eq!(keychain_service(None), "Claude Code-credentials");
        // Measured on this machine on 2026-09-26 against a real login.
        assert_eq!(
            keychain_service(Some(
                "/var/folders/bq/7dtfgtc12hg7yhdykjdh_23r0000gn/T/omniget-acc-1"
            )),
            "Claude Code-credentials-417fbab5"
        );
        let other = keychain_service(Some("/tmp/x"));
        let suffix = other.strip_prefix("Claude Code-credentials-").unwrap();
        assert_eq!(suffix.len(), 8);
        assert!(suffix
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn an_account_reads_its_own_file_then_its_own_keychain_item() {
        let dir = PathBuf::from("/tmp/omniget-acc-x");
        let got = sources(Some(&dir));
        assert_eq!(
            got,
            vec![
                Source::File(dir.join(".credentials.json")),
                Source::Keychain(keychain_service(Some("/tmp/omniget-acc-x"))),
            ]
        );
        // The default profile ends on the plain item.
        let default = sources(None);
        assert_eq!(
            default.last(),
            Some(&Source::Keychain("Claude Code-credentials".into()))
        );
    }

    #[test]
    fn the_email_comes_from_oauth_account() {
        let v = serde_json::json!({"oauthAccount":{"emailAddress":" a@b.c ","accountUuid":"u"}});
        assert_eq!(parse_email(&v).as_deref(), Some("a@b.c"));
        assert_eq!(parse_email(&serde_json::json!({})), None);
        assert_eq!(
            profile_json(Some(Path::new("/p"))),
            Some(PathBuf::from("/p/.claude.json"))
        );
    }

    /// Live: reads every Claude login saved in the app and asks the usage
    /// endpoint once each. Prints ids, e-mails and percentages only.
    /// `cargo test -p omniget --lib live_every_claude_account -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_every_claude_account() {
        for r in crate::limits_strip::providers::all() {
            if r.label() != "Claude Code" {
                continue;
            }
            let who = r.account().cloned().unwrap_or_default();
            match r.read().await {
                Ok(reading) => {
                    let pcts: Vec<String> = reading
                        .windows
                        .iter()
                        .map(|w| format!("{} {:.0}%", w.id, w.used.unwrap_or(0.0) * 100.0))
                        .collect();
                    println!(
                        "{} [{:?} / {:?}] plan={:?}: {}",
                        r.id(),
                        who.label,
                        who.email,
                        reading.plan,
                        pcts.join(", ")
                    );
                }
                Err(e) => println!(
                    "{} [{:?} / {:?}]: {}",
                    r.id(),
                    who.label,
                    who.email,
                    e.code()
                ),
            }
        }
    }
}
