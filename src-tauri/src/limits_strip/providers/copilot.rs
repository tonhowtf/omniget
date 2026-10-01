//! GitHub Copilot.
//!
//! Credential (read only), the GitHub CLI's own session, in this order:
//!   1. `GH_TOKEN`, then `GITHUB_TOKEN`.
//!   2. `hosts.yml` of the GitHub CLI: `$GH_CONFIG_DIR`, else
//!      `$XDG_CONFIG_HOME/gh`, else `~/.config/gh` (macOS and Linux) or
//!      `%APPDATA%\GitHub CLI` (Windows); the `oauth_token:` under `github.com:`.
//!      Often absent because `gh` prefers the OS keyring.
//!   3. `gh auth token --hostname github.com`, which reads that keyring for us.
//!
//! Endpoint: `GET https://api.github.com/copilot_internal/user`, the quota the
//! editors' own Copilot panels show. `quota_snapshots.<id>` carries
//! `entitlement`, `remaining`, `unlimited`; `quota_reset_date` is ISO.

use crate::limits_strip::{
    epoch_ms, http, iso_ms, json_or_error, net_err, num, LimitWindow, ReadError, Reading, Secret,
    UsageProvider,
};
use std::path::PathBuf;
use std::time::Duration;

const ENDPOINT: &str = "https://api.github.com/copilot_internal/user";

pub struct Copilot;

pub fn gh_config_dir() -> Option<PathBuf> {
    for var in ["GH_CONFIG_DIR", "XDG_CONFIG_HOME"] {
        if let Some(v) = std::env::var_os(var).filter(|v| !v.is_empty()) {
            let p = PathBuf::from(v);
            return Some(if var == "GH_CONFIG_DIR" {
                p
            } else {
                p.join("gh")
            });
        }
    }
    #[cfg(windows)]
    {
        dirs::config_dir().map(|c| c.join("GitHub CLI"))
    }
    #[cfg(not(windows))]
    {
        crate::limits_strip::home().map(|h| h.join(".config").join("gh"))
    }
}

/// The `oauth_token:` inside the `github.com:` block of `hosts.yml`. A tiny
/// line reader is enough for the two-level file `gh` writes.
pub fn token_from_hosts(yaml: &str) -> Option<Secret> {
    let mut in_host = false;
    for line in yaml.lines() {
        if !line.starts_with([' ', '\t']) {
            in_host = line.trim_end().trim_end_matches(':') == "github.com";
            continue;
        }
        if in_host {
            if let Some(rest) = line.trim().strip_prefix("oauth_token:") {
                if let Some(t) = Secret::new(rest.trim().trim_matches(['"', '\''])) {
                    return Some(t);
                }
            }
        }
    }
    None
}

async fn token() -> Option<Secret> {
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Some(t) = std::env::var(var).ok().as_deref().and_then(Secret::new) {
            return Some(t);
        }
    }
    if let Some(t) = gh_config_dir()
        .and_then(|d| std::fs::read_to_string(d.join("hosts.yml")).ok())
        .as_deref()
        .and_then(token_from_hosts)
    {
        return Some(t);
    }
    let gh = which::which("gh").ok().or_else(|| {
        ["/opt/homebrew/bin/gh", "/usr/local/bin/gh", "/usr/bin/gh"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
    })?;
    let (ok, out, _) = super::run_cli(
        &gh,
        &["auth", "token", "--hostname", "github.com"],
        &[],
        Duration::from_secs(8),
    )
    .await?;
    Secret::new(&out).filter(|_| ok)
}

fn title(id: &str) -> String {
    match id {
        "premium_interactions" => "Premium requests".into(),
        "chat" => "Chat requests".into(),
        "completions" => "Completions".into(),
        other => {
            let s = other.replace('_', " ");
            let mut c = s.chars();
            c.next()
                .map(|f| format!("{}{}", f.to_uppercase(), c.as_str()))
                .unwrap_or_default()
        }
    }
}

fn reset_of(v: Option<&serde_json::Value>) -> Option<i64> {
    iso_ms(v).or_else(|| num(v).map(epoch_ms))
}

pub fn parse_usage(v: &serde_json::Value) -> Option<Vec<LimitWindow>> {
    let snaps = v.get("quota_snapshots")?.as_object()?;
    let root_reset = reset_of(v.get("quota_reset_date"));
    let mut ids: Vec<&String> = snaps.keys().collect();
    ids.sort_by_key(|id| match id.as_str() {
        "premium_interactions" => (0, String::new()),
        "chat" => (1, String::new()),
        "completions" => (2, String::new()),
        other => (3, other.to_string()),
    });
    let mut out = Vec::new();
    for id in ids {
        let q = &snaps[id];
        let entitlement = num(q.get("entitlement")).unwrap_or(0.0);
        if q.get("unlimited").and_then(|x| x.as_bool()) == Some(true) || entitlement <= 0.0 {
            continue;
        }
        let used = num(q.get("used"))
            .or_else(|| num(q.get("remaining")).map(|r| entitlement - r))
            .unwrap_or(0.0);
        let reset = ["reset_date", "reset_at", "resets_at"]
            .iter()
            .find_map(|k| reset_of(q.get(*k)))
            .or(root_reset);
        out.push(
            LimitWindow::ratio(id, &title(id), used, entitlement, "requests")
                .resets(reset)
                .span(30 * crate::limits_strip::DAY_MS),
        );
    }
    Some(out)
}

#[async_trait::async_trait]
impl UsageProvider for Copilot {
    fn id(&self) -> &str {
        "copilot"
    }
    fn label(&self) -> &'static str {
        "GitHub Copilot"
    }

    async fn detect(&self) -> bool {
        std::env::var_os("GH_TOKEN").is_some()
            || gh_config_dir().map(|d| d.is_dir()).unwrap_or(false)
    }

    async fn read(&self) -> Result<Reading, ReadError> {
        let token = token()
            .await
            .ok_or_else(|| ReadError::NeedsAuth("run gh auth login".into()))?;
        let resp = http()
            .get(ENDPOINT)
            .bearer_auth(token.expose())
            .header("Accept", "application/json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(net_err)?;
        let status = resp.status().as_u16();
        if status == 404 {
            return Ok(Reading {
                note: Some("no Copilot plan on this GitHub account".into()),
                ..Default::default()
            });
        }
        let v = json_or_error(resp).await?;
        let windows = parse_usage(&v).ok_or_else(|| ReadError::Other("unexpected reply".into()))?;
        Ok(Reading {
            note: windows.is_empty().then(|| "nothing metered".to_string()),
            windows,
            plan: v
                .get("copilot_plan")
                .or_else(|| v.get("plan"))
                .and_then(|x| x.as_str())
                .map(str::to_string),
            ..Default::default()
        })
    }
}
