//! Tauri commands exposed to the Settings → Cookies UI.

use serde::{Deserialize, Serialize};

use super::parsers;
use super::platform::PlatformKind;
use super::storage::{self, AccountEntry, CookieRegistry, IngestSource};

#[derive(Debug, Serialize)]
pub struct CookieListResponse {
    pub registry: CookieRegistry,
    pub cookies_dir: String,
}

#[derive(Debug, Deserialize)]
pub struct ImportRequest {
    pub content: String,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub source_label: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
}

fn domain_from_url(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw).ok()?;
    let host = parsed.host_str()?;
    let root = super::platform::root_domain_of(host);
    if root.is_empty() {
        None
    } else {
        Some(root)
    }
}

#[derive(Debug, Serialize)]
pub struct ImportResponse {
    pub buckets_written: Vec<BucketWrite>,
}

#[derive(Debug, Serialize)]
pub struct BucketWrite {
    pub domain: String,
    pub cookie_count: usize,
    pub platform_kind: String,
}

#[derive(Debug, Deserialize)]
pub struct ReadRequest {
    pub domain: String,
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReadResponse {
    pub content: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct ClearRequest {
    pub domain: String,
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClearBatchRequest {
    pub items: Vec<ClearRequest>,
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub domain: String,
    #[serde(default)]
    pub slug: Option<String>,
    pub new_alias: String,
}

#[derive(Debug, Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct ClearBatchResponse {
    pub ok: bool,
    pub cleared: usize,
}

const DEFAULT_SLUG: &str = "_default";

#[tauri::command]
pub async fn cookies_list() -> Result<CookieListResponse, String> {
    let registry = storage::load_registry();
    let cookies_dir = storage::cookies_root().to_string_lossy().into_owned();
    Ok(CookieListResponse {
        registry,
        cookies_dir,
    })
}

#[tauri::command]
pub async fn cookies_read(request: ReadRequest) -> Result<ReadResponse, String> {
    let slug = request.slug.as_deref().unwrap_or(DEFAULT_SLUG);
    let content = storage::read_account_file(&request.domain, slug).map_err(|e| e.to_string())?;
    let path = storage::account_file(&request.domain, slug)
        .to_string_lossy()
        .into_owned();
    Ok(ReadResponse { content, path })
}

#[tauri::command]
pub async fn cookies_import(request: ImportRequest) -> Result<ImportResponse, String> {
    let source_domain = request.source_url.as_deref().and_then(domain_from_url);
    let cookies = match source_domain.as_deref() {
        Some(domain) => parsers::parse_for_domain(&request.content, domain),
        None => parsers::parse(&request.content),
    }
    .map_err(|e| e.to_string())?;
    if cookies.is_empty() {
        return Err("No cookies found in payload".to_string());
    }
    let label = request
        .source_label
        .unwrap_or_else(|| "Manual import".to_string());
    let written = storage::ingest_batch(
        &cookies,
        IngestSource {
            source_url: request.source_url,
            source_label: label,
            alias_hint: request.alias,
        },
    )
    .map_err(|e| e.to_string())?;

    let buckets_written = written
        .into_iter()
        .map(|(domain, cookie_count)| {
            let platform_kind = PlatformKind::from_domain(&domain).as_str().to_string();
            BucketWrite {
                domain,
                cookie_count,
                platform_kind,
            }
        })
        .collect();
    Ok(ImportResponse { buckets_written })
}

#[tauri::command]
pub async fn cookies_clear(request: ClearRequest) -> Result<OkResponse, String> {
    let slug = request.slug.as_deref().unwrap_or(DEFAULT_SLUG);
    storage::move_to_trash(&request.domain, slug).map_err(|e| e.to_string())?;
    Ok(OkResponse { ok: true })
}

#[tauri::command]
pub async fn cookies_clear_batch(request: ClearBatchRequest) -> Result<ClearBatchResponse, String> {
    let mut cleared = 0usize;
    for item in request.items {
        let slug = item.slug.as_deref().unwrap_or(DEFAULT_SLUG);
        storage::move_to_trash(&item.domain, slug).map_err(|e| e.to_string())?;
        cleared += 1;
    }
    Ok(ClearBatchResponse { ok: true, cleared })
}

#[tauri::command]
pub async fn cookies_rename(request: RenameRequest) -> Result<OkResponse, String> {
    let slug = request.slug.as_deref().unwrap_or(DEFAULT_SLUG);
    if request.new_alias.trim().is_empty() {
        return Err("Alias cannot be empty".to_string());
    }
    storage::rename_account(&request.domain, slug, request.new_alias.trim())
        .map_err(|e| e.to_string())?;
    Ok(OkResponse { ok: true })
}

#[derive(Debug, Serialize)]
pub struct AccountsForUrlResponse {
    pub domain: String,
    pub accounts: Vec<AccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct CookieJsonEntry {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub expires: i64,
    #[serde(rename = "httpOnly")]
    pub http_only: bool,
}

#[derive(Debug, Deserialize)]
pub struct ReadAsJsonRequest {
    pub domain: String,
    #[serde(default)]
    pub slug: Option<String>,
}

#[tauri::command]
pub async fn cookies_read_as_json(
    request: ReadAsJsonRequest,
) -> Result<Vec<CookieJsonEntry>, String> {
    let slug = request.slug.as_deref().unwrap_or(DEFAULT_SLUG);
    let content = storage::read_account_file(&request.domain, slug).map_err(|e| e.to_string())?;
    let parsed = parsers::parse_netscape(&content).map_err(|e| e.to_string())?;
    Ok(parsed
        .into_iter()
        .map(|c| CookieJsonEntry {
            name: c.name,
            value: c.value,
            domain: c.domain,
            path: c.path,
            secure: c.secure,
            expires: c.expires,
            http_only: c.http_only,
        })
        .collect())
}

#[tauri::command]
pub async fn cookies_accounts_for_url(url: String) -> Result<AccountsForUrlResponse, String> {
    let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
    let host = parsed.host_str().unwrap_or("");
    let root = super::platform::root_domain_of(host);
    if root.is_empty() {
        return Ok(AccountsForUrlResponse {
            domain: String::new(),
            accounts: Vec::new(),
        });
    }
    let registry = storage::load_registry();
    let accounts = registry
        .buckets
        .get(&root)
        .map(|b| b.accounts.clone())
        .unwrap_or_default();
    Ok(AccountsForUrlResponse {
        domain: root,
        accounts,
    })
}

#[derive(Debug, Deserialize)]
pub struct ImportFileRequest {
    pub path: String,
    #[serde(default)]
    pub alias: Option<String>,
}

#[tauri::command]
pub async fn cookies_import_file(request: ImportFileRequest) -> Result<ImportResponse, String> {
    let path = std::path::Path::new(&request.path);
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", request.path))?;
    let cookies = parsers::parse(&content).map_err(|e| e.to_string())?;
    if cookies.is_empty() {
        return Err("No cookies found in file".to_string());
    }
    let filename = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| request.path.clone());
    let label = format!("File import: {}", filename);
    let written = storage::ingest_batch(
        &cookies,
        IngestSource {
            source_url: None,
            source_label: label,
            alias_hint: request.alias,
        },
    )
    .map_err(|e| e.to_string())?;
    let buckets_written = written
        .into_iter()
        .map(|(domain, cookie_count)| {
            let platform_kind = PlatformKind::from_domain(&domain).as_str().to_string();
            BucketWrite {
                domain,
                cookie_count,
                platform_kind,
            }
        })
        .collect();
    Ok(ImportResponse { buckets_written })
}

#[derive(Debug, Deserialize)]
pub struct ExportToRequest {
    pub domain: String,
    #[serde(default)]
    pub slug: Option<String>,
    pub destination_path: String,
}

#[derive(Debug, Deserialize)]
pub struct AddAccountRequest {
    pub domain: String,
    pub alias: String,
    pub content: String,
    #[serde(default)]
    pub source_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AddAccountResponse {
    pub slug: String,
    pub cookie_count: usize,
}

#[tauri::command]
pub async fn cookies_add_account(request: AddAccountRequest) -> Result<AddAccountResponse, String> {
    if request.alias.trim().is_empty() {
        return Err("Alias is required for a new account".to_string());
    }
    let cookies =
        parsers::parse_for_domain(&request.content, &request.domain).map_err(|e| e.to_string())?;
    if cookies.is_empty() {
        return Err("No cookies found in payload".to_string());
    }
    let (slug, cookie_count) = storage::ingest_to_account(
        &request.domain,
        request.alias.trim(),
        &cookies,
        IngestSource {
            source_url: request.source_url,
            source_label: "Manual add (multi-account)".to_string(),
            alias_hint: Some(request.alias.trim().to_string()),
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(AddAccountResponse { slug, cookie_count })
}

#[tauri::command]
pub async fn cookies_export_to(request: ExportToRequest) -> Result<OkResponse, String> {
    let slug = request.slug.as_deref().unwrap_or(DEFAULT_SLUG);
    let content = storage::read_account_file(&request.domain, slug).map_err(|e| e.to_string())?;
    std::fs::write(&request.destination_path, content)
        .map_err(|e| format!("Failed to write {}: {e}", request.destination_path))?;
    Ok(OkResponse { ok: true })
}

const COOKIE_FRESH_DAYS: i64 = 7;
const COOKIE_EXPIRE_DAYS: i64 = 28;

#[derive(Debug, Serialize)]
pub struct CookieHealthItem {
    pub domain: String,
    pub slug: String,
    pub status: String,
    pub age_days: i64,
    pub expires_in_days: i64,
    pub cookie_count: usize,
}

#[derive(Debug, Serialize)]
pub struct CookieHealthResponse {
    pub items: Vec<CookieHealthItem>,
    pub fresh_days: i64,
    pub expire_days: i64,
}

#[tauri::command]
pub async fn cookies_health() -> Result<CookieHealthResponse, String> {
    let registry = storage::load_registry();
    let now = storage::current_unix_ms();
    let mut items = Vec::new();
    for (domain, bucket) in &registry.buckets {
        for acc in &bucket.accounts {
            let age_days = (now - acc.captured_at_ms).max(0) / 86_400_000;
            let status = if age_days >= COOKIE_EXPIRE_DAYS {
                "expired"
            } else if age_days >= COOKIE_FRESH_DAYS {
                "stale"
            } else {
                "fresh"
            };
            items.push(CookieHealthItem {
                domain: domain.clone(),
                slug: acc.slug.clone(),
                status: status.to_string(),
                age_days,
                expires_in_days: (COOKIE_EXPIRE_DAYS - age_days).max(0),
                cookie_count: acc.cookie_count,
            });
        }
    }
    Ok(CookieHealthResponse {
        items,
        fresh_days: COOKIE_FRESH_DAYS,
        expire_days: COOKIE_EXPIRE_DAYS,
    })
}

#[derive(Debug, Deserialize)]
pub struct CookieTestRequest {
    pub url: String,
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CookieTestResponse {
    pub ok: bool,
    pub message: String,
}

fn cookie_names_from_netscape(content: &str) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("# Netscape") {
            continue;
        }
        let effective = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if effective.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = effective.split('\t').collect();
        if cols.len() >= 7 {
            names.insert(cols[5].to_string());
        }
    }
    names
}

fn test_x_twitter_cookie(slug: Option<&str>) -> CookieTestResponse {
    let path = storage::account_path_for_consumer("x.com", slug)
        .or_else(|| storage::account_path_for_consumer("twitter.com", slug));
    let Some(path) = path else {
        return CookieTestResponse {
            ok: false,
            message: "No X/Twitter cookie account found".to_string(),
        };
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return CookieTestResponse {
            ok: false,
            message: "Could not read X/Twitter cookie file".to_string(),
        };
    };
    let names = cookie_names_from_netscape(&content);
    let has_auth = names.contains("auth_token");
    let has_csrf = names.contains("ct0");
    if has_auth && has_csrf {
        CookieTestResponse {
            ok: true,
            message: "ok".to_string(),
        }
    } else {
        let mut missing = Vec::new();
        if !has_auth {
            missing.push("auth_token");
        }
        if !has_csrf {
            missing.push("ct0");
        }
        CookieTestResponse {
            ok: false,
            message: format!("Missing X/Twitter cookie fields: {}", missing.join(", ")),
        }
    }
}

#[tauri::command]
pub async fn cookies_test(request: CookieTestRequest) -> Result<CookieTestResponse, String> {
    if let Some(domain) = domain_from_url(&request.url) {
        if matches!(domain.as_str(), "x.com" | "twitter.com") {
            return Ok(test_x_twitter_cookie(request.slug.as_deref()));
        }
    }

    let ytdlp = crate::core::ytdlp::find_ytdlp_cached()
        .await
        .ok_or_else(|| "yt-dlp unavailable".to_string())?;
    let url = request.url.clone();
    let flags = vec!["--no-warnings".to_string()];
    let result = omniget_core::core::log_hook::CURRENT_COOKIE_SLUG
        .scope(request.slug.clone(), async move {
            crate::core::ytdlp::get_video_info(&ytdlp, &url, &flags).await
        })
        .await;
    match result {
        Ok(_) => Ok(CookieTestResponse {
            ok: true,
            message: "ok".to_string(),
        }),
        Err(e) => {
            let raw = e.to_string();
            let summary = raw
                .lines()
                .find(|l| l.contains("ERROR"))
                .unwrap_or_else(|| raw.lines().next().unwrap_or("failed"))
                .trim()
                .chars()
                .take(240)
                .collect::<String>();
            Ok(CookieTestResponse {
                ok: false,
                message: summary,
            })
        }
    }
}
