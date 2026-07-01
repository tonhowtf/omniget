use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use crate::models::progress::ProgressUpdate;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType};
use crate::models::settings::AppSettings;
use crate::platforms::traits::PlatformDownloader;

pub mod api;
pub mod auth;
pub mod cdn;
pub mod cookie;
pub mod cover;
pub mod danmaku;
pub mod engine;
pub mod legacy;
pub mod naming;
pub mod nfo;
pub mod notify;
pub mod parser;
pub mod preview;
pub mod url_kind;
pub mod wbi;

#[derive(Debug, Clone)]
pub struct BilibiliAuthCookie {
    pub domain: String,
    pub http_only: bool,
    pub path: String,
    pub secure: bool,
    pub expires: i64,
    pub name: String,
    pub value: String,
    pub host_only: Option<bool>,
    pub same_site: Option<String>,
}

pub trait BilibiliRuntimeProvider: Send + Sync {
    fn active_account_slug(&self) -> Option<String> {
        None
    }

    fn settings(&self) -> AppSettings {
        AppSettings::default()
    }

    fn session_expired(&self, slug: Option<&str>) {
        notify::session_expired(slug);
    }

    fn persist_account(
        &self,
        _cookies: &[BilibiliAuthCookie],
        _uname: &str,
        _source_label: &str,
    ) -> Result<String, String> {
        Err("errors.bilibili.content_unavailable".to_string())
    }
}

static RUNTIME_PROVIDER: OnceLock<Arc<dyn BilibiliRuntimeProvider>> = OnceLock::new();

pub fn set_runtime_provider(provider: Arc<dyn BilibiliRuntimeProvider>) {
    let _ = RUNTIME_PROVIDER.set(provider);
}

fn runtime_provider() -> Option<&'static Arc<dyn BilibiliRuntimeProvider>> {
    RUNTIME_PROVIDER.get()
}

fn runtime_settings() -> AppSettings {
    runtime_provider()
        .map(|provider| provider.settings())
        .unwrap_or_default()
}

fn runtime_session_expired(slug: Option<&str>) {
    if let Some(provider) = runtime_provider() {
        provider.session_expired(slug);
    } else {
        notify::session_expired(slug);
    }
}

pub(crate) fn runtime_persist_account(
    cookies: &[BilibiliAuthCookie],
    uname: &str,
    source_label: &str,
) -> Result<String, String> {
    if let Some(provider) = runtime_provider() {
        provider.persist_account(cookies, uname, source_label)
    } else {
        persist_account_to_default_cookie_file(cookies, uname)
    }
}

fn persist_account_to_default_cookie_file(
    cookies: &[BilibiliAuthCookie],
    uname: &str,
) -> Result<String, String> {
    let slug = auth::slug_from_uname(uname);
    let path = crate::core::paths::app_data_dir()
        .ok_or_else(|| "errors.bilibili.content_unavailable".to_string())?
        .join("cookies")
        .join("bilibili.com")
        .join(format!("{}.txt", slug));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut content = String::from("# Netscape HTTP Cookie File\n");
    for cookie in cookies {
        let raw_domain = sanitize_cookie_field(&cookie.domain);
        let path_field = sanitize_cookie_field(&cookie.path);
        let name = sanitize_cookie_field(&cookie.name);
        let value = sanitize_cookie_field(&cookie.value);
        let http_only_prefix = if cookie.http_only { "#HttpOnly_" } else { "" };
        let is_host_only = cookie
            .host_only
            .unwrap_or_else(|| !raw_domain.starts_with('.'));
        let (domain, include_subdomains) = if is_host_only {
            (
                raw_domain
                    .strip_prefix('.')
                    .unwrap_or(&raw_domain)
                    .to_string(),
                "FALSE",
            )
        } else if raw_domain.starts_with('.') {
            (raw_domain.clone(), "TRUE")
        } else {
            (format!(".{}", raw_domain), "TRUE")
        };
        let secure = if cookie.secure { "TRUE" } else { "FALSE" };
        content.push_str(&format!(
            "{}{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            http_only_prefix,
            domain,
            include_subdomains,
            path_field,
            secure,
            cookie.expires,
            name,
            value,
        ));
    }
    std::fs::write(path, content).map_err(|e| e.to_string())?;
    Ok(slug)
}

fn sanitize_cookie_field(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '\n' && *c != '\r' && *c != '\t')
        .collect()
}

pub struct BilibiliDownloader;

impl Default for BilibiliDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl BilibiliDownloader {
    pub fn new() -> Self {
        Self
    }
}

pub fn has_active_account() -> bool {
    active_account_slug().is_some()
}

pub fn active_account_slug() -> Option<String> {
    if let Some(slug) = runtime_provider().and_then(|provider| provider.active_account_slug()) {
        return Some(slug);
    }

    if let Some(selected) = crate::core::log_hook::current_cookie_slug() {
        if selected != "_anonymous" && slug_has_session_cookie(&selected) {
            return Some(selected);
        }
    }

    if slug_has_session_cookie("_default") {
        return Some("_default".to_string());
    }

    None
}

fn slug_has_session_cookie(slug: &str) -> bool {
    let path =
        match crate::platforms::cookie_provider::cookie_path_for_account("bilibili.com", slug) {
            Some(p) => p,
            None => return false,
        };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    content.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        line.split('\t').nth(5) == Some("SESSDATA")
    })
}

async fn api_engine_download(
    info: &MediaInfo,
    opts: &DownloadOptions,
    progress: mpsc::Sender<ProgressUpdate>,
) -> anyhow::Result<DownloadResult> {
    let _ = cookie::ensure_fresh().await;

    let url = info
        .available_qualities
        .first()
        .map(|q| q.url.as_str())
        .unwrap_or("");
    if url.is_empty() {
        return Err(anyhow!("No URL available"));
    }

    let slug = active_account_slug();
    let client = build_api_client(slug.as_deref(), opts.user_agent.as_deref())?;

    let mut effective_url = url.to_string();
    if url_kind::is_b23_short(&effective_url) {
        if let Ok(resolved) = url_kind::resolve_b23(&client, &effective_url).await {
            effective_url = resolved;
        }
    }

    let kind = url_kind::detect(&effective_url)
        .map_err(|e| anyhow!("Failed to detect URL kind: {}", e.i18n_key()))?;
    let parsed = parser::parse(&client, &kind)
        .await
        .map_err(|e| anyhow!("Failed to parse content: {}", e.i18n_key()))?;

    let settings = runtime_settings();
    let container = mux::container_from_setting(&settings.download.bilibili_container);
    let danmaku_format = danmaku_format_from_setting(&settings.download.bilibili_danmaku_format);
    let cover_format = cover::CoverFormat::from_str(&settings.download.bilibili_cover_format);
    let template_set = naming::TemplateSet {
        video: settings.download.bilibili_naming_video.clone(),
        multi_part: settings.download.bilibili_naming_multi_part.clone(),
        bangumi: settings.download.bilibili_naming_bangumi.clone(),
        cheese: settings.download.bilibili_naming_cheese.clone(),
        collection: settings.download.bilibili_naming_collection.clone(),
    };
    let first_item_owned = parsed.items.first().cloned().unwrap_or_default();
    let naming_kind = naming::classify(&kind, &first_item_owned);
    let naming_inputs = naming::NamingInputs {
        item: &first_item_owned,
        metadata: &parsed.metadata,
        parsed_title: &parsed.title,
    };
    let rendered_name = naming::render_for_kind(naming_kind, &naming_inputs, &template_set);
    let filename = if rendered_name.is_empty() {
        sanitize(&first_item_owned.title)
    } else {
        rendered_name
    };
    let cdn_alt_hosts = cdn::parse_hosts(&settings.download.bilibili_cdn_hosts);
    let engine_opts = engine::EngineOptions {
        output_dir: PathBuf::from(opts.output_dir.clone()),
        container,
        video_qn_pref: if settings.download.bilibili_preferred_qn != 0 {
            settings.download.bilibili_preferred_qn
        } else {
            preview::QN_AUTO
        },
        video_codec_pref: if settings.download.bilibili_preferred_codec != 0 {
            settings.download.bilibili_preferred_codec
        } else {
            preview::CODEC_AUTO
        },
        audio_qn_pref: if settings.download.bilibili_preferred_audio_qn != 0 {
            settings.download.bilibili_preferred_audio_qn
        } else {
            preview::AUDIO_AUTO
        },
        embed_cover: settings.download.embed_thumbnail,
        keep_streams: false,
        filename,
        cancel: opts.cancel_token.clone(),
        danmaku_enabled: settings.download.bilibili_danmaku_enabled,
        danmaku_format: Some(danmaku_format),
        nfo_enabled: settings.download.bilibili_nfo_enabled,
        cover_sidecar_enabled: settings.download.bilibili_cover_sidecar,
        cover_format,
        cdn_alt_hosts,
        cdn_prefer_alternatives: settings.download.bilibili_cdn_prefer_alternatives,
    };

    let result = engine::run_parsed_content(&client, &parsed, &kind, &engine_opts, progress)
        .await
        .map_err(|e| anyhow!("Engine failed: {}", e.i18n_key()))?;

    Ok(DownloadResult {
        file_path: result.final_path,
        file_size_bytes: result.bytes,
        duration_seconds: parsed
            .items
            .first()
            .and_then(|i| i.duration_seconds)
            .unwrap_or(0.0),
        torrent_id: None,
    })
}

fn build_api_client(
    slug: Option<&str>,
    user_agent: Option<&str>,
) -> anyhow::Result<api::ApiClient> {
    let mut client =
        api::ApiClient::new().map_err(|e| anyhow!("api client init failed: {}", e.i18n_key()))?;
    if let Some(ua) = user_agent.filter(|s| !s.is_empty()) {
        client = client.with_user_agent(ua);
    }
    match slug {
        Some(s) => client = client.with_account(s),
        None => client = client.with_anonymous_cookies(),
    }
    Ok(client)
}

fn sanitize(s: &str) -> String {
    let cleaned = sanitize_filename::sanitize(s);
    if cleaned.is_empty() {
        "video".to_string()
    } else {
        cleaned
    }
}

mod mux {
    pub use super::engine::mux::Container;

    pub fn container_from_setting(value: &str) -> Container {
        match value {
            "mkv" => Container::Mkv,
            _ => Container::Mp4,
        }
    }
}

fn danmaku_format_from_setting(value: &str) -> danmaku::DanmakuFormat {
    match value {
        "ass" => danmaku::DanmakuFormat::Ass,
        "json" => danmaku::DanmakuFormat::Json,
        _ => danmaku::DanmakuFormat::Xml,
    }
}

#[async_trait]
impl PlatformDownloader for BilibiliDownloader {
    fn name(&self) -> &str {
        "bilibili"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host.contains("bilibili.com")
                    || host.contains("bilibili.tv")
                    || host == "b23.tv";
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        legacy::get_media_info(url).await
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        if has_active_account() && info.media_type != MediaType::Playlist {
            tracing::info!("[bilibili] using api-direct engine (account active)");
            match api_engine_download(info, opts, progress.clone()).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("errors.bilibili.not_logged_in") {
                        runtime_session_expired(active_account_slug().as_deref());
                    }
                    tracing::warn!(
                        "[bilibili] api engine failed, falling back to legacy yt-dlp: {}",
                        e
                    );
                    return legacy::download(info, opts, progress).await;
                }
            }
        }
        legacy::download(info, opts, progress).await
    }
}
