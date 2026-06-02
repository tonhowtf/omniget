use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::{ApiClient, BilibiliError, Result};

pub mod captcha;
pub mod qr;
pub mod sms;

const NAV_URL: &str = "https://api.bilibili.com/x/web-interface/nav";
const LOGOUT_URL: &str = "https://passport.bilibili.com/login/exit/v2";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub uname: String,
    pub mid: u64,
    pub face: Option<String>,
    pub is_vip: bool,
    pub vip_due_secs: Option<u64>,
}

pub async fn fetch_account_info(client: &ApiClient) -> Result<AccountInfo> {
    let raw = client.get_json(NAV_URL).await?;
    let code = raw.get("code").and_then(Value::as_i64).unwrap_or(-1);
    let data = if code == 0 {
        raw.get("data")
    } else {
        Some(&raw)
    }
    .ok_or(BilibiliError::ContentUnavailable)?;
    let is_login = data
        .get("isLogin")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !is_login {
        return Err(BilibiliError::NotLoggedIn);
    }
    let uname = data
        .get("uname")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mid = data.get("mid").and_then(Value::as_u64).unwrap_or(0);
    let face = data.get("face").and_then(Value::as_str).map(String::from);
    let vip_status = data.get("vipStatus").and_then(Value::as_u64).unwrap_or(0);
    let vip_due_ms = data
        .get("vip_due_date")
        .or_else(|| data.get("vipDueDate"))
        .and_then(Value::as_u64);
    Ok(AccountInfo {
        uname,
        mid,
        face,
        is_vip: vip_status == 1,
        vip_due_secs: vip_due_ms.map(|ms| ms / 1000),
    })
}

pub async fn logout(client: &ApiClient, csrf: &str) -> Result<()> {
    let url = format!("{}?biliCSRF={}", LOGOUT_URL, urlencoding::encode(csrf));
    let _ = client.post_form(&url, &[("biliCSRF", csrf)]).await?;
    Ok(())
}

pub fn slug_from_uname(uname: &str) -> String {
    let lower = uname.to_lowercase();
    let cleaned: String = lower
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "user".to_string()
    } else {
        cleaned
    }
}

pub fn persist_account(
    cookies: &[(String, String)],
    uname: &str,
    source_label: &str,
) -> std::result::Result<String, String> {
    use crate::extension_storage::ExtensionCookie;

    let slug = slug_from_uname(uname);
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let expires = now_unix + 60 * 60 * 24 * 365;

    let entries: Vec<ExtensionCookie> = cookies
        .iter()
        .map(|(name, value)| ExtensionCookie {
            domain: ".bilibili.com".to_string(),
            http_only: name == "SESSDATA",
            path: "/".to_string(),
            secure: true,
            expires,
            name: name.clone(),
            value: value.clone(),
            host_only: None,
            same_site: None,
        })
        .collect();

    crate::cookies::storage::write_account_file("bilibili.com", &slug, &entries)
        .map_err(|e| e.to_string())?;

    let mut registry = crate::cookies::storage::load_registry();
    let now_ms = crate::cookies::storage::current_unix_ms();
    let bucket = registry
        .buckets
        .entry("bilibili.com".to_string())
        .or_insert_with(|| crate::cookies::storage::BucketEntry {
            platform_kind: "bilibili".to_string(),
            accounts: Vec::new(),
        });
    bucket.platform_kind = "bilibili".to_string();

    let alias = format!("{} · {}", uname, source_label);
    if let Some(existing) = bucket.accounts.iter_mut().find(|a| a.slug == slug) {
        existing.captured_at_ms = now_ms;
        existing.cookie_count = entries.len();
        existing.source_label = Some(source_label.to_string());
        existing.alias = alias;
    } else {
        bucket.accounts.push(crate::cookies::storage::AccountEntry {
            slug: slug.clone(),
            alias,
            source_url: Some("https://www.bilibili.com".to_string()),
            source_label: Some(source_label.to_string()),
            captured_at_ms: now_ms,
            cookie_count: entries.len(),
            last_used_at_ms: Some(now_ms),
        });
    }

    crate::cookies::storage::save_registry(&registry).map_err(|e| e.to_string())?;
    Ok(slug)
}
