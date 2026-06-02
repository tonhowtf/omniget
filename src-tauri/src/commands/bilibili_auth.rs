use serde::{Deserialize, Serialize};

use crate::platforms::bilibili::api::ApiClient;
use crate::platforms::bilibili::auth::{self, qr, sms};
use crate::platforms::bilibili::parser;
use crate::platforms::bilibili::preview;
use crate::platforms::bilibili::url_kind::{self, UrlKind};

fn build_anonymous_client(user_agent: Option<String>) -> Result<ApiClient, String> {
    let mut client = ApiClient::new().map_err(|e| e.i18n_key().to_string())?;
    if let Some(ua) = user_agent.filter(|s| !s.is_empty()) {
        client = client.with_user_agent(ua);
    }
    let _ = crate::platforms::bilibili::cookie::ensure_fresh();
    client = client.with_anonymous_cookies();
    Ok(client)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliWebviewLoginResult {
    pub slug: String,
    pub uname: String,
    pub mid: u64,
    pub is_vip: bool,
}

#[tauri::command]
pub async fn bilibili_webview_login(
    app: tauri::AppHandle,
) -> Result<BilibiliWebviewLoginResult, String> {
    use crate::commands::auth_webview::{open_auth_webview, AuthWebviewRequest};

    let request = AuthWebviewRequest {
        url: "https://passport.bilibili.com/login".to_string(),
        title: "Sign in to Bilibili".to_string(),
        cookie_domains: vec![
            ".bilibili.com".to_string(),
            "passport.bilibili.com".to_string(),
            "www.bilibili.com".to_string(),
        ],
        success_url_contains: None,
        wait_for_cookie: Some("SESSDATA".to_string()),
        initialization_script: None,
        width: Some(480.0),
        height: Some(720.0),
    };

    let result = open_auth_webview(app, request)
        .await
        .map_err(|e| format!("errors.bilibili.network_failed: {}", e))?;

    let mut cookies: Vec<(String, String)> = Vec::new();
    for c in result.cookies {
        if matches!(
            c.name.as_str(),
            "SESSDATA" | "bili_jct" | "DedeUserID" | "DedeUserID__ckMd5" | "sid"
        ) {
            cookies.push((c.name, c.value));
        }
    }
    if cookies.is_empty() {
        return Err("errors.bilibili.not_logged_in".to_string());
    }

    let cookie_header = cookies
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("; ");
    let probe = ApiClient::new()
        .map_err(|e| e.i18n_key().to_string())?
        .with_raw_cookies(cookie_header);
    let info = auth::fetch_account_info(&probe)
        .await
        .map_err(|e| e.i18n_key().to_string())?;
    let slug = auth::persist_account(&cookies, &info.uname, "Webview login")
        .map_err(|_| "errors.bilibili.content_unavailable".to_string())?;
    Ok(BilibiliWebviewLoginResult {
        slug,
        uname: info.uname,
        mid: info.mid,
        is_vip: info.is_vip,
    })
}

#[tauri::command]
pub async fn bilibili_qr_generate(user_agent: Option<String>) -> Result<qr::QrSession, String> {
    let _ = crate::platforms::bilibili::cookie::ensure_fresh().await;
    let client = build_anonymous_client(user_agent)?;
    qr::generate(&client)
        .await
        .map_err(|e| e.i18n_key().to_string())
}

#[tauri::command]
pub async fn bilibili_qr_poll(
    qrcode_key: String,
    user_agent: Option<String>,
) -> Result<qr::QrPollStatus, String> {
    let client = build_anonymous_client(user_agent)?;
    qr::poll(&client, &qrcode_key)
        .await
        .map_err(|e| e.i18n_key().to_string())
}

#[tauri::command]
pub async fn bilibili_captcha_challenge(
    user_agent: Option<String>,
) -> Result<auth::captcha::CaptchaChallenge, String> {
    let client = build_anonymous_client(user_agent)?;
    auth::captcha::request_challenge(&client)
        .await
        .map_err(|e| e.i18n_key().to_string())
}

#[tauri::command]
pub async fn bilibili_sms_send(
    input: sms::SmsSendInput,
    user_agent: Option<String>,
) -> Result<sms::SmsSendResult, String> {
    let client = build_anonymous_client(user_agent)?;
    sms::send(&client, input)
        .await
        .map_err(|e| e.i18n_key().to_string())
}

#[tauri::command]
pub async fn bilibili_sms_verify(
    input: sms::SmsVerifyInput,
    user_agent: Option<String>,
) -> Result<sms::SmsVerifyResult, String> {
    let client = build_anonymous_client(user_agent)?;
    sms::verify(&client, input)
        .await
        .map_err(|e| e.i18n_key().to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliAccountStatus {
    pub logged_in: bool,
    pub slug: Option<String>,
    pub uname: Option<String>,
    pub mid: Option<u64>,
    pub is_vip: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliCodecOption {
    pub codec_id: u32,
    pub label_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliQualityOption {
    pub qn: u32,
    pub label_key: String,
    pub codecs: Vec<BilibiliCodecOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliAudioOption {
    pub qn: u32,
    pub label_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliPreviewSummary {
    pub kind_label_key: String,
    pub item_title: String,
    pub item_count: u32,
    pub qualities: Vec<BilibiliQualityOption>,
    pub audios: Vec<BilibiliAudioOption>,
    pub premium_required: bool,
}

#[tauri::command]
pub async fn bilibili_preview_info(
    url: String,
    slug: Option<String>,
) -> Result<BilibiliPreviewSummary, String> {
    let _ = crate::platforms::bilibili::cookie::ensure_fresh().await;
    let mut client = ApiClient::new().map_err(|e| e.i18n_key().to_string())?;
    client = match slug.as_deref() {
        Some(s) if !s.is_empty() => client.with_account(s),
        _ => client.with_anonymous_cookies(),
    };

    let mut effective_url = url.clone();
    if url_kind::is_b23_short(&effective_url) {
        if let Ok(resolved) = url_kind::resolve_b23(&client, &effective_url).await {
            effective_url = resolved;
        }
    }
    let kind = url_kind::detect(&effective_url).map_err(|e| e.i18n_key().to_string())?;
    let parsed = parser::parse(&client, &kind)
        .await
        .map_err(|e| e.i18n_key().to_string())?;
    let item = parsed
        .items
        .first()
        .ok_or_else(|| "errors.bilibili.content_unavailable".to_string())?;

    let preview_info = preview::fetch(&client, item, &kind).await.ok();

    let mut qualities: Vec<BilibiliQualityOption> = Vec::new();
    let mut audios: Vec<BilibiliAudioOption> = Vec::new();
    let mut premium = false;
    if let Some(info) = preview_info.as_ref() {
        premium = info.premium_required;
        for qn in info.available_qns() {
            let codecs: Vec<BilibiliCodecOption> = info
                .available_codecs_for(qn)
                .into_iter()
                .map(|c| BilibiliCodecOption {
                    codec_id: c,
                    label_key: preview::codec_label(c).to_string(),
                })
                .collect();
            qualities.push(BilibiliQualityOption {
                qn,
                label_key: preview::qn_label(qn).to_string(),
                codecs,
            });
        }
        for qn in info.available_audio_qns() {
            audios.push(BilibiliAudioOption {
                qn,
                label_key: preview::audio_qn_label(qn).to_string(),
            });
        }
    }

    Ok(BilibiliPreviewSummary {
        kind_label_key: kind.label_key().to_string(),
        item_title: item.title.clone(),
        item_count: parsed.items.len() as u32,
        qualities,
        audios,
        premium_required: premium,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliImportedItem {
    pub url: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliImportResult {
    pub total: u32,
    pub items: Vec<BilibiliImportedItem>,
}

async fn run_import(slug: String, kind: UrlKind) -> Result<BilibiliImportResult, String> {
    let _ = crate::platforms::bilibili::cookie::ensure_fresh().await;
    let client = ApiClient::new()
        .map_err(|e| e.i18n_key().to_string())?
        .with_account(&slug);
    let mut all_items: Vec<BilibiliImportedItem> = Vec::new();
    let mut total: u32 = 0;
    let mut current_page: u32 = 1;
    loop {
        let kind_ref = match &kind {
            UrlKind::WatchLater => UrlKind::WatchLater,
            UrlKind::History => UrlKind::History,
            other => other.clone(),
        };
        let parsed = match kind_ref {
            UrlKind::WatchLater => {
                crate::platforms::bilibili::parser::watch_later::parse(&client, current_page).await
            }
            UrlKind::History => {
                crate::platforms::bilibili::parser::history::parse(&client, current_page).await
            }
            _ => return Err("errors.bilibili.content_unavailable".to_string()),
        }
        .map_err(|e| e.i18n_key().to_string())?;
        if let Some(pg) = parsed.pagination.as_ref() {
            total = pg.total_items;
        }
        for it in &parsed.items {
            if let Some(url) = it.url.clone() {
                all_items.push(BilibiliImportedItem {
                    url,
                    title: it.title.clone(),
                    cover_url: it.cover_url.clone(),
                    duration_seconds: it.duration_seconds,
                });
            }
        }
        match parsed.pagination {
            Some(pg) if current_page < pg.total_pages && current_page < 50 => {
                current_page += 1;
            }
            _ => break,
        }
    }
    if total == 0 {
        total = all_items.len() as u32;
    }
    Ok(BilibiliImportResult {
        total,
        items: all_items,
    })
}

#[tauri::command]
pub async fn bilibili_import_watch_later(slug: String) -> Result<BilibiliImportResult, String> {
    let _ = parser::watch_later::parse;
    run_import(slug, UrlKind::WatchLater).await
}

#[tauri::command]
pub async fn bilibili_import_history(slug: String) -> Result<BilibiliImportResult, String> {
    run_import(slug, UrlKind::History).await
}

#[tauri::command]
pub async fn bilibili_account_status(
    slug: Option<String>,
) -> Result<BilibiliAccountStatus, String> {
    let client = match slug.as_deref() {
        Some(s) if !s.is_empty() => ApiClient::new()
            .map_err(|e| e.i18n_key().to_string())?
            .with_account(s),
        _ => {
            return Ok(BilibiliAccountStatus {
                logged_in: false,
                slug: None,
                uname: None,
                mid: None,
                is_vip: false,
            });
        }
    };
    match auth::fetch_account_info(&client).await {
        Ok(info) => Ok(BilibiliAccountStatus {
            logged_in: true,
            slug,
            uname: Some(info.uname),
            mid: Some(info.mid),
            is_vip: info.is_vip,
        }),
        Err(_) => Ok(BilibiliAccountStatus {
            logged_in: false,
            slug: None,
            uname: None,
            mid: None,
            is_vip: false,
        }),
    }
}
