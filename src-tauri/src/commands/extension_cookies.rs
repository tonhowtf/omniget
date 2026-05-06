use std::collections::BTreeMap;

use serde::Serialize;

use crate::extension_storage::extension_cookie_file_path;

#[derive(Debug, Serialize)]
pub struct ExtensionCookiesStatus {
    pub file_exists: bool,
    pub file_path: String,
    pub last_modified_secs: Option<i64>,
    pub age_seconds: Option<i64>,
    pub total_cookies: i64,
    pub domains: Vec<String>,
}

#[tauri::command]
pub async fn extension_cookies_status() -> Result<ExtensionCookiesStatus, String> {
    let path = extension_cookie_file_path();
    let file_path = path.to_string_lossy().into_owned();
    if !path.exists() {
        return Ok(ExtensionCookiesStatus {
            file_exists: false,
            file_path,
            last_modified_secs: None,
            age_seconds: None,
            total_cookies: 0,
            domains: Vec::new(),
        });
    }
    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    let last_modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64);
    let age = match (now, last_modified) {
        (Some(n), Some(m)) => Some(n - m),
        _ => None,
    };
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut total = 0i64;
    let mut domains: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let effective = if line.starts_with("#HttpOnly_") {
            &line["#HttpOnly_".len()..]
        } else if line.starts_with('#') {
            continue;
        } else {
            line
        };
        let parts: Vec<&str> = effective.split('\t').collect();
        if parts.len() < 7 {
            continue;
        }
        total += 1;
        let domain = parts[0].trim_start_matches('.').to_lowercase();
        domains.insert(domain);
    }
    Ok(ExtensionCookiesStatus {
        file_exists: true,
        file_path,
        last_modified_secs: last_modified,
        age_seconds: age,
        total_cookies: total,
        domains: domains.into_iter().collect(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionCookieEntry {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
}

#[derive(Debug, Serialize)]
pub struct ExtensionCookieBundle {
    pub domain: String,
    pub cookies: Vec<ExtensionCookieEntry>,
    pub cookie_string: String,
    pub fetched_at: i64,
    pub source_path: String,
    pub well_known: BTreeMap<String, String>,
}

const WELL_KNOWN_NAMES: &[&str] = &[
    "__Secure-3PAPISID",
    "__Secure-1PAPISID",
    "__Secure-3PSID",
    "__Secure-1PSID",
    "SAPISID",
    "SID",
    "HSID",
    "SSID",
    "APISID",
    "LOGIN_INFO",
    "VISITOR_INFO1_LIVE",
    "PREF",
];

fn cookie_domain_matches(request_domain: &str, cookie_domain: &str, include_subdomains: bool) -> bool {
    if include_subdomains {
        request_domain == cookie_domain
            || request_domain.ends_with(&format!(".{}", cookie_domain))
    } else {
        request_domain == cookie_domain
    }
}

#[tauri::command]
pub async fn read_extension_cookies(
    domain: String,
) -> Result<Option<ExtensionCookieBundle>, String> {
    let path = extension_cookie_file_path();
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let request_domain = domain.trim_start_matches('.').to_lowercase();

    let mut cookies: Vec<ExtensionCookieEntry> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let is_http_only_line = line.starts_with("#HttpOnly_");
        let effective = if is_http_only_line {
            &line["#HttpOnly_".len()..]
        } else if line.starts_with('#') {
            continue;
        } else {
            line
        };
        let parts: Vec<&str> = effective.split('\t').collect();
        if parts.len() < 7 {
            continue;
        }
        let raw_domain = parts[0];
        let include_subdomains = parts[1].eq_ignore_ascii_case("TRUE");
        let cookie_domain = raw_domain.trim_start_matches('.').to_lowercase();
        if !cookie_domain_matches(&request_domain, &cookie_domain, include_subdomains) {
            continue;
        }
        let path_field = parts[2].to_string();
        let secure = parts[3].eq_ignore_ascii_case("TRUE");
        let name = parts[5].to_string();
        let value = parts[6].to_string();
        cookies.push(ExtensionCookieEntry {
            name,
            value,
            domain: raw_domain.to_string(),
            path: path_field,
            secure,
            http_only: is_http_only_line,
        });
    }

    if cookies.is_empty() {
        return Ok(None);
    }

    let mut seen = std::collections::HashSet::new();
    let mut deduped: Vec<&ExtensionCookieEntry> = Vec::new();
    for c in cookies.iter().rev() {
        if seen.insert(c.name.clone()) {
            deduped.push(c);
        }
    }
    deduped.reverse();

    let cookie_string = deduped
        .iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ");

    let mut well_known = BTreeMap::new();
    for c in &deduped {
        if WELL_KNOWN_NAMES.iter().any(|w| *w == c.name) {
            well_known.insert(c.name.clone(), c.value.clone());
        }
    }

    let fetched_at = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    Ok(Some(ExtensionCookieBundle {
        domain: request_domain,
        cookies: deduped.into_iter().cloned().collect(),
        cookie_string,
        fetched_at,
        source_path: path.to_string_lossy().into_owned(),
        well_known,
    }))
}
