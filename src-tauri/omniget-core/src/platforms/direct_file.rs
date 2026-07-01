use crate::models::progress::ProgressUpdate;
use crate::platforms::traits::PlatformDownloader;
use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::direct_downloader;
use crate::core::http_client;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality,
};

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

async fn probe_file_size(url: &str) -> Option<u64> {
    let client = http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    let resp = client.head(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.content_length().filter(|len| *len > 0)
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
        let title = filename_from_url(url);
        let file_size_bytes = probe_file_size(url).await;

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
        let output_path = opts.output_dir.join(&filename);

        let mut builder = http_client::apply_global_proxy(reqwest::Client::builder())
            .connect_timeout(std::time::Duration::from_secs(30));

        if let Some(ua) = opts.user_agent.as_deref() {
            builder = builder.user_agent(ua);
        }

        let jar = crate::core::cookie_parser::load_extension_cookies_for_url(file_url).or_else(|| {
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
            progress,
            Some(headers),
            Some(&opts.cancel_token),
        )
        .await?;

        Ok(DownloadResult {
            file_path: output_path,
            file_size_bytes: bytes,
            duration_seconds: 0.0,
            torrent_id: None,
        })
    }
}
