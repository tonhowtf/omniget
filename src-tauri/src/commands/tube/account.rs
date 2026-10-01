//! The YouTube account comes from the OmniGet cookie vault (`youtube.com`,
//! primary slot): whoever already imported cookies through the extension or
//! Settings is signed in without doing anything. `tube_login` fills the same
//! slot from a login window, and a background loop keeps the session alive by
//! folding the cookies Google rotates back into the file.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use omniget_core::core::tube::innertube::CookieJar;
use omniget_core::core::tube::parse;
use omniget_core::core::tube::Account;
use serde::Serialize;
use tauri::AppHandle;

use crate::commands::auth_webview::{open_auth_webview, AuthWebviewRequest};
use crate::extension_storage::ExtensionCookie;

use super::http;

const DOMAIN: &str = "youtube.com";
const SLOT: &str = "_default";
const ACCOUNT_TTL: Duration = Duration::from_secs(10 * 60);
const REFRESH_EVERY: Duration = Duration::from_secs(20 * 60);

/// Kaset's flow (`accounts.google.com` -> `youtube.com/signin` -> home).
const LOGIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&uilel=3&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26app%3Ddesktop%26hl%3Den%26next%3Dhttps%253A%252F%252Fwww.youtube.com%252F&hl=en";

/// The embedded webview has WebAuthn but no passkey entitlement: Google
/// offers a passkey and dead-ends. Hiding the API makes it fall back to the
/// password flow (Kaset, `LoginPasskeySuppression.swift`, MIT).
const HIDE_PASSKEYS: &str = r#"(function () {
  "use strict";
  try {
    delete window.PublicKeyCredential;
    Object.defineProperty(window, "PublicKeyCredential", { value: undefined, writable: false, configurable: false });
  } catch (e) {}
  try {
    var credentials = window.navigator && window.navigator.credentials;
    if (!credentials) return;
    var proto = Object.getPrototypeOf(credentials);
    var wrap = function (original) {
      if (typeof original !== "function") return original;
      return function (options) {
        if (options && options.publicKey) {
          return Promise.reject(new DOMException("Passkeys are not available in this app.", "NotAllowedError"));
        }
        return original.apply(this, arguments);
      };
    };
    proto.get = wrap(proto.get);
    proto.create = wrap(proto.create);
  } catch (e) {}
})();"#;

#[derive(Debug, Clone, Serialize)]
pub struct TubeStatus {
    pub account: Account,
    /// Proxy server base (`http://127.0.0.1:PORT`).
    pub server: String,
    pub ytdlp: bool,
    pub ffmpeg: bool,
}

static SAVE_LOCK: Mutex<()> = Mutex::new(());
static ACCOUNT: Mutex<Option<(Account, Instant, String)>> = Mutex::new(None);

/// The account jar, or `None` when there is no signed-in youtube.com
/// cookie file or the user turned `use_account` off.
pub fn jar() -> Option<CookieJar> {
    if !http::settings().use_account {
        return None;
    }
    let path = crate::cookies::account_path_for_consumer(DOMAIN, None)?;
    let text = std::fs::read_to_string(path).ok()?;
    let jar = CookieJar::from_netscape(&text);
    jar.is_logged_in().then_some(jar)
}

pub fn is_logged_in() -> bool {
    jar().is_some()
}

/// Identifies the signed-in account without exposing it (cache key).
fn identity(jar: &CookieJar) -> String {
    use sha2::{Digest, Sha256};
    let id = jar
        .get("SAPISID")
        .or_else(|| jar.get("__Secure-3PAPISID"))
        .unwrap_or("");
    let digest = Sha256::digest(id.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Writes rotated cookies back into the vault file: existing entries get the
/// new value and expiry, new ones are added. Nothing is removed (a sign-out
/// goes through `tube_logout`).
pub fn save_jar(jar: &CookieJar) {
    let Ok(_g) = SAVE_LOCK.lock() else {
        return;
    };
    let Some(path) = crate::cookies::account_path_for_consumer(DOMAIN, None) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut cookies) = crate::cookies::parsers::parse_netscape(&text) else {
        return;
    };
    let same_domain = |a: &str, b: &str| a.trim_start_matches('.') == b.trim_start_matches('.');
    let mut changed = false;
    for e in &jar.entries {
        // the vault bucket only holds youtube.com cookies
        if crate::cookies::root_domain_of(&e.domain) != DOMAIN {
            continue;
        }
        match cookies
            .iter_mut()
            .find(|c| c.name == e.name && same_domain(&c.domain, &e.domain))
        {
            Some(c) => {
                if c.value != e.value || (e.expires > 0 && c.expires != e.expires) {
                    c.value = e.value.clone();
                    if e.expires > 0 {
                        c.expires = e.expires;
                    }
                    changed = true;
                }
            }
            None => {
                cookies.push(ExtensionCookie {
                    domain: e.domain.clone(),
                    http_only: e.name.starts_with("__Secure-") || e.name.ends_with("SID"),
                    path: e.path.clone(),
                    secure: e.secure,
                    expires: e.expires,
                    name: e.name.clone(),
                    value: e.value.clone(),
                    host_only: Some(!e.include_sub),
                    same_site: None,
                });
                changed = true;
            }
        }
    }
    if !changed {
        return;
    }
    match crate::cookies::storage::write_account_file(DOMAIN, SLOT, &cookies) {
        Ok(_) => tracing::debug!("[tube] account cookies rotated ({} entries)", cookies.len()),
        Err(e) => tracing::warn!("[tube] could not save rotated cookies: {e}"),
    }
}

fn forget_account() {
    if let Ok(mut g) = ACCOUNT.lock() {
        *g = None;
    }
}

fn cached_account(key: &str) -> Option<Account> {
    let g = ACCOUNT.lock().ok()?;
    let (acc, at, k) = g.as_ref()?;
    (k == key && at.elapsed() < ACCOUNT_TTL).then(|| acc.clone())
}

/// Premium flag of the last account menu read, when known.
pub fn cached_premium() -> Option<bool> {
    ACCOUNT.lock().ok()?.as_ref()?.0.premium
}

fn signed_out() -> Account {
    Account {
        logged_in: false,
        ..Default::default()
    }
}

async fn fetch_account() -> Result<Account, String> {
    let Some(jar) = jar() else {
        forget_account();
        return Ok(signed_out());
    };
    let key = identity(&jar);
    if let Some(acc) = cached_account(&key) {
        return Ok(acc);
    }
    let json = http::web("account/account_menu", |c, v, hl, gl| {
        http::body_with(c, v, hl, gl, serde_json::json!({}))
    })
    .await?;
    let mut acc = parse::web::account_menu(&json);
    // the menu of a dead session has no name: treat it as signed out
    if acc.name.is_none() && acc.channel_id.is_none() {
        acc.logged_in = false;
        return Ok(acc);
    }
    acc.logged_in = true;
    acc.slug = Some(SLOT.to_string());
    if let Ok(mut g) = ACCOUNT.lock() {
        *g = Some((acc.clone(), Instant::now(), key));
    }
    Ok(acc)
}

#[tauri::command]
pub async fn tube_status(app: AppHandle) -> Result<TubeStatus, String> {
    let account = match jar() {
        Some(jar) => cached_account(&identity(&jar)).unwrap_or(Account {
            logged_in: true,
            slug: Some(SLOT.to_string()),
            ..Default::default()
        }),
        None => signed_out(),
    };
    let server = super::server::ensure(&app)
        .await
        .map(|i| super::server::base(&i))
        .unwrap_or_default();
    let ytdlp = omniget_core::core::ytdlp::find_ytdlp_cached()
        .await
        .is_some();
    let ffmpeg = omniget_core::core::ffmpeg::is_ffmpeg_available().await;
    Ok(TubeStatus {
        account,
        server,
        ytdlp,
        ffmpeg,
    })
}

#[tauri::command]
pub async fn tube_account() -> Result<Account, String> {
    fetch_account().await
}

fn to_extension_cookie(c: crate::commands::auth_webview::AuthCookie) -> ExtensionCookie {
    ExtensionCookie {
        host_only: Some(!c.domain.starts_with('.')),
        domain: c.domain,
        http_only: c.http_only,
        path: if c.path.is_empty() {
            "/".into()
        } else {
            c.path
        },
        secure: c.secure,
        expires: 0,
        name: c.name,
        value: c.value,
        same_site: None,
    }
}

#[tauri::command]
pub async fn tube_login(app: AppHandle) -> Result<Account, String> {
    let request = AuthWebviewRequest {
        url: LOGIN_URL.to_string(),
        title: "OmniTube".to_string(),
        cookie_domains: vec![".youtube.com".to_string(), ".google.com".to_string()],
        success_url_contains: Some("www.youtube.com".to_string()),
        wait_for_cookie: Some("SAPISID|__Secure-3PAPISID".to_string()),
        initialization_script: Some(HIDE_PASSKEYS.to_string()),
        width: Some(480.0),
        height: Some(720.0),
    };
    let result = open_auth_webview(app, request).await?;
    let cookies: Vec<ExtensionCookie> = result
        .cookies
        .into_iter()
        .map(to_extension_cookie)
        .filter(|c| crate::cookies::root_domain_of(&c.domain) == DOMAIN)
        .collect();
    let probe = CookieJar::from_netscape(&netscape(&cookies));
    if !probe.is_logged_in() {
        return Err("sign-in did not finish".to_string());
    }
    // the primary slot is what yt-dlp and OmniTube read; keep the previous
    // one recoverable in the vault trash
    if crate::cookies::account_path_for_consumer(DOMAIN, None).is_some() {
        let _ = crate::cookies::storage::move_to_trash(DOMAIN, SLOT);
    }
    crate::cookies::ingest_batch(
        &cookies,
        crate::cookies::IngestSource {
            source_url: Some("https://www.youtube.com/".to_string()),
            source_label: "OmniTube".to_string(),
            alias_hint: None,
        },
    )
    .map_err(|e| format!("could not save the account: {e}"))?;
    forget_account();
    super::streams::forget_account_state();
    fetch_account().await
}

/// Netscape text for a cookie list, only to probe it with `CookieJar`.
fn netscape(cookies: &[ExtensionCookie]) -> String {
    let mut s = String::from("# Netscape HTTP Cookie File\n");
    for c in cookies {
        let domain = if c.domain.starts_with('.') {
            c.domain.clone()
        } else {
            format!(".{}", c.domain)
        };
        s.push_str(&format!(
            "{domain}\tTRUE\t{}\t{}\t0\t{}\t{}\n",
            c.path,
            if c.secure { "TRUE" } else { "FALSE" },
            c.name,
            c.value
        ));
    }
    s
}

#[tauri::command]
pub async fn tube_logout() -> Result<(), String> {
    crate::cookies::storage::move_to_trash(DOMAIN, SLOT)
        .map_err(|e| format!("could not remove the account: {e}"))?;
    forget_account();
    super::streams::forget_account_state();
    Ok(())
}

/// One keep-alive: loads youtube.com with the account and keeps whatever
/// cookies Google rotated (YTubic `run_refresh_loop`, limusic `session.rs`).
async fn refresh_session() -> Result<(), String> {
    let Some(jar) = jar() else {
        return Ok(());
    };
    let resp = http::client()
        .get("https://www.youtube.com/")
        .header(reqwest::header::USER_AGENT, http::BROWSER_UA)
        .header(reqwest::header::COOKIE, jar.header())
        .send()
        .await
        .map_err(|_| "network error".to_string())?;
    http::absorb_cookies("www.youtube.com", resp.headers());
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    forget_account();
    Ok(())
}

/// Every 20 min while signed in, refresh the session cookies. Called once
/// from the app setup.
pub fn spawn_session_refresh(_app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            if let Err(e) = refresh_session().await {
                tracing::debug!("[tube] session refresh failed: {e}");
            }
            tokio::time::sleep(REFRESH_EVERY).await;
        }
    });
}
