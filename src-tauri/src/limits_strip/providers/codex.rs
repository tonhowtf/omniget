//! Codex.
//!
//! Credential (read only): `$CODEX_HOME/auth.json`, default `~/.codex/auth.json`
//! on every OS (`%USERPROFILE%\.codex` on Windows). An account saved in the
//! app reads `<its CODEX_HOME>/auth.json` and its own `sessions/`. Keys `tokens.access_token`
//! and `tokens.account_id`. A keychain-only or API-key-only login carries no
//! ChatGPT plan limits and reads as absent here.
//!
//! Live: `GET https://chatgpt.com/backend-api/wham/usage` with `Authorization:
//! Bearer` and `ChatGPT-Account-Id`. Reply: `rate_limit.{primary_window,
//! secondary_window}` with `used_percent`, `limit_window_seconds`, `reset_at`
//! (epoch seconds) or `reset_after_seconds`, plus `plan_type`.
//!
//! Fallback, no network: the limits Codex itself wrote on its last turn into
//! `sessions/YYYY/MM/DD/rollout-*.jsonl` (`payload.type == "token_count"`,
//! `rate_limits.{primary,secondary}` with `window_minutes`, `resets_at`).

use crate::limits_strip::{
    epoch_ms, http, json_or_error, net_err, now_ms, num, LimitWindow, ReadError, Reading, Secret,
    UsageProvider,
};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const ENDPOINT: &str = "https://chatgpt.com/backend-api/wham/usage";
const TAIL_BYTES: u64 = 256 * 1024;

/// One Codex login. The default profile has `home: None`.
pub struct Codex {
    id: String,
    home: Option<PathBuf>,
    tag: crate::limits_strip::AccountTag,
}

impl Codex {
    pub fn new(p: super::Profile) -> Self {
        Self {
            id: p.id,
            home: p.dir,
            tag: p.tag,
        }
    }

    fn home(&self) -> Option<PathBuf> {
        self.home.clone().or_else(codex_home)
    }
}

pub fn codex_home() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("CODEX_HOME") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    crate::limits_strip::home().map(|h| h.join(".codex"))
}

/// Codex names its windows only by length, and "5h limit" says more than
/// "primary". The primary window is not always five hours.
pub fn label_for_minutes(minutes: Option<f64>, fallback: &str) -> String {
    let Some(m) = minutes.filter(|m| *m > 0.0) else {
        return fallback.to_string();
    };
    let hours = m / 60.0;
    if (hours - 5.0).abs() < 0.5 {
        "5h limit".into()
    } else if (hours - 168.0).abs() < 12.0 {
        "Weekly limit".into()
    } else if (hours / 24.0 - 30.0).abs() <= 2.0 {
        "Monthly limit".into()
    } else if hours < 48.0 {
        format!("{}h limit", hours.round() as i64)
    } else {
        format!("{}d limit", (hours / 24.0).round() as i64)
    }
}

fn window_from(
    w: &serde_json::Value,
    id: &str,
    minutes: Option<f64>,
    now: i64,
    group: Option<&str>,
) -> Option<LimitWindow> {
    let pct = num(w.get("used_percent"))?;
    let resets = num(w.get("reset_at"))
        .or_else(|| num(w.get("resets_at")))
        .map(epoch_ms)
        .or_else(|| {
            num(w.get("reset_after_seconds"))
                .or_else(|| num(w.get("resets_in_seconds")))
                .map(|s| now + (s * 1000.0) as i64)
        });
    let mut out = LimitWindow::percent(id, &label_for_minutes(minutes, id), pct).resets(resets);
    out.span_ms = minutes.map(|m| (m * 60_000.0) as i64);
    out.group = group.map(str::to_string);
    Some(out)
}

pub fn parse_usage(v: &serde_json::Value, now: i64) -> Vec<LimitWindow> {
    let mut out = Vec::new();
    let mut pair = |rl: Option<&serde_json::Value>, ids: [&str; 2], group: Option<&str>| {
        let Some(rl) = rl.filter(|x| x.is_object()) else {
            return;
        };
        for (id, key) in ids.iter().zip(["primary_window", "secondary_window"]) {
            if let Some(w) = rl.get(key).filter(|w| w.is_object()) {
                let minutes = num(w.get("limit_window_seconds")).map(|s| s / 60.0);
                if let Some(win) = window_from(w, id, minutes, now, group) {
                    out.push(win);
                }
            }
        }
    };
    pair(v.get("rate_limit"), ["primary", "secondary"], None);
    if let Some(extras) = v.get("additional_rate_limits").and_then(|x| x.as_array()) {
        for extra in extras {
            let name = ["limit_name", "metered_feature"]
                .iter()
                .find_map(|k| extra.get(*k).and_then(|x| x.as_str()))
                .unwrap_or("");
            if name.to_lowercase().contains("spark") {
                pair(
                    extra.get("rate_limit"),
                    ["spark", "spark-secondary"],
                    Some("Spark"),
                );
            }
        }
    }
    pair(
        v.get("code_review_rate_limit"),
        ["code-review", "code-review-secondary"],
        Some("Code review"),
    );
    let mut seen = std::collections::HashSet::new();
    out.retain(|w: &LimitWindow| seen.insert(w.id.clone()));
    out
}

/// The last `token_count` line of a rollout log: `(windows, plan, line ts ms)`.
pub fn parse_rollout_tail(text: &str, now: i64) -> Option<(Vec<LimitWindow>, Option<String>, i64)> {
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(rl) = v.pointer("/payload/rate_limits").filter(|x| x.is_object()) else {
            continue;
        };
        let mut windows = Vec::new();
        for id in ["primary", "secondary"] {
            if let Some(w) = rl.get(id).filter(|w| w.is_object()) {
                if let Some(win) = window_from(w, id, num(w.get("window_minutes")), now, None) {
                    windows.push(win);
                }
            }
        }
        if windows.is_empty() {
            continue;
        }
        let plan = rl
            .get("plan_type")
            .and_then(|x| x.as_str())
            .map(str::to_string);
        let ts = crate::limits_strip::iso_ms(v.get("timestamp")).unwrap_or(0);
        return Some((windows, plan, ts));
    }
    None
}

fn newest_rollout(root: &Path) -> Option<PathBuf> {
    // sessions/YYYY/MM/DD: walk each level newest-first by name.
    fn newest_dirs(dir: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        v.sort();
        v.reverse();
        v
    }
    for y in newest_dirs(root) {
        for m in newest_dirs(&y) {
            for d in newest_dirs(&m) {
                let best = std::fs::read_dir(&d)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| {
                        let n = e.file_name();
                        let n = n.to_string_lossy();
                        n.starts_with("rollout-") && n.ends_with(".jsonl")
                    })
                    .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                    .max_by_key(|(t, _)| *t);
                if let Some((_, p)) = best {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn read_tail(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
        .ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn from_rollout(home: Option<PathBuf>) -> Option<Reading> {
    let root = home?.join("sessions");
    let now = now_ms();
    let (windows, plan, ts) = parse_rollout_tail(&read_tail(&newest_rollout(&root)?)?, now)?;
    let age_min = ((now - ts).max(0)) / 60_000;
    Some(Reading {
        windows,
        plan,
        note: (age_min > 5).then(|| "as of the last Codex run".to_string()),
        ..Default::default()
    })
}

#[derive(Debug, PartialEq)]
pub struct Credential {
    access: Secret,
    account: String,
}

pub fn parse_credential(v: &serde_json::Value) -> Option<Credential> {
    let tokens = v.get("tokens")?;
    let access = Secret::new(tokens.get("access_token")?.as_str()?)?;
    let account = tokens.get("account_id")?.as_str()?.trim().to_string();
    (!account.is_empty()).then_some(Credential { access, account })
}

fn credential(home: Option<&Path>) -> Option<Credential> {
    parse_credential(&crate::limits_strip::read_json(&home?.join("auth.json"))?)
}

#[async_trait::async_trait]
impl UsageProvider for Codex {
    fn id(&self) -> &str {
        &self.id
    }
    fn label(&self) -> &'static str {
        "Codex"
    }

    fn account(&self) -> Option<&crate::limits_strip::AccountTag> {
        Some(&self.tag)
    }

    async fn detect(&self) -> bool {
        self.home()
            .map(|h| h.join("auth.json").is_file() || h.join("sessions").is_dir())
            .unwrap_or(false)
    }

    async fn read(&self) -> Result<Reading, ReadError> {
        let home = self.home();
        let Some(cred) = credential(home.as_deref()) else {
            return tokio::task::spawn_blocking(move || from_rollout(home))
                .await
                .ok()
                .flatten()
                .ok_or_else(|| ReadError::NeedsAuth("no ChatGPT sign-in in Codex".into()));
        };
        let live = async {
            let resp = http()
                .get(ENDPOINT)
                .bearer_auth(cred.access.expose())
                .header("ChatGPT-Account-Id", &cred.account)
                .header("Accept", "application/json")
                .header("Cache-Control", "no-cache, no-store")
                .send()
                .await
                .map_err(net_err)?;
            json_or_error(resp).await
        }
        .await;
        match live {
            Ok(v) => {
                let windows = parse_usage(&v, now_ms());
                Ok(Reading {
                    note: windows
                        .is_empty()
                        .then(|| "nothing metered on this account".to_string()),
                    windows,
                    plan: v
                        .get("plan_type")
                        .and_then(|x| x.as_str())
                        .map(str::to_string),
                    ..Default::default()
                })
            }
            Err(e) => match tokio::task::spawn_blocking(move || from_rollout(home)).await {
                Ok(Some(r)) => Ok(r),
                _ => Err(e),
            },
        }
    }
}
