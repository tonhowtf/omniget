//! `auth_connection_status` / `auth_connection_request`: an MCP client learns
//! whether a login is configured in OmniGet and can ask the PERSON to connect
//! one. The client never receives a cookie, token, cookie name, account alias
//! or file path, and nothing here performs a login: a request only becomes a
//! visible prompt in the desktop app that the person resolves or denies.
use super::{
    policy::{self, Principal},
    ToolDef,
};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

/// A pending request that nobody answers stops being actionable after this.
pub const REQUEST_TTL_SECS: i64 = 900;
/// Pending requests one client may hold at a time (all targets together).
pub const MAX_PENDING: i64 = 5;
/// Longest client-supplied reason kept; it is untrusted text shown to a person.
pub const REASON_MAX_CHARS: usize = 200;
/// Platform keys a client may ask about (`cookies::PlatformKind` minus
/// `generic`; a generic site is addressed by `host`).
pub const PLATFORMS: &[&str] = &[
    "youtube",
    "youtube_music",
    "soundcloud",
    "spotify",
    "twitch",
    "instagram",
    "x_twitter",
    "vimeo",
    "tiktok",
    "bilibili",
    "douyin",
    "reddit",
    "pinterest",
    "bluesky",
];
const STATES: &[&str] = &["pending", "resolved", "denied", "expired"];

pub fn tools() -> Vec<ToolDef> {
    let platform = json!({"type":"string","enum":PLATFORMS});
    let host = json!({"type":"string","minLength":3,"maxLength":253});
    vec![
        ToolDef {
            name: "auth_connection_status",
            description: "Whether a login for ONE platform or host is configured in OmniGet, without any secret. Pass exactly one of `platform`, `host` or `requestId` (to poll an auth_connection_request). States: not_connected, unknown (configured, validity not verified; OmniGet has no stored proof that a session works), expired (every stored cookie is past its expiry). No cookie values, cookie names, account aliases or paths are returned, and there is no listing of every site with a login. Note `usedByExternalDownloads`: downloads started through this MCP server run in a confined worker that does not read local logins.",
            input_schema: json!({"type":"object","properties":{"platform":platform,"host":host,"requestId":{"type":"string","minLength":1,"maxLength":64}},"required":[]}),
        },
        ToolDef {
            name: "auth_connection_request",
            description: "Ask the person at the computer to connect a login for ONE platform or host inside OmniGet. Creates a pending request shown in the desktop app (and a system notification); the person opens the login settings there and marks it done or denies it. Returns a requestId: poll it with auth_connection_status {requestId}. Pending requests expire after 15 minutes; at most 5 pending per client; an equal pending request is reused. This never logs in automatically, never returns credentials and never exposes a local URL. `reason` is shown to the person as a message from this client (max 200 chars).",
            input_schema: json!({"type":"object","properties":{"platform":platform,"host":host,"reason":{"type":"string","minLength":1,"maxLength":400},"idempotencyKey":{"type":"string","minLength":1,"maxLength":100}},"required":["reason","idempotencyKey"]}),
        },
    ]
}

/// Output schemas for the protocol fixer's `outputSchema` table.
pub fn output_schema(name: &str) -> Option<Value> {
    let request = json!({"type":"object","properties":{
        "requestId":{"type":"string"},
        "state":{"type":"string","enum":STATES},
        "platform":{"type":"string"},
        "host":{"type":["string","null"]},
        "createdAt":{"type":"integer"},
        "expiresAt":{"type":"integer"},
        "updatedAt":{"type":"integer"}
    },"required":["requestId","state","platform","createdAt","expiresAt"]});
    match name {
        "auth_connection_status" => Some(json!({"type":"object","properties":{
            "platform":{"type":"string"},
            "host":{"type":["string","null"]},
            "state":{"type":"string","enum":["not_connected","unknown","expired"]},
            "configured":{"type":"boolean"},
            "sources":{"type":"array","items":{"type":"string","enum":["cookie_manager","global_cookie_file","browser_cookies"]}},
            "accounts":{"type":"array","items":{"type":"object","properties":{
                "accountRef":{"type":"string"},
                "state":{"type":"string","enum":["unknown","expired"]},
                "capturedAt":{"type":"integer"}
            },"required":["accountRef","state","capturedAt"]}},
            "lastVerifiedAt":{"type":"null"},
            "usedByExternalDownloads":{"type":"boolean"},
            "request":{"anyOf":[request.clone(),{"type":"null"}]},
            "note":{"type":"string"}
        },"required":["platform","state","configured","accounts","usedByExternalDownloads"]})),
        "auth_connection_request" => {
            let mut schema = request;
            schema["properties"]["deduplicated"] = json!({"type":"boolean"});
            schema["properties"]["requiresLocalInteraction"] =
                json!({"type":"boolean","const":true});
            schema["properties"]["nextPollAfterMs"] = json!({"type":"integer"});
            schema["properties"]["localAction"] = json!({"type":"string"});
            Some(schema)
        }
        _ => None,
    }
}

// ── Target ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// A `PLATFORMS` key, or `generic` for a host without first-class support.
    pub platform: String,
    /// Registrable domain, only for a generic host.
    pub host: Option<String>,
}
impl Target {
    fn key(&self) -> String {
        match &self.host {
            Some(h) => format!("host:{h}"),
            None => format!("platform:{}", self.platform),
        }
    }
    /// Cookie Manager buckets (registrable domains) that belong to it.
    fn matches_bucket(&self, bucket: &str) -> bool {
        match &self.host {
            Some(h) => bucket == h,
            None => {
                let want = if self.platform == "youtube_music" {
                    "youtube"
                } else {
                    self.platform.as_str()
                };
                crate::cookies::PlatformKind::from_domain(bucket).as_str() == want
            }
        }
    }
}

pub(super) fn valid_host(raw: &str) -> Option<String> {
    let h = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    let ok = (3..=253).contains(&h.len())
        && h.contains('.')
        && !h.starts_with('.')
        && !h.starts_with('-')
        && h.split('.').all(|l| !l.is_empty() && l.len() <= 63)
        && h.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.')
        // A literal address is a device, not a platform login.
        && h.parse::<std::net::IpAddr>().is_err()
        && !h.rsplit('.').next().is_some_and(|tld| tld.bytes().all(|c| c.is_ascii_digit()));
    ok.then_some(h)
}

/// Exactly one of `platform` / `host`.
pub fn target(a: &Value) -> Result<Target, String> {
    match (a["platform"].as_str(), a["host"].as_str()) {
        (Some(p), None) if PLATFORMS.contains(&p) => Ok(Target {
            platform: p.into(),
            host: None,
        }),
        (None, Some(h)) => {
            let host = valid_host(h).ok_or("INVALID_ARGUMENT: host")?;
            let kind = crate::cookies::PlatformKind::from_domain(&host);
            if kind.as_str() == "generic" {
                Ok(Target {
                    platform: "generic".into(),
                    host: Some(crate::cookies::root_domain_of(&host)),
                })
            } else {
                // music.youtube.com is its own platform key; everything else by root.
                Ok(Target {
                    platform: kind.as_str().into(),
                    host: None,
                })
            }
        }
        (Some(_), None) => Err("INVALID_ARGUMENT: platform".into()),
        _ => Err("INVALID_ARGUMENT: pass exactly one of platform or host".into()),
    }
}

// ── Status (pure) ──────────────────────────────────────────────────────

/// Global yt-dlp cookie sources configured in Settings (presence only).
#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalSources {
    pub cookie_file: bool,
    pub browser: bool,
}

/// True when the Netscape file has at least one persistent cookie and all of
/// them expired. Session cookies (expiry 0) make the answer unknown (false).
pub fn all_expired(netscape: &str, now_secs: i64) -> bool {
    let mut any = false;
    for raw in netscape.lines() {
        let line = raw.trim();
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 7 {
            continue;
        }
        match cols[4].trim().parse::<i64>() {
            Ok(0) | Err(_) => return false,
            Ok(exp) if exp > now_secs => return false,
            Ok(_) => any = true,
        }
    }
    any
}

/// Opaque, stable reference to one stored account; never its alias or slug.
fn account_ref(bucket: &str, slug: &str) -> String {
    use sha2::{Digest, Sha256};
    let d = format!(
        "{:x}",
        Sha256::digest(format!("omniget-account\0{bucket}\0{slug}").as_bytes())
    );
    format!("acct_{}", &d[..16])
}

/// `read_file(bucket, slug)` returns the account's Netscape text (read
/// locally only to derive `expired`; its content never leaves this function).
pub fn status_for(
    t: &Target,
    registry: &crate::cookies::CookieRegistry,
    globals: GlobalSources,
    now_secs: i64,
    read_file: &dyn Fn(&str, &str) -> Option<String>,
) -> Value {
    let mut accounts = vec![];
    for (bucket, entry) in &registry.buckets {
        if !t.matches_bucket(bucket) {
            continue;
        }
        for acc in &entry.accounts {
            let expired =
                read_file(bucket, &acc.slug).is_some_and(|text| all_expired(&text, now_secs));
            accounts.push(json!({
                "accountRef": account_ref(bucket, &acc.slug),
                "state": if expired { "expired" } else { "unknown" },
                "capturedAt": acc.captured_at_ms,
            }));
        }
    }
    let mut sources = vec![];
    if !accounts.is_empty() {
        sources.push("cookie_manager");
    }
    if globals.cookie_file {
        sources.push("global_cookie_file");
    }
    if globals.browser {
        sources.push("browser_cookies");
    }
    let configured = !sources.is_empty();
    // Global sources may cover any site: their validity is unknown, never expired.
    let state = if !configured {
        "not_connected"
    } else if globals.cookie_file
        || globals.browser
        || accounts.iter().any(|a| a["state"] == "unknown")
    {
        "unknown"
    } else {
        "expired"
    };
    json!({
        "platform": t.platform,
        "host": t.host,
        "state": state,
        "configured": configured,
        "sources": sources,
        "accounts": accounts,
        "lastVerifiedAt": null,
        "usedByExternalDownloads": false,
        "note": "Validity is not verified by OmniGet; unknown is not a working session. Downloads started through this MCP server run in a confined worker that does not use local logins; a login configured here applies to downloads started in the OmniGet app.",
    })
}

// ── Requests (durable, per principal) ──────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub request_id: String,
    pub state: String,
    pub platform: String,
    pub host: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub updated_at: i64,
}

/// What the desktop shows the person (includes the client name and reason).
#[derive(Debug, Clone, Serialize)]
pub struct UiRequest {
    pub id: String,
    pub client: String,
    pub platform: String,
    pub host: Option<String>,
    pub reason: String,
    pub created_at: i64,
    pub expires_at: i64,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn db() -> Result<rusqlite::Connection, String> {
    let c = policy::db()?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS auth_requests(id TEXT PRIMARY KEY, principal TEXT NOT NULL, target TEXT NOT NULL, platform TEXT NOT NULL, host TEXT, reason TEXT NOT NULL, state TEXT NOT NULL, created INTEGER NOT NULL, expires INTEGER NOT NULL, updated INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS auth_requests_principal ON auth_requests(principal, state);",
    )
    .map_err(|_| "AUTH_REQUESTS_UNAVAILABLE")?;
    Ok(c)
}

fn expire_stale(c: &rusqlite::Connection, now: i64) -> Result<(), String> {
    c.execute(
        "UPDATE auth_requests SET state='expired', updated=expires WHERE state='pending' AND expires<=?1",
        [now],
    )
    .map_err(|_| "AUTH_REQUESTS_UNAVAILABLE")?;
    Ok(())
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Request> {
    Ok(Request {
        request_id: r.get(0)?,
        state: r.get(1)?,
        platform: r.get(2)?,
        host: r.get(3)?,
        created_at: r.get(4)?,
        expires_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}
const COLUMNS: &str = "id,state,platform,host,created,expires,updated";

/// Untrusted client text for a person: one line, redacted, bounded.
pub fn clean_reason(raw: &str) -> String {
    let flat: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let redacted = crate::core::flight_recorder::redact(flat.trim());
    let mut out: String = redacted.chars().take(REASON_MAX_CHARS).collect();
    if redacted.chars().count() > REASON_MAX_CHARS {
        out.push('…');
    }
    out
}

/// Returns the request and whether it reused an equal pending one.
pub fn create_request(
    p: &Principal,
    t: &Target,
    reason: &str,
    now: i64,
) -> Result<(Request, bool), String> {
    policy::active(p)?;
    let mut c = db()?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "AUTH_REQUESTS_BUSY")?;
    expire_stale(&tx, now)?;
    let existing = tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM auth_requests WHERE principal=?1 AND target=?2 AND state='pending'"),
            params![p.id, t.key()],
            row,
        )
        .optional()
        .map_err(|_| "AUTH_REQUESTS_UNAVAILABLE")?;
    if let Some(found) = existing {
        return Ok((found, true));
    }
    let pending: i64 = tx
        .query_row(
            "SELECT count(*) FROM auth_requests WHERE principal=?1 AND state='pending'",
            [&p.id],
            |r| r.get(0),
        )
        .map_err(|_| "AUTH_REQUESTS_UNAVAILABLE")?;
    if pending >= MAX_PENDING {
        return Err(
            "AUTH_REQUEST_LIMIT: too many pending requests; wait for the person or for expiry"
                .into(),
        );
    }
    let request = Request {
        request_id: format!("authreq_{}", uuid::Uuid::new_v4().simple()),
        state: "pending".into(),
        platform: t.platform.clone(),
        host: t.host.clone(),
        created_at: now,
        expires_at: now + REQUEST_TTL_SECS,
        updated_at: now,
    };
    tx.execute(
        "INSERT INTO auth_requests(id,principal,target,platform,host,reason,state,created,expires,updated) VALUES(?1,?2,?3,?4,?5,?6,'pending',?7,?8,?7)",
        params![request.request_id, p.id, t.key(), request.platform, request.host, clean_reason(reason), now, request.expires_at],
    )
    .map_err(|_| "AUTH_REQUESTS_UNAVAILABLE")?;
    tx.commit().map_err(|_| "AUTH_REQUESTS_UNAVAILABLE")?;
    Ok((request, false))
}

/// A principal only ever sees its own requests; another client's id is
/// indistinguishable from a missing one.
pub fn get_request(p: &Principal, id: &str, now: i64) -> Result<Request, String> {
    policy::active(p)?;
    let c = db()?;
    expire_stale(&c, now)?;
    c.query_row(
        &format!("SELECT {COLUMNS} FROM auth_requests WHERE id=?1 AND principal=?2"),
        params![id, p.id],
        row,
    )
    .optional()
    .map_err(|_| "AUTH_REQUESTS_UNAVAILABLE")?
    .ok_or_else(|| "AUTH_REQUEST_NOT_FOUND".into())
}

/// Latest request of this principal for a target (for the status answer).
fn latest_for(p: &Principal, t: &Target, now: i64) -> Result<Option<Request>, String> {
    let c = db()?;
    expire_stale(&c, now)?;
    c.query_row(
        &format!("SELECT {COLUMNS} FROM auth_requests WHERE principal=?1 AND target=?2 ORDER BY created DESC LIMIT 1"),
        params![p.id, t.key()],
        row,
    )
    .optional()
    .map_err(|_| "AUTH_REQUESTS_UNAVAILABLE".into())
}

/// Trusted desktop only: pending requests of live clients.
pub fn pending_for_ui(now: i64) -> Result<Vec<UiRequest>, String> {
    let c = db()?;
    expire_stale(&c, now)?;
    let mut stmt = c
        .prepare(
            "SELECT a.id,c.name,a.platform,a.host,a.reason,a.created,a.expires FROM auth_requests a JOIN clients c ON c.id=a.principal WHERE a.state='pending' AND c.revoked=0 ORDER BY a.created",
        )
        .map_err(|_| "AUTH_REQUESTS_UNAVAILABLE")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(UiRequest {
                id: r.get(0)?,
                client: r.get(1)?,
                platform: r.get(2)?,
                host: r.get(3)?,
                reason: r.get(4)?,
                created_at: r.get(5)?,
                expires_at: r.get(6)?,
            })
        })
        .map_err(|_| "AUTH_REQUESTS_UNAVAILABLE")?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| "AUTH_REQUESTS_UNAVAILABLE".into())
}

/// Trusted desktop only: the person's answer. Only a live pending request of
/// a non-revoked client changes; everything else is `AUTH_REQUEST_NOT_PENDING`.
pub fn answer(id: &str, outcome: &str, now: i64) -> Result<(), String> {
    if !matches!(outcome, "resolved" | "denied") {
        return Err("INVALID_OUTCOME".into());
    }
    let c = db()?;
    expire_stale(&c, now)?;
    let changed = c
        .execute(
            "UPDATE auth_requests SET state=?2, updated=?3 WHERE id=?1 AND state='pending' AND expires>?3 AND principal IN (SELECT id FROM clients WHERE revoked=0)",
            params![id, outcome, now],
        )
        .map_err(|_| "AUTH_REQUESTS_UNAVAILABLE")?;
    if changed == 1 {
        Ok(())
    } else {
        Err("AUTH_REQUEST_NOT_PENDING".into())
    }
}

// ── MCP dispatch ───────────────────────────────────────────────────────

fn globals(app: &tauri::AppHandle) -> GlobalSources {
    let s = crate::storage::config::load_settings(app);
    GlobalSources {
        cookie_file: !s.download.cookie_file.trim().is_empty(),
        browser: !s.advanced.cookies_from_browser.trim().is_empty(),
    }
}

fn local_status(app: &tauri::AppHandle, t: &Target, now: i64) -> Value {
    let registry = crate::cookies::load_registry();
    let read =
        |bucket: &str, slug: &str| crate::cookies::storage::read_account_file(bucket, slug).ok();
    status_for(t, &registry, globals(app), now, &read)
}

/// Called by `downloads::call` after schema validation and scope check.
pub async fn call(
    app: &tauri::AppHandle,
    p: &Principal,
    name: &str,
    a: &Value,
) -> Result<Value, String> {
    if !policy::allowed(p, name) {
        return Err("TOOL_NOT_AUTHORIZED".into());
    }
    let now = now_secs();
    match name {
        "auth_connection_status" => {
            if let Some(id) = a["requestId"].as_str() {
                if a.get("platform").is_some() || a.get("host").is_some() {
                    return Err(
                        "INVALID_ARGUMENT: pass exactly one of platform, host or requestId".into(),
                    );
                }
                let request = get_request(p, id, now)?;
                let t = Target {
                    platform: request.platform.clone(),
                    host: request.host.clone(),
                };
                let mut status = local_status(app, &t, now);
                status["request"] = serde_json::to_value(&request).map_err(|_| "SERIALIZATION")?;
                return Ok(status);
            }
            let t = target(a)?;
            let mut status = local_status(app, &t, now);
            status["request"] =
                serde_json::to_value(latest_for(p, &t, now)?).map_err(|_| "SERIALIZATION")?;
            Ok(status)
        }
        "auth_connection_request" => {
            let t = target(a)?;
            let reason = a["reason"].as_str().unwrap_or("");
            if reason.trim().is_empty() {
                return Err("INVALID_ARGUMENT: reason".into());
            }
            let key = a["idempotencyKey"]
                .as_str()
                .ok_or("IDEMPOTENCY_KEY_REQUIRED")?;
            if let Some(receipt) = policy::reserve(p, name, key, a)? {
                return Ok(receipt);
            }
            let (request, deduplicated) = match create_request(p, &t, reason, now) {
                Ok(v) => v,
                Err(e) => {
                    // Known refusal with no effect: the key stays usable.
                    let _ = policy::release(p, name, key);
                    return Err(e);
                }
            };
            if !deduplicated {
                notify(app, p, &request);
            }
            let mut value = serde_json::to_value(&request).map_err(|_| "SERIALIZATION")?;
            value["deduplicated"] = json!(deduplicated);
            value["requiresLocalInteraction"] = json!(true);
            value["nextPollAfterMs"] = json!(5000);
            value["localAction"] = json!("The person resolves this in OmniGet (Settings > Cookies). Poll auth_connection_status with requestId.");
            policy::finish(p, name, key, &value)?;
            Ok(value)
        }
        _ => Err("TOOL_NOT_AUTHORIZED".into()),
    }
}

/// Visible prompt: an in-app event for the banner and a system notification.
/// The notification names the client and platform, never the reason text.
fn notify(app: &tauri::AppHandle, p: &Principal, r: &Request) {
    use tauri::Emitter;
    let _ = app.emit(
        "mcp-auth-request",
        json!({"id": r.request_id, "client": p.name, "platform": r.platform, "host": r.host}),
    );
    use tauri_plugin_notification::NotificationExt;
    let what = r.host.clone().unwrap_or_else(|| r.platform.clone());
    let _ = app
        .notification()
        .builder()
        .title("OmniGet")
        .body(format!(
            "{} asks you to connect {} in OmniGet.",
            p.name.chars().take(80).collect::<String>(),
            what
        ))
        .show();
}

// ── Trusted Tauri surface ──────────────────────────────────────────────

#[tauri::command]
pub fn tool_mcp_auth_requests() -> Result<Vec<UiRequest>, String> {
    pending_for_ui(now_secs())
}

#[tauri::command]
pub fn tool_mcp_auth_request_answer(
    app: tauri::AppHandle,
    id: String,
    outcome: String,
) -> Result<(), String> {
    answer(&id, &outcome, now_secs())?;
    use tauri::Emitter;
    let _ = app.emit("mcp-auth-request", json!({"id": id, "state": outcome}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cookies::{AccountEntry, BucketEntry, CookieRegistry};

    fn isolated() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("omniget-mcp-auth-{}", uuid::Uuid::new_v4()));
        policy::TEST_DIR.with(|p| *p.borrow_mut() = Some(dir.clone()));
        dir
    }
    fn registry() -> CookieRegistry {
        let mut r = CookieRegistry::default();
        r.buckets.insert(
            "instagram.com".into(),
            BucketEntry {
                platform_kind: "instagram".into(),
                accounts: vec![AccountEntry {
                    slug: "joao-private".into(),
                    alias: "Joao private".into(),
                    source_url: Some("https://instagram.com/?token=SYNTHETIC_SECRET".into()),
                    source_label: Some("SYNTHETIC_LABEL".into()),
                    captured_at_ms: 1_000,
                    cookie_count: 3,
                    last_used_at_ms: Some(2_000),
                }],
            },
        );
        r.buckets.insert(
            "example.org".into(),
            BucketEntry {
                platform_kind: "generic".into(),
                accounts: vec![AccountEntry {
                    slug: "_default".into(),
                    alias: "default".into(),
                    source_url: None,
                    source_label: None,
                    captured_at_ms: 5,
                    cookie_count: 1,
                    last_used_at_ms: None,
                }],
            },
        );
        r
    }

    #[test]
    fn target_takes_exactly_one_platform_or_host() {
        assert_eq!(
            target(&json!({"platform":"instagram"})).unwrap().platform,
            "instagram"
        );
        assert_eq!(
            target(&json!({"host":"www.instagram.com"})).unwrap(),
            Target {
                platform: "instagram".into(),
                host: None
            }
        );
        assert_eq!(
            target(&json!({"host":"cdn.example.org"})).unwrap(),
            Target {
                platform: "generic".into(),
                host: Some("example.org".into())
            }
        );
        for bad in [
            json!({}),
            json!({"platform":"instagram","host":"x.com"}),
            json!({"platform":"generic"}),
            json!({"host":"https://x.com"}),
            json!({"host":"127.0.0.1"}),
            json!({"host":"localhost"}),
            json!({"host":"a..b"}),
        ] {
            assert!(target(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn status_never_carries_secrets_names_or_aliases() {
        let cookie = "# Netscape HTTP Cookie File\n.instagram.com\tTRUE\t/\tTRUE\t9999999999\tsessionid\tSYNTHETIC_SECRET\n";
        let read = |_: &str, _: &str| Some(cookie.to_string());
        let t = target(&json!({"platform":"instagram"})).unwrap();
        let v = status_for(&t, &registry(), GlobalSources::default(), 100, &read);
        let text = v.to_string();
        for leak in [
            "SYNTHETIC_SECRET",
            "sessionid",
            "joao",
            "Joao",
            "SYNTHETIC_LABEL",
            "token",
            "cookie_count",
            "/",
        ] {
            if leak == "/" {
                assert!(
                    !v["accounts"].to_string().contains('/'),
                    "path-like data leaked"
                );
                continue;
            }
            assert!(!text.contains(leak), "{leak} leaked: {text}");
        }
        assert_eq!(v["state"], "unknown");
        assert_eq!(v["configured"], true);
        assert_eq!(v["usedByExternalDownloads"], false);
        assert_eq!(v["accounts"].as_array().unwrap().len(), 1);
        assert!(v["accounts"][0]["accountRef"]
            .as_str()
            .unwrap()
            .starts_with("acct_"));
        // A different platform does not see Instagram's account.
        let x = target(&json!({"platform":"x_twitter"})).unwrap();
        let v = status_for(&x, &registry(), GlobalSources::default(), 100, &read);
        assert_eq!(v["state"], "not_connected");
        assert!(v["accounts"].as_array().unwrap().is_empty());
        // A global source is unknown, never valid.
        let v = status_for(
            &x,
            &registry(),
            GlobalSources {
                cookie_file: true,
                browser: false,
            },
            100,
            &read,
        );
        assert_eq!(v["state"], "unknown");
        assert_eq!(v["sources"], json!(["global_cookie_file"]));
        let schema = output_schema("auth_connection_status").unwrap();
        for key in schema["required"].as_array().unwrap() {
            assert!(v.get(key.as_str().unwrap()).is_some(), "missing {key}");
        }
    }

    #[test]
    fn expiry_is_derived_from_cookie_dates_and_session_cookies_stay_unknown() {
        let old = ".instagram.com\tTRUE\t/\tTRUE\t50\tsessionid\tX\n#HttpOnly_.instagram.com\tTRUE\t/\tTRUE\t60\tds_user_id\tY\n";
        assert!(all_expired(old, 100));
        assert!(!all_expired(old, 55));
        assert!(!all_expired(
            ".instagram.com\tTRUE\t/\tTRUE\t0\tsessionid\tX\n",
            100
        ));
        assert!(!all_expired("# only comments\n", 100));
        let read = |_: &str, _: &str| Some(old.to_string());
        let t = target(&json!({"platform":"instagram"})).unwrap();
        let v = status_for(&t, &registry(), GlobalSources::default(), 100, &read);
        assert_eq!(v["state"], "expired");
    }

    #[test]
    fn request_lifecycle_pending_resolved_denied_expired_and_isolation() {
        let dir = isolated();
        let a = policy::create("A".into(), vec!["auth".into()])
            .unwrap()
            .principal;
        let b = policy::create("B".into(), vec!["auth".into()])
            .unwrap()
            .principal;
        let ig = target(&json!({"platform":"instagram"})).unwrap();
        let (r, dedup) = create_request(
            &a,
            &ig,
            "Login required\nfor reel; Cookie: SYNTHETIC_SECRET",
            1_000,
        )
        .unwrap();
        assert!(!dedup);
        assert_eq!(r.state, "pending");
        assert_eq!(r.expires_at, 1_000 + REQUEST_TTL_SECS);
        // Equal pending request is reused.
        let (again, dedup) = create_request(&a, &ig, "again", 1_001).unwrap();
        assert!(dedup);
        assert_eq!(again.request_id, r.request_id);
        // B cannot see or poll A's request; the UI shows the client and a clean reason.
        assert_eq!(
            get_request(&b, &r.request_id, 1_002).unwrap_err(),
            "AUTH_REQUEST_NOT_FOUND"
        );
        let ui = pending_for_ui(1_002).unwrap();
        assert_eq!(ui.len(), 1);
        assert_eq!(ui[0].client, "A");
        assert!(!ui[0].reason.contains('\n'));
        assert!(
            !ui[0].reason.contains("SYNTHETIC_SECRET"),
            "{}",
            ui[0].reason
        );
        // Resolved.
        answer(&r.request_id, "resolved", 1_003).unwrap();
        assert_eq!(
            get_request(&a, &r.request_id, 1_004).unwrap().state,
            "resolved"
        );
        assert_eq!(
            answer(&r.request_id, "denied", 1_005).unwrap_err(),
            "AUTH_REQUEST_NOT_PENDING"
        );
        assert!(answer(&r.request_id, "pending", 1_005).is_err());
        // Denied.
        let x = target(&json!({"platform":"x_twitter"})).unwrap();
        let (d, _) = create_request(&a, &x, "need X", 2_000).unwrap();
        answer(&d.request_id, "denied", 2_001).unwrap();
        assert_eq!(
            get_request(&a, &d.request_id, 2_002).unwrap().state,
            "denied"
        );
        // Expired: never answerable afterwards.
        let (e, _) = create_request(&b, &x, "need X", 3_000).unwrap();
        assert_eq!(
            get_request(&b, &e.request_id, 3_000 + REQUEST_TTL_SECS)
                .unwrap()
                .state,
            "expired"
        );
        assert_eq!(
            answer(&e.request_id, "resolved", 3_000 + REQUEST_TTL_SECS + 1).unwrap_err(),
            "AUTH_REQUEST_NOT_PENDING"
        );
        // Limit per client.
        for (i, p) in PLATFORMS.iter().take(MAX_PENDING as usize).enumerate() {
            let t = target(&json!({"platform":p})).unwrap();
            create_request(&b, &t, "r", 10_000 + i as i64).unwrap();
        }
        let extra = target(&json!({"host":"example.org"})).unwrap();
        assert!(create_request(&b, &extra, "r", 10_100)
            .unwrap_err()
            .starts_with("AUTH_REQUEST_LIMIT"));
        // A revoked client's requests leave the desktop list and cannot be answered.
        policy::revoke(&b.id).unwrap();
        assert!(pending_for_ui(10_200)
            .unwrap()
            .iter()
            .all(|r| r.client != "B"));
        assert!(create_request(&b, &extra, "r", 10_300).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scopes_gate_both_tools() {
        let without = Principal {
            id: "a".into(),
            name: "A".into(),
            scopes: vec!["discover".into(), "diagnostics".into()],
        };
        let with = Principal {
            id: "b".into(),
            name: "B".into(),
            scopes: vec!["auth".into()],
        };
        for name in ["auth_connection_status", "auth_connection_request"] {
            assert!(!policy::allowed(&without, name));
            assert!(policy::allowed(&with, name));
        }
        assert!(!policy::allowed(&with, "download_status"));
    }
}
