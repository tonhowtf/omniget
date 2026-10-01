//! Cursor.
//!
//! Credential (read only): the editor's own global state database, a SQLite
//! file with the VS Code `ItemTable(key, value)`:
//!   - macOS:   `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`
//!   - Windows: `%APPDATA%\Cursor\User\globalStorage\state.vscdb`
//!   - Linux:   `~/.config/Cursor/User/globalStorage/state.vscdb`
//!
//! Keys `cursorAuth/accessToken` and `cursorAuth/stripeMembershipAuthId` (or
//! the `sub` claim of the token) form the session the editor itself uses;
//! `cursorAuth/cachedEmail` and `cursorAuth/stripeMembershipType` are shown.
//! This is the editor's state file, not a browser cookie store.
//!
//! Endpoint: `GET https://cursor.com/api/usage-summary`. Cursor meters a
//! percentage of the allowance (`individualUsage.plan.totalPercentUsed`), not
//! requests, and 0 is a reading, not a gap.

use crate::limits_strip::{
    http, iso_ms, json_or_error, net_err, num, LimitWindow, ReadError, Reading, Secret,
    UsageProvider,
};
use base64::Engine;
use std::path::PathBuf;

const ENDPOINT: &str = "https://cursor.com/api/usage-summary";

pub struct Cursor;

pub fn state_db() -> Option<PathBuf> {
    dirs::config_dir().map(|c| {
        c.join("Cursor")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb")
    })
}

#[derive(Debug)]
struct Session {
    /// The session value the editor itself sends, built from its own state.
    cookie: Secret,
    email: Option<String>,
    plan: Option<String>,
}

/// The `sub` claim of the access token is `provider|user_id`; the user id is
/// the first half of the session value.
pub fn user_id_from_jwt(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let sub = v.get("sub")?.as_str()?;
    Some(sub.rsplit('|').next().unwrap_or(sub).to_string())
}

fn session_cookie(auth_id: &str, token: &str) -> Option<Secret> {
    Secret::new(&format!("WorkosCursorSessionToken={auth_id}::{token}"))
}

fn session() -> Option<Session> {
    let conn = super::open_sqlite_ro(&state_db()?)?;
    let clean = |s: String| s.trim().trim_matches('"').to_string();
    let token = super::vscdb_value(&conn, "cursorAuth/accessToken").map(clean)?;
    if token.is_empty() {
        return None;
    }
    let auth_id = super::vscdb_value(&conn, "cursorAuth/stripeMembershipAuthId")
        .map(clean)
        .filter(|s| !s.is_empty())
        .or_else(|| user_id_from_jwt(&token))?;
    Some(Session {
        cookie: session_cookie(&auth_id, &token)?,
        email: super::vscdb_value(&conn, "cursorAuth/cachedEmail").map(clean),
        plan: super::vscdb_value(&conn, "cursorAuth/stripeMembershipType").map(clean),
    })
}

pub fn parse_usage(v: &serde_json::Value) -> (Vec<LimitWindow>, Option<String>) {
    let resets = iso_ms(v.get("billingCycleEnd"));
    let mut out = Vec::new();
    if let Some(total) = num(v.pointer("/individualUsage/plan/totalPercentUsed")) {
        let mut w = LimitWindow::percent("included", "Included usage", total).resets(resets);
        w.span_ms = Some(30 * crate::limits_strip::DAY_MS);
        out.push(w);
    }
    if let Some(api) = num(v.pointer("/individualUsage/plan/apiPercentUsed")).filter(|p| *p > 0.0) {
        out.push(LimitWindow::percent("api", "API usage", api).resets(resets));
    }
    if let Some(od) = v.pointer("/individualUsage/onDemand") {
        let enabled = od.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
        let limit = num(od.get("limit")).unwrap_or(0.0);
        if let (true, Some(used)) = (enabled && limit > 0.0, num(od.get("used"))) {
            // Cursor reports money in cents.
            out.push(
                LimitWindow::ratio("on-demand", "On demand", used / 100.0, limit / 100.0, "usd")
                    .resets(resets),
            );
        }
    }
    let note = (v.get("isUnlimited").and_then(|x| x.as_bool()) == Some(true))
        .then(|| "unlimited plan".to_string());
    (out, note)
}

#[async_trait::async_trait]
impl UsageProvider for Cursor {
    fn id(&self) -> &str {
        "cursor"
    }
    fn label(&self) -> &'static str {
        "Cursor"
    }

    async fn detect(&self) -> bool {
        state_db().map(|p| p.is_file()).unwrap_or(false)
    }

    async fn read(&self) -> Result<Reading, ReadError> {
        let session = tokio::task::spawn_blocking(session)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| ReadError::NeedsAuth("Cursor is not signed in".into()))?;
        let resp = http()
            .get(ENDPOINT)
            .header("Cookie", session.cookie.expose())
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(net_err)?;
        let v = json_or_error(resp).await?;
        let (windows, note) = parse_usage(&v);
        Ok(Reading {
            windows,
            note,
            plan: v
                .get("membershipType")
                .and_then(|x| x.as_str())
                .map(str::to_string)
                .or(session.plan),
            account: session.email,
            ..Default::default()
        })
    }
}
