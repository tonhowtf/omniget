use crate::models::progress::ProgressUpdate;
use crate::platforms::traits::PlatformDownloader;
use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::direct_downloader;
use crate::core::http_client;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality};

pub struct DirectFileDownloader;

impl Default for DirectFileDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectFileDownloader {
    pub fn new() -> Self {
        Self
    }
}

fn free_output_path(wanted: std::path::PathBuf) -> std::path::PathBuf {
    let mut candidate = wanted.clone();
    let mut n = 2;
    while candidate.exists() {
        candidate = crate::core::global_dl::jobs::numbered(&wanted, n);
        n += 1;
    }
    candidate
}

fn filename_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            let last = u.path().rsplit('/').next()?.to_string();
            if last.is_empty() || !last.contains('.') {
                return None;
            }
            Some(
                urlencoding::decode(&last)
                    .map(|d| d.to_string())
                    .unwrap_or(last),
            )
        })
        .map(|name| sanitize_filename::sanitize(&name))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "download".to_string())
}

async fn probe_client() -> Option<reqwest::Client> {
    http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()
}

async fn probe_file(url: &str) -> Option<crate::core::http_fetcher::RemoteProbe> {
    let client = probe_client().await?;
    crate::core::http_fetcher::probe_remote(&client, url, None, std::time::Duration::from_secs(8))
        .await
        .ok()
}

const PAGE_EXTENSIONS: &[&str] = &[
    "html", "htm", "php", "asp", "aspx", "jsp", "jspx", "xhtml", "shtml", "cgi", "do", "action",
    "xml", "json", "js", "css", "rss", "atom",
];

const DIRECT_FILE_MIN_BYTES: u64 = 1024 * 1024;

pub fn has_unknown_extension(url_str: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url_str) else {
        return false;
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return false;
    }
    let path = parsed.path().to_lowercase();
    let Some(last) = path.rsplit('/').next() else {
        return false;
    };
    let Some((name, ext)) = last.rsplit_once('.') else {
        return false;
    };
    if name.is_empty() || ext.is_empty() || ext.len() > 6 {
        return false;
    }
    if !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    if PAGE_EXTENSIONS.contains(&ext) {
        return false;
    }
    if crate::platforms::is_direct_file_url(url_str) {
        return false;
    }
    crate::platforms::generic_ytdlp::is_direct_media_url(url_str).is_none()
}

fn is_page_content_type(ct: &str) -> bool {
    let ct = ct.to_ascii_lowercase();
    ct.starts_with("text/html")
        || ct.starts_with("application/xhtml")
        || ct.starts_with("application/json")
        || ct.starts_with("text/xml")
        || ct.starts_with("application/xml")
}

pub fn probe_says_direct_file(
    content_type: Option<&str>,
    content_length: Option<u64>,
    filename: Option<&str>,
) -> bool {
    if content_type.map(is_page_content_type).unwrap_or(false) {
        return false;
    }
    if filename.is_some() {
        return true;
    }
    content_length.unwrap_or(0) >= DIRECT_FILE_MIN_BYTES
}

fn direct_probe_cache(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, bool)>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, bool)>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub async fn looks_like_direct_file(url: &str) -> bool {
    if !has_unknown_extension(url) {
        return false;
    }
    if let Ok(cache) = direct_probe_cache().lock() {
        if let Some((at, hit)) = cache.get(url) {
            if at.elapsed() < std::time::Duration::from_secs(600) {
                return *hit;
            }
        }
    }
    let hit = match tokio::time::timeout(std::time::Duration::from_secs(5), probe_file(url)).await {
        Ok(Some(p)) => probe_says_direct_file(
            p.content_type.as_deref(),
            p.content_length,
            p.filename.as_deref(),
        ),
        _ => false,
    };
    tracing::debug!("[direct_file] probe {} → direct_file={}", url, hit);
    if let Ok(mut cache) = direct_probe_cache().lock() {
        cache.retain(|_, (at, _)| at.elapsed() < std::time::Duration::from_secs(600));
        cache.insert(url.to_string(), (std::time::Instant::now(), hit));
    }
    hit
}

#[async_trait]
impl PlatformDownloader for DirectFileDownloader {
    fn name(&self) -> &str {
        "direct_file"
    }

    fn can_handle(&self, url: &str) -> bool {
        crate::platforms::is_direct_file_url(url)
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let probe = probe_file(url).await;
        let title = probe
            .as_ref()
            .and_then(|p| p.filename.clone())
            .map(|n| sanitize_filename::sanitize(&n))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| filename_from_url(url));
        let file_size_bytes = probe.as_ref().and_then(|p| p.content_length);

        Ok(MediaInfo {
            title,
            author: String::new(),
            platform: "direct_file".to_string(),
            duration_seconds: None,
            thumbnail_url: None,
            available_qualities: vec![VideoQuality {
                label: "original".to_string(),
                width: 0,
                height: 0,
                url: url.to_string(),
                format: "direct_file".to_string(),
            }],
            media_type: MediaType::File,
            file_size_bytes,
        })
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        let _ = progress.send(ProgressUpdate::percent(0.0)).await;

        let file_url = info
            .available_qualities
            .first()
            .map(|q| q.url.as_str())
            .filter(|u| !u.is_empty())
            .ok_or_else(|| anyhow!("No URL available"))?;

        let filename = sanitize_filename::sanitize(&info.title);
        let filename = if filename.is_empty() {
            filename_from_url(file_url)
        } else {
            filename
        };
        let output_path = free_output_path(opts.output_dir.join(&filename));

        let mut builder = http_client::apply_global_proxy(reqwest::Client::builder())
            .connect_timeout(std::time::Duration::from_secs(30));

        if let Some(ua) = opts.user_agent.as_deref() {
            builder = builder.user_agent(ua);
        }

        let jar =
            crate::core::cookie_parser::load_extension_cookies_for_url(file_url).or_else(|| {
                opts.referer
                    .as_deref()
                    .and_then(crate::core::cookie_parser::load_extension_cookies_for_url)
            });
        if let Some(jar) = jar {
            builder = builder.cookie_provider(jar);
        }

        let client = builder
            .build()
            .map_err(|e| anyhow!("Failed to build HTTP client: {}", e))?;

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(ref r) = opts.referer {
            if let Ok(val) = reqwest::header::HeaderValue::from_str(r) {
                headers.insert(reqwest::header::REFERER, val);
            }
        }
        if let Some(ref hdrs) = opts.extra_headers {
            for (name, value) in hdrs {
                if let (Ok(hname), Ok(hval)) = (
                    reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                    reqwest::header::HeaderValue::from_str(value),
                ) {
                    headers.insert(hname, hval);
                }
            }
        }
        http_client::inject_ua_header(&mut headers, opts.user_agent.as_deref());

        let bytes = direct_downloader::download_direct_with_headers(
            &client,
            file_url,
            &output_path,
            progress.clone(),
            Some(headers),
            Some(&opts.cancel_token),
        )
        .await?;
        let (output_path, bytes) =
            crate::core::ytdlp::finish_direct_video(output_path, bytes, Some(&progress)).await;

        Ok(DownloadResult {
            file_path: output_path,
            file_size_bytes: bytes,
            duration_seconds: 0.0,
            torrent_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_file_with_the_same_name_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("report.pdf");
        assert_eq!(free_output_path(wanted.clone()), wanted);

        std::fs::write(&wanted, b"first").unwrap();
        let second = dir.path().join("report (2).pdf");
        assert_eq!(free_output_path(wanted.clone()), second);

        std::fs::write(&second, b"second").unwrap();
        assert_eq!(free_output_path(wanted), dir.path().join("report (3).pdf"));
    }

    #[test]
    fn unknown_extension_is_the_only_case_worth_probing() {
        assert!(has_unknown_extension(
            "https://nbg1-speed.hetzner.com/100MB.xyz"
        ));
        assert!(has_unknown_extension(
            "https://example.com/files/blob.abc123"
        ));
        assert!(!has_unknown_extension("https://example.com/index.html"));
        assert!(!has_unknown_extension("https://example.com/video.mp4"));
        assert!(!has_unknown_extension("https://example.com/archive.zip"));
        assert!(!has_unknown_extension("https://example.com/watch?v=abc"));
        assert!(!has_unknown_extension("https://example.com/"));
        assert!(!has_unknown_extension("ftp://example.com/a.xyz"));
    }

    #[test]
    fn probe_decides_by_content_type_and_size() {
        assert!(probe_says_direct_file(
            Some("application/octet-stream"),
            Some(104_857_600),
            None
        ));
        assert!(probe_says_direct_file(None, Some(5 * 1024 * 1024), None));
        assert!(probe_says_direct_file(
            Some("application/octet-stream"),
            Some(10),
            Some("x.bin")
        ));
        assert!(!probe_says_direct_file(
            Some("text/html; charset=utf-8"),
            Some(50_000_000),
            None
        ));
        assert!(!probe_says_direct_file(
            Some("application/octet-stream"),
            Some(1000),
            None
        ));
        assert!(!probe_says_direct_file(None, None, None));
    }
}
