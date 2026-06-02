use std::path::Path;
use std::time::Duration;

use omniget_core::core::http_fetcher::{HttpFetcher, HttpFetcherConfig, HttpFetcherResult};
use omniget_core::models::progress::ProgressUpdate;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, COOKIE, REFERER, USER_AGENT};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::super::api::{ApiClient, BilibiliError, Result};

pub struct FetchOptions<'a> {
    pub url: &'a str,
    pub output_path: &'a Path,
    pub referer: &'a str,
    pub user_agent: &'a str,
    pub cookie_header: Option<&'a str>,
    pub cancel: Option<CancellationToken>,
}

fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60 * 30))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(BilibiliError::Network)
}

fn build_headers(referer: &str, user_agent: &str, cookie_header: Option<&str>) -> HeaderMap {
    let mut h = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(referer) {
        h.insert(REFERER, v);
    }
    if let Ok(v) = HeaderValue::from_str(user_agent) {
        h.insert(USER_AGENT, v);
    }
    if let Some(c) = cookie_header {
        if let Ok(v) = HeaderValue::from_str(c) {
            h.insert(COOKIE, v);
        }
    }
    if let (Ok(k), Ok(v)) = (
        HeaderName::from_bytes(b"Origin"),
        HeaderValue::from_str("https://www.bilibili.com"),
    ) {
        h.insert(k, v);
    }
    h
}

pub async fn fetch_stream(
    opts: FetchOptions<'_>,
    progress: mpsc::Sender<ProgressUpdate>,
) -> Result<HttpFetcherResult> {
    let client = build_client()?;
    let headers = build_headers(opts.referer, opts.user_agent, opts.cookie_header);
    let mut fetcher =
        HttpFetcher::new(client, opts.url.to_string(), opts.output_path.to_path_buf())
            .with_headers(headers)
            .with_config(HttpFetcherConfig {
                concurrent_segments: 8,
                segment_size_hint: 4 * 1024 * 1024,
                min_size_for_chunked: 8 * 1024 * 1024,
                connect_timeout: Duration::from_secs(10),
                read_timeout: Duration::from_secs(45),
                max_retries_per_segment: 3,
                steal_threshold: Duration::from_secs(3),
                steal_min_chunk_size: 512 * 1024,
                use_sidecar_resume: true,
                resume_save_interval: Duration::from_secs(2),
            });
    if let Some(c) = opts.cancel {
        fetcher = fetcher.with_cancel(c);
    }
    fetcher
        .download(progress)
        .await
        .map_err(|_| BilibiliError::ContentUnavailable)
}

pub fn from_api_client(client: &ApiClient) -> (&str, Option<&str>) {
    (client.user_agent(), client.cookie_header())
}
