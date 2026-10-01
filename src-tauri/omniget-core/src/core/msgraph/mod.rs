//! Microsoft 365 through Microsoft Graph, signed in as the person (delegated
//! permissions, device-code flow): Teams chats and channels to export, the
//! calendar's online meetings, and the official meeting transcripts.
//!
//! The desktop app, the CLI (`omniget teams`, what Claude Code calls) and the
//! MCP tools share one sign-in: the config sits in `<app_data>/msgraph.json`
//! and the refresh token in the shared encrypted store (`secrets::shared_*`),
//! because the app's keychain is not visible to the CLI.
//!
//! OmniGet ships no Entra app of its own: the person registers one (public
//! client, device code allowed) and pastes its client id. Tenants that allow
//! user consent need nothing else; channel messages and transcripts need an
//! admin to approve their permissions, so they are opt-in.

pub mod calendar;
pub mod chats;
pub mod html;
pub mod transcripts;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::secrets;

const CONFIG_FILE: &str = "msgraph.json";
const REFRESH_ACCOUNT: &str = "msgraph:refresh";
const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// Scopes any user can consent to.
pub const BASE_SCOPES: &[&str] = &[
    "offline_access",
    "User.Read",
    "Chat.Read",
    "Calendars.Read",
    "OnlineMeetings.Read",
    "Team.ReadBasic.All",
    "Channel.ReadBasic.All",
];
/// Scopes that need an admin's approval in most tenants.
pub const ADMIN_SCOPES: &[&str] = &[
    "ChannelMessage.Read.All",
    "OnlineMeetingTranscript.Read.All",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphConfig {
    /// Application (client) id of the person's Entra app registration.
    pub client_id: String,
    /// "organizations", "common" or a tenant id / domain.
    pub tenant: String,
    /// Also ask for channel messages and meeting transcripts.
    pub admin_scopes: bool,
    /// Filled after sign-in.
    pub account: Option<Account>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub upn: String,
    pub id: String,
}

impl GraphConfig {
    fn tenant(&self) -> &str {
        let t = self.tenant.trim();
        if t.is_empty() {
            "organizations"
        } else {
            t
        }
    }

    pub fn scopes(&self) -> String {
        let mut s: Vec<&str> = BASE_SCOPES.to_vec();
        if self.admin_scopes {
            s.extend_from_slice(ADMIN_SCOPES);
        }
        s.join(" ")
    }
}

fn config_path() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("OMNIGET_MSGRAPH_DIR") {
        if !dir.trim().is_empty() {
            return Some(std::path::PathBuf::from(dir).join(CONFIG_FILE));
        }
    }
    crate::core::paths::app_data_dir().map(|d| d.join(CONFIG_FILE))
}

pub fn load_config() -> GraphConfig {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_config(c: &GraphConfig) -> anyhow::Result<()> {
    let p = config_path().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(p, serde_json::to_vec_pretty(c)?)?;
    Ok(())
}

/// Changes the app registration. A different client, tenant or scope set
/// needs a new sign-in, so the old session is dropped.
pub fn configure(client_id: &str, tenant: &str, admin_scopes: bool) -> anyhow::Result<GraphConfig> {
    let client_id = client_id.trim();
    if !client_id.is_empty() && uuid::Uuid::parse_str(client_id).is_err() {
        bail!("The client id is the Application (client) ID of your app registration, a GUID");
    }
    let mut c = load_config();
    let changed = c.client_id != client_id
        || c.tenant.trim() != tenant.trim()
        || c.admin_scopes != admin_scopes;
    c.client_id = client_id.to_string();
    c.tenant = tenant.trim().to_string();
    c.admin_scopes = admin_scopes;
    if changed {
        c.account = None;
        let _ = secrets::shared_delete(REFRESH_ACCOUNT);
        clear_cache();
    }
    save_config(&c)?;
    Ok(c)
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub configured: bool,
    pub signed_in: bool,
    pub account: Option<Account>,
    pub client_id: String,
    pub tenant: String,
    pub admin_scopes: bool,
}

pub fn status() -> Status {
    let c = load_config();
    let signed_in = c.account.is_some()
        && secrets::shared_get(REFRESH_ACCOUNT)
            .ok()
            .flatten()
            .is_some();
    Status {
        configured: !c.client_id.is_empty(),
        signed_in,
        account: if signed_in { c.account.clone() } else { None },
        tenant: c.tenant().to_string(),
        client_id: c.client_id,
        admin_scopes: c.admin_scopes,
    }
}

pub fn sign_out() -> anyhow::Result<()> {
    let mut c = load_config();
    c.account = None;
    save_config(&c)?;
    secrets::shared_delete(REFRESH_ACCOUNT).map_err(|e| anyhow!(e))?;
    clear_cache();
    Ok(())
}

// ------------------------------------------------------------------ tokens

struct Cached {
    token: String,
    until: Instant,
}

static ACCESS: Mutex<Option<Cached>> = Mutex::new(None);

fn clear_cache() {
    *ACCESS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn http() -> anyhow::Result<reqwest::Client> {
    Ok(
        crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .user_agent("OmniGet")
            .timeout(Duration::from_secs(120))
            .build()?,
    )
}

fn login_url(c: &GraphConfig, path: &str) -> String {
    format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/{path}",
        c.tenant()
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub message: String,
    pub expires_in: u64,
    pub interval: u64,
    #[serde(skip_serializing)]
    pub device_code: String,
}

/// Step 1: the code the person types at `verification_uri`.
pub async fn start_sign_in() -> anyhow::Result<DeviceCode> {
    let c = load_config();
    if c.client_id.is_empty() {
        bail!("Set the client id of your Entra app registration first");
    }
    let resp = http()?
        .post(login_url(&c, "devicecode"))
        .form(&[
            ("client_id", c.client_id.as_str()),
            ("scope", c.scopes().as_str()),
        ])
        .send()
        .await?;
    let v: Value = resp.json().await?;
    if let Some(e) = v["error"].as_str() {
        bail!(
            "{}",
            explain_aad(e, v["error_description"].as_str().unwrap_or(""))
        );
    }
    Ok(serde_json::from_value(v)?)
}

/// Step 2: waits until the person finished (or the code expired) and keeps
/// the session. Returns the signed-in account.
pub async fn finish_sign_in(code: &DeviceCode) -> anyhow::Result<Account> {
    let c = load_config();
    let client = http()?;
    let deadline = Instant::now() + Duration::from_secs(code.expires_in.max(60));
    let mut interval = code.interval.max(2);
    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if Instant::now() > deadline {
            bail!("The sign-in code expired; start again");
        }
        let v: Value = client
            .post(login_url(&c, "token"))
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", c.client_id.as_str()),
                ("device_code", code.device_code.as_str()),
            ])
            .send()
            .await?
            .json()
            .await?;
        match v["error"].as_str() {
            None => {
                store_tokens(&v)?;
                let me = get("/me?$select=id,displayName,userPrincipalName").await?;
                let account = Account {
                    name: me["displayName"].as_str().unwrap_or("").to_string(),
                    upn: me["userPrincipalName"].as_str().unwrap_or("").to_string(),
                    id: me["id"].as_str().unwrap_or("").to_string(),
                };
                let mut c = load_config();
                c.account = Some(account.clone());
                save_config(&c)?;
                return Ok(account);
            }
            Some("authorization_pending") => {}
            Some("slow_down") => interval += 5,
            Some(e) => bail!(
                "{}",
                explain_aad(e, v["error_description"].as_str().unwrap_or(""))
            ),
        }
    }
}

fn store_tokens(v: &Value) -> anyhow::Result<()> {
    let token = v["access_token"]
        .as_str()
        .ok_or_else(|| anyhow!("no access token in the answer"))?;
    let secs = v["expires_in"].as_u64().unwrap_or(3600);
    if let Some(r) = v["refresh_token"].as_str() {
        secrets::shared_set(REFRESH_ACCOUNT, r).map_err(|e| anyhow!(e))?;
    }
    *ACCESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Cached {
        token: token.to_string(),
        until: Instant::now() + Duration::from_secs(secs.saturating_sub(120)),
    });
    Ok(())
}

async fn access_token(force_refresh: bool) -> anyhow::Result<String> {
    if !force_refresh {
        if let Some(c) = ACCESS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if Instant::now() < c.until {
                return Ok(c.token.clone());
            }
        }
    }
    let c = load_config();
    let refresh = secrets::shared_get(REFRESH_ACCOUNT)
        .map_err(|e| anyhow!(e))?
        .ok_or_else(|| anyhow!("Not signed in to Microsoft 365: run `omniget teams login` or connect in OmniGet > Meetings > Settings"))?;
    let v: Value = http()?
        .post(login_url(&c, "token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", c.client_id.as_str()),
            ("refresh_token", refresh.as_str()),
            ("scope", c.scopes().as_str()),
        ])
        .send()
        .await?
        .json()
        .await?;
    if let Some(e) = v["error"].as_str() {
        if e == "invalid_grant" {
            let _ = sign_out();
        }
        bail!(
            "{}",
            explain_aad(e, v["error_description"].as_str().unwrap_or(""))
        );
    }
    store_tokens(&v)?;
    Ok(v["access_token"].as_str().unwrap_or_default().to_string())
}

/// Microsoft's AADSTS errors in words a person can act on.
fn explain_aad(code: &str, desc: &str) -> String {
    let first = desc.lines().next().unwrap_or("").trim();
    let hint = if desc.contains("AADSTS65001") || desc.contains("consent") {
        " An admin has to approve the app's permissions (or turn off the admin-approved permissions in settings)."
    } else if desc.contains("AADSTS7000218") {
        " In the app registration, set Authentication > Allow public client flows to Yes."
    } else if desc.contains("AADSTS700016") {
        " The client id is not an app registration in this tenant."
    } else if code == "invalid_grant" {
        " The session ended: sign in again."
    } else {
        ""
    };
    format!("Microsoft sign-in: {code}: {first}{hint}")
}

// ------------------------------------------------------------------ requests

fn absolute(path_or_url: &str) -> String {
    if path_or_url.starts_with("https://") {
        path_or_url.to_string()
    } else {
        format!("{GRAPH}{path_or_url}")
    }
}

/// GET with the session: refreshes once on 401, waits on 429/503 as Graph
/// asks (`Retry-After`), up to three times.
pub async fn get_raw(path_or_url: &str, accept: Option<&str>) -> anyhow::Result<reqwest::Response> {
    let url = absolute(path_or_url);
    let client = http()?;
    let mut refreshed = false;
    for attempt in 0..4 {
        let token = access_token(false).await?;
        let mut req = client
            .get(&url)
            .bearer_auth(&token)
            .header("Prefer", "outlook.timezone=\"UTC\"");
        if let Some(a) = accept {
            req = req.header("Accept", a);
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        if status == 401 && !refreshed {
            refreshed = true;
            access_token(true).await?;
            continue;
        }
        if (status == 429 || status == 503 || status == 504) && attempt < 3 {
            let wait = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(2 << attempt)
                .min(60);
            tokio::time::sleep(Duration::from_secs(wait)).await;
            continue;
        }
        if !resp.status().is_success() {
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            let code = body["error"]["code"].as_str().unwrap_or("");
            let msg = body["error"]["message"].as_str().unwrap_or("");
            let hint = if status == 403 {
                " (the app lacks this permission, or an admin has to approve it)"
            } else {
                ""
            };
            bail!("Microsoft Graph answered HTTP {status} {code}: {msg}{hint}");
        }
        return Ok(resp);
    }
    bail!("Microsoft Graph kept throttling the request; try again in a minute")
}

pub async fn get(path_or_url: &str) -> anyhow::Result<Value> {
    Ok(get_raw(path_or_url, None).await?.json().await?)
}

/// Follows `@odata.nextLink` until `max` items (0 = no limit) or `stop`
/// says the rest is not wanted.
pub async fn get_paged(
    path: &str,
    max: usize,
    mut stop: impl FnMut(&Value) -> bool,
) -> anyhow::Result<Vec<Value>> {
    let mut out = Vec::new();
    let mut next = Some(absolute(path));
    while let Some(url) = next.take() {
        let page = get(&url).await?;
        for item in page["value"].as_array().into_iter().flatten() {
            if stop(item) {
                return Ok(out);
            }
            out.push(item.clone());
            if max > 0 && out.len() >= max {
                return Ok(out);
            }
        }
        next = page["@odata.nextLink"].as_str().map(str::to_string);
    }
    Ok(out)
}

/// `7d`, `12h`, `30m`, `2w` or an RFC 3339 date → the instant that far back.
pub fn parse_since(s: &str) -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
    let s = s.trim();
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(d.with_timezone(&chrono::Utc));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc());
    }
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: i64 = num
        .parse()
        .map_err(|_| anyhow!("use 30m, 12h, 7d, 2w or a date like 2026-09-01"))?;
    let dur = match unit {
        "m" => chrono::Duration::minutes(n),
        "h" => chrono::Duration::hours(n),
        "d" | "" => chrono::Duration::days(n),
        "w" => chrono::Duration::weeks(n),
        _ => bail!("use 30m, 12h, 7d, 2w or a date like 2026-09-01"),
    };
    Ok(chrono::Utc::now() - dur)
}
