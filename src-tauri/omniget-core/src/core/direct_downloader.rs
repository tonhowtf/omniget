use std::path::{Path, PathBuf};

use std::time::Duration;

use anyhow::anyhow;
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::core::http_fetcher::sidecar_path_for;
use crate::core::http_fetcher::{
    get_global_max_concurrent_segments, probe_remote, HttpFetcher, HttpFetcherConfig, ServerBusy,
};
use crate::core::media_signature::{looks_like_html, sniff_media_format};
use crate::models::progress::ProgressUpdate;

const CHUNK_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_RETRIES: u32 = 3;
const CHUNK_SIZE: u64 = 10 * 1024 * 1024;
const CHUNK_THRESHOLD: u64 = 10 * 1024 * 1024;
const MAX_PARALLEL: usize = 12;
const WORKER_CHUNK_THRESHOLD: u64 = 20 * 1024 * 1024;
const WORKER_MAX_SEGMENTS: usize = 4;

#[derive(Debug, Clone, Copy)]
struct StreamPolicy {
    idle: Duration,
    resumes: u32,
    backoff: Duration,
}

const STREAM_POLICY: StreamPolicy = StreamPolicy {
    idle: CHUNK_TIMEOUT,
    resumes: 3,
    backoff: Duration::from_secs(1),
};

fn idle_timeout_error(idle: Duration) -> anyhow::Error {
    anyhow!(
        "Download timeout — no data received for {} seconds",
        idle.as_secs()
    )
}

fn build_worker_client() -> anyhow::Result<reqwest::Client> {
    Ok(crate::core::http_client::apply_global_proxy(
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(15)),
    )
    .build()?)
}
const SNIFF_BYTES: usize = 512;

#[derive(Debug)]
pub struct RateLimitError {
    pub retry_after_seconds: Option<u64>,
}
impl std::fmt::Display for RateLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP 429 rate limited")
    }
}
impl std::error::Error for RateLimitError {}
fn parse_retry_after(raw: &str, now: chrono::DateTime<chrono::Utc>) -> Option<u64> {
    let raw = raw.trim();
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(seconds);
    }
    let date = chrono::DateTime::parse_from_rfc2822(raw).ok()?;
    Some(date.signed_duration_since(now).num_seconds().max(0) as u64)
}

struct ProbeResult {
    content_length: Option<u64>,
    accept_ranges: bool,
}

pub async fn download_direct(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: mpsc::Sender<ProgressUpdate>,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<u64> {
    download_direct_with_headers(client, url, output, progress_tx, None, cancel).await
}

pub async fn download_direct_with_headers(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: mpsc::Sender<ProgressUpdate>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<u64> {
    if crate::core::dependencies::worker_mode() {
        let worker_client = build_worker_client()?;
        return download_attempt(
            &worker_client,
            url,
            output,
            &progress_tx,
            headers,
            cancel,
            true,
            &STREAM_POLICY,
        )
        .await;
    }
    let mut last_err = None;
    let mut attempt: u32 = 0;
    let mut forbidden_retries: u32 = 0;
    let mut requests_made: u32 = 0;
    let mut effective_headers = headers;

    while attempt < MAX_RETRIES {
        if let Some(token) = cancel {
            if token.is_cancelled() {
                return Err(anyhow!("Download cancelled"));
            }
        }

        if requests_made > 0 {
            let base = 1000 * (requests_made as u64);
            let jitter = rand::random::<u64>() % (base / 2 + 1);
            tokio::time::sleep(Duration::from_millis(base + jitter)).await;
        }

        requests_made += 1;
        match download_attempt(
            client,
            url,
            output,
            &progress_tx,
            effective_headers.clone(),
            cancel,
            false,
            &STREAM_POLICY,
        )
        .await
        {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                if is_forbidden_error(&e) {
                    if let Some(step) = forbidden_retry_step(forbidden_retries) {
                        tracing::warn!("[direct] HTTP 403; retrying with {}", step.describe());
                        effective_headers = Some(headers_for_forbidden_retry(
                            effective_headers.as_ref(),
                            step,
                        ));
                        forbidden_retries += 1;
                        last_err = Some(e);
                        continue;
                    }
                }
                if is_fatal_error(&e) {
                    let _ = discard_part(&part_path_for(output));
                    return Err(e);
                }
                tracing::warn!(
                    "[direct] attempt {}/{} failed: {}",
                    attempt + 1,
                    MAX_RETRIES,
                    e
                );
                last_err = Some(e);
                attempt += 1;
            }
        }
    }

    let _ = discard_part(&part_path_for(output));
    Err(last_err.unwrap_or_else(|| anyhow!("Download failed after {} attempts", MAX_RETRIES)))
}

fn discard_part(part_path: &Path) -> std::io::Result<()> {
    let sidecar = sidecar_path_for(part_path);
    let mut tmp = sidecar.as_os_str().to_owned();
    tmp.push(".tmp");
    let _ = std::fs::remove_file(PathBuf::from(tmp));
    let _ = std::fs::remove_file(&sidecar);
    std::fs::remove_file(part_path)
}

fn part_path_for(output: &Path) -> PathBuf {
    let mut part = output.as_os_str().to_owned();
    part.push(".part");
    PathBuf::from(part)
}

fn is_fatal_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    for code in &[
        "HTTP 400", "HTTP 401", "HTTP 403", "HTTP 404", "HTTP 405", "HTTP 410", "HTTP 451",
    ] {
        if msg.contains(code) {
            return true;
        }
    }
    if msg.contains("HTML instead of media") {
        return true;
    }
    if msg.contains("cancelled") {
        return true;
    }
    false
}

fn is_forbidden_error(err: &anyhow::Error) -> bool {
    err.to_string().contains("HTTP 403")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForbiddenRetryStep {
    RangeOnly,
    RangeAndFetchHints,
}

impl ForbiddenRetryStep {
    fn describe(self) -> &'static str {
        match self {
            ForbiddenRetryStep::RangeOnly => "Range: bytes=0-",
            ForbiddenRetryStep::RangeAndFetchHints => "Range + sec-fetch hints",
        }
    }
}

fn forbidden_retry_step(retries_done: u32) -> Option<ForbiddenRetryStep> {
    match retries_done {
        0 => Some(ForbiddenRetryStep::RangeOnly),
        1 => Some(ForbiddenRetryStep::RangeAndFetchHints),
        _ => None,
    }
}

fn headers_for_forbidden_retry(
    base: Option<&reqwest::header::HeaderMap>,
    step: ForbiddenRetryStep,
) -> reqwest::header::HeaderMap {
    use reqwest::header::HeaderValue;

    let mut headers = base.cloned().unwrap_or_default();
    headers.insert(reqwest::header::RANGE, HeaderValue::from_static("bytes=0-"));
    if step == ForbiddenRetryStep::RangeAndFetchHints {
        headers.insert("sec-fetch-mode", HeaderValue::from_static("no-cors"));
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-site"));
    }
    headers
}

async fn probe_url(
    client: &reqwest::Client,
    url: &str,
    headers: Option<&reqwest::header::HeaderMap>,
    worker: bool,
) -> anyhow::Result<ProbeResult> {
    match probe_remote(client, url, headers, Duration::from_secs(12)).await {
        Ok(p) => Ok(ProbeResult {
            content_length: p.content_length,
            accept_ranges: p.accept_ranges,
        }),
        Err(e) if e.to_string().starts_with("unreachable") => Err(e),
        Err(e) if e.downcast_ref::<ServerBusy>().is_some() => {
            let busy = e.downcast::<ServerBusy>().expect("checked above");
            if busy.status == 429 {
                if let Some(h) = host_of(url) {
                    concurrency_tuner().lock().await.record_rate_limit(&h);
                }
            }
            if worker && busy.status == 429 {
                return Err(RateLimitError {
                    retry_after_seconds: busy
                        .retry_after
                        .as_deref()
                        .and_then(|v| parse_retry_after(v, chrono::Utc::now())),
                }
                .into());
            }
            let status = reqwest::StatusCode::from_u16(busy.status)
                .unwrap_or(reqwest::StatusCode::SERVICE_UNAVAILABLE);
            Err(anyhow!("HTTP {} downloading {}", status, url))
        }
        Err(e) => {
            tracing::debug!("[direct] probe failed, streaming without Range: {}", e);
            Ok(ProbeResult {
                content_length: None,
                accept_ranges: false,
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn download_attempt(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: &mpsc::Sender<ProgressUpdate>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
    worker: bool,
    policy: &StreamPolicy,
) -> anyhow::Result<u64> {
    let part_path = part_path_for(output);
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let probe = probe_url(client, url, headers.as_ref(), worker).await?;

    let threshold = if worker {
        WORKER_CHUNK_THRESHOLD
    } else {
        CHUNK_THRESHOLD
    };
    let use_chunked = probe.accept_ranges && probe.content_length.is_some_and(|s| s > threshold);

    if use_chunked {
        let max_segments = worker.then_some(WORKER_MAX_SEGMENTS);
        match run_http_fetcher(
            client,
            url,
            output,
            progress_tx,
            headers.clone(),
            cancel,
            max_segments,
            policy.backoff,
        )
        .await
        {
            Ok(size) => return Ok(size),
            Err(fetch_err) => {
                if is_fatal_error(&fetch_err) {
                    return Err(fetch_err);
                }
                if worker && fetch_err.to_string().contains("HTTP 429") {
                    let _ = discard_part(&part_path);
                    return Err(RateLimitError {
                        retry_after_seconds: None,
                    }
                    .into());
                }
                if fetch_err.to_string().contains("HTTP 429") {
                    let _ = discard_part(&part_path);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    return Err(fetch_err);
                }
                let _ = discard_part(&part_path);
                tracing::warn!(
                    "[direct] http_fetcher failed, falling back to single stream: {}",
                    fetch_err
                );
                download_single_stream(
                    client,
                    url,
                    &part_path,
                    0,
                    probe.content_length,
                    progress_tx,
                    headers,
                    cancel,
                    worker,
                    policy,
                )
                .await?;
            }
        }
    } else {
        let existing = match std::fs::metadata(&part_path) {
            Ok(m) if m.len() > 0 && probe.accept_ranges => m.len(),
            _ => 0,
        };
        download_single_stream(
            client,
            url,
            &part_path,
            existing,
            probe.content_length,
            progress_tx,
            headers,
            cancel,
            worker,
            policy,
        )
        .await?;
    }

    if let Some(expected) = probe.content_length {
        let actual = std::fs::metadata(&part_path)?.len();
        if expected > 0 && actual != expected {
            let _ = discard_part(&part_path);
            return Err(anyhow!(
                "Size mismatch: expected {} bytes, got {}",
                expected,
                actual
            ));
        }
    }

    if let Err(e) = reject_html_masquerading_as_media(&part_path) {
        let _ = discard_part(&part_path);
        return Err(e);
    }

    std::fs::rename(&part_path, output)?;
    let _ = progress_tx.send(ProgressUpdate::percent(100.0)).await;

    let size = std::fs::metadata(output)?.len();
    Ok(size)
}

#[allow(clippy::too_many_arguments)]
async fn run_http_fetcher(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: &mpsc::Sender<ProgressUpdate>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
    max_segments: Option<usize>,
    retry_backoff: Duration,
) -> anyhow::Result<u64> {
    let teto = get_global_max_concurrent_segments()
        .unwrap_or(MAX_PARALLEL)
        .clamp(1, 32)
        .min(max_segments.unwrap_or(usize::MAX));
    let host = host_of(url);
    let concurrent = match &host {
        Some(h) => concurrency_tuner()
            .lock()
            .await
            .suggest(h, teto as u32, teto as u32) as usize,
        None => teto,
    };
    let inicio = std::time::Instant::now();
    let _active = super::adaptive_concurrency::ActiveDownloadGuard::new();
    let cfg = HttpFetcherConfig {
        min_size_for_chunked: 0,
        concurrent_segments: concurrent,
        segment_size_hint: CHUNK_SIZE,
        use_sidecar_resume: max_segments.is_none(),
        retry_backoff,
        ..Default::default()
    };

    let mut fetcher =
        HttpFetcher::new(client.clone(), url.to_string(), output.to_path_buf()).with_config(cfg);
    if let Some(h) = headers {
        fetcher = fetcher.with_headers(h);
    }
    if let Some(c) = cancel {
        fetcher = fetcher.with_cancel(c.clone());
    }
    let result = match fetcher.download(progress_tx.clone()).await {
        Ok(r) => r,
        Err(e) => {
            if e.to_string().contains("HTTP 429") {
                if let Some(h) = &host {
                    concurrency_tuner().lock().await.record_rate_limit(h);
                    tracing::warn!(
                        "[direct] {} rate-limited with {} segments; next attempt uses fewer",
                        h,
                        concurrent
                    );
                }
            }
            return Err(e);
        }
    };

    if let Some(h) = host {
        let secs = inicio.elapsed().as_secs_f64();
        if secs >= 1.0 && result.bytes_written > 0 {
            let bps = result.bytes_written as f64 / secs;
            concurrency_tuner()
                .lock()
                .await
                .record(&h, concurrent as u32, bps);
        }
    }

    Ok(result.bytes_written)
}

fn concurrency_tuner() -> &'static tokio::sync::Mutex<super::adaptive_concurrency::ConcurrencyTuner>
{
    super::adaptive_concurrency::global()
}

fn host_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    Some(match u.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

fn reject_html_masquerading_as_media(part_path: &Path) -> anyhow::Result<()> {
    let head = read_head(part_path, SNIFF_BYTES)?;
    if sniff_media_format(&head).is_none() && looks_like_html(&head) {
        return Err(anyhow!(
            "Server returned HTML instead of media — the link may have expired or needs a login"
        ));
    }
    Ok(())
}

fn read_head(path: &Path, max: usize) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; max];
    let mut filled = 0usize;
    while filled < max {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    buf.truncate(filled);
    Ok(buf)
}

#[allow(clippy::too_many_arguments)]
async fn download_single_stream(
    client: &reqwest::Client,
    url: &str,
    part_path: &Path,
    existing_bytes: u64,
    total_size: Option<u64>,
    progress_tx: &mpsc::Sender<ProgressUpdate>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
    worker: bool,
    policy: &StreamPolicy,
) -> anyhow::Result<()> {
    let mut state = StreamState {
        written: existing_bytes,
        total: total_size,
        resumable: false,
        transient: false,
    };
    let mut resumes = 0u32;
    loop {
        state.transient = false;
        let from = state.written;
        let result = stream_once(
            client,
            url,
            part_path,
            from,
            progress_tx,
            headers.clone(),
            cancel,
            worker,
            policy,
            &mut state,
        )
        .await;
        let err = match result {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };
        let cancelled = cancel.is_some_and(|c| c.is_cancelled());
        if cancelled
            || !state.transient
            || !state.resumable
            || state.written == 0
            || resumes >= policy.resumes
        {
            return Err(err);
        }
        let wait = policy
            .backoff
            .saturating_mul(1 << resumes)
            .min(Duration::from_secs(8));
        resumes += 1;
        tracing::warn!(
            "[direct] stream broke at byte {} ({}); resuming with Range ({}/{})",
            state.written,
            err,
            resumes,
            policy.resumes
        );
        tokio::time::sleep(wait).await;
    }
}

struct StreamState {
    written: u64,
    total: Option<u64>,
    resumable: bool,
    transient: bool,
}

#[allow(clippy::too_many_arguments)]
async fn stream_once(
    client: &reqwest::Client,
    url: &str,
    part_path: &Path,
    existing_bytes: u64,
    progress_tx: &mpsc::Sender<ProgressUpdate>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
    worker: bool,
    policy: &StreamPolicy,
    state: &mut StreamState,
) -> anyhow::Result<()> {
    let mut h = headers.unwrap_or_default();
    if existing_bytes > 0 {
        h.remove(reqwest::header::RANGE);
    }
    h.insert(
        reqwest::header::ACCEPT_ENCODING,
        reqwest::header::HeaderValue::from_static("identity"),
    );
    let mut request = client.get(url).headers(h);
    let total_size = state.total;

    if existing_bytes > 0 {
        if let Some(total) = total_size {
            if existing_bytes >= total {
                return Ok(());
            }
        }
        request = request.header("Range", format!("bytes={}-", existing_bytes));
    }

    let response = if worker {
        let mut request = request.build()?;
        let mut hops = 0;
        loop {
            if cancel.is_some_and(|c| c.is_cancelled()) {
                return Err(anyhow!("Download cancelled"));
            }
            let previous = request.url().clone();
            let mut next_request = request
                .try_clone()
                .ok_or_else(|| anyhow!("WORKER_REQUEST_INVALID"))?;
            let response = tokio::time::timeout(policy.idle, client.execute(request))
                .await
                .map_err(|_| idle_timeout_error(policy.idle))??;
            if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                break response;
            }
            if hops >= 5 {
                return Err(anyhow!("WORKER_REDIRECT_LIMIT"));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| anyhow!("WORKER_REDIRECT_INVALID"))?;
            let next = previous
                .join(location)
                .map_err(|_| anyhow!("WORKER_REDIRECT_INVALID"))?;
            if !matches!(next.scheme(), "http" | "https")
                || !next.username().is_empty()
                || next.password().is_some()
                || (previous.scheme() == "https" && next.scheme() != "https")
            {
                return Err(anyhow!("WORKER_REDIRECT_DENIED"));
            }
            if previous.host_str() != next.host_str()
                || previous.port_or_known_default() != next.port_or_known_default()
            {
                for header in [
                    reqwest::header::AUTHORIZATION,
                    reqwest::header::COOKIE,
                    reqwest::header::REFERER,
                ] {
                    next_request.headers_mut().remove(header);
                }
            }
            next_request
                .headers_mut()
                .remove(reqwest::header::PROXY_AUTHORIZATION);
            *next_request.url_mut() = next;
            request = next_request;
            hops += 1;
        }
    } else {
        tokio::time::timeout(policy.idle, request.send())
            .await
            .map_err(|_| idle_timeout_error(policy.idle))??
    };
    if worker && response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(RateLimitError {
            retry_after_seconds: response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| parse_retry_after(v, chrono::Utc::now())),
        }
        .into());
    }

    let mut offset = 0u64;
    if existing_bytes > 0 {
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            offset = existing_bytes;
        } else if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            let _ = discard_part(part_path);
            state.written = 0;
            return Err(anyhow!("Range not satisfiable, restarting"));
        } else if !response.status().is_success() {
            return Err(anyhow!("HTTP {} downloading {}", response.status(), url));
        }
    } else if !response.status().is_success() {
        return Err(anyhow!("HTTP {} downloading {}", response.status(), url));
    }

    if let Some(ct) = response.headers().get("content-type") {
        if let Ok(ct_str) = ct.to_str() {
            if ct_str.contains("text/html") {
                return Err(anyhow!(
                    "Server returned HTML instead of media — URL may have expired"
                ));
            }
        }
    }

    state.resumable = response.status() == reqwest::StatusCode::PARTIAL_CONTENT
        || response
            .headers()
            .get(reqwest::header::ACCEPT_RANGES)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("bytes"));

    let total_size = total_size
        .filter(|t| *t > 0)
        .or_else(|| response_total_bytes(response.headers(), response.content_length(), offset));
    state.total = total_size;

    use std::io::Write;
    let raw_file = if offset > 0 {
        std::fs::OpenOptions::new().append(true).open(part_path)?
    } else {
        std::fs::File::create(part_path)?
    };
    state.written = offset;

    let mut file = std::io::BufWriter::with_capacity(256 * 1024, raw_file);
    let mut downloaded = offset;
    let mut stream = response.bytes_stream();

    let mut last_emit = std::time::Instant::now();
    let mut speed_anchor_bytes = downloaded;
    let mut speed_anchor_time = std::time::Instant::now();
    let mut speed_ema: f64 = 0.0;

    loop {
        if let Some(token) = cancel {
            if token.is_cancelled() {
                file.flush()?;
                state.written = downloaded;
                return Err(anyhow!("Download cancelled"));
            }
        }

        let chunk_result = tokio::time::timeout(policy.idle, stream.next()).await;
        match chunk_result {
            Ok(Some(Ok(chunk))) => {
                file.write_all(&chunk)
                    .map_err(|e| anyhow!("Write error (disk full?): {}", e))?;
                downloaded += chunk.len() as u64;

                if last_emit.elapsed() >= std::time::Duration::from_millis(250) {
                    let dt = speed_anchor_time.elapsed().as_secs_f64();
                    if dt >= 0.2 {
                        let instant = (downloaded.saturating_sub(speed_anchor_bytes)) as f64 / dt;
                        speed_ema = if speed_ema > 0.0 {
                            speed_ema * 0.6 + instant * 0.4
                        } else {
                            instant
                        };
                        speed_anchor_bytes = downloaded;
                        speed_anchor_time = std::time::Instant::now();
                    }
                    let speed = (speed_ema > 0.0).then_some(speed_ema);
                    let _ = progress_tx
                        .send(single_stream_progress(downloaded, total_size, speed))
                        .await;
                    last_emit = std::time::Instant::now();
                }
            }
            Ok(Some(Err(e))) => {
                file.flush()?;
                state.written = downloaded;
                state.transient = true;
                return Err(anyhow!("Download stream error: {}", e));
            }
            Ok(None) => {
                file.flush()?;
                state.written = downloaded;
                if total_size.is_some_and(|t| downloaded < t) {
                    state.transient = true;
                    return Err(anyhow!(
                        "Download stream ended early: {} of {} bytes",
                        downloaded,
                        total_size.unwrap_or(0)
                    ));
                }
                break;
            }
            Err(_) => {
                file.flush()?;
                state.written = downloaded;
                state.transient = true;
                return Err(idle_timeout_error(policy.idle));
            }
        }
    }

    Ok(())
}

fn response_total_bytes(
    headers: &reqwest::header::HeaderMap,
    content_length: Option<u64>,
    offset: u64,
) -> Option<u64> {
    if let Some(total) = headers
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit_once('/'))
        .and_then(|(_, total)| total.trim().parse::<u64>().ok())
    {
        return (total > 0).then_some(total);
    }
    content_length
        .filter(|len| *len > 0)
        .map(|len| len.saturating_add(offset))
}

fn progress_percent(downloaded: u64, total: Option<u64>) -> Option<f64> {
    total
        .filter(|t| *t > 0)
        .map(|t| ((downloaded as f64 / t as f64) * 100.0).min(100.0))
}

fn single_stream_progress(
    downloaded: u64,
    total: Option<u64>,
    speed: Option<f64>,
) -> ProgressUpdate {
    let total = total.filter(|t| *t > 0);
    match progress_percent(downloaded, total) {
        Some(pct) => {
            let eta = total.and_then(|total| {
                speed.and_then(|s| {
                    (s > 0.0 && total > downloaded)
                        .then(|| ((total - downloaded) as f64 / s) as u64)
                })
            });
            ProgressUpdate::rich(pct, Some(downloaded), total, speed, eta)
        }
        None => ProgressUpdate::rich(0.0, Some(downloaded), None, speed, None).indeterminate(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_none_when_total_unknown() {
        assert_eq!(progress_percent(6_000_000, None), None);
        assert_eq!(progress_percent(6_000_000, Some(0)), None);
        let u = single_stream_progress(6_000_000, None, Some(1_000_000.0));
        assert!(u.indeterminate);
        assert_eq!(u.percent_value(), None);
        assert_eq!(u.total_bytes, None);
        assert_eq!(u.eta_seconds, None);
        assert_eq!(u.downloaded_bytes, Some(6_000_000));
    }

    #[test]
    fn percent_is_real_when_total_known() {
        let u = single_stream_progress(6_151_268, Some(102_521_139), Some(1_000_000.0));
        let p = u.percent_value().expect("known total");
        assert!((p - 6.0).abs() < 0.01, "{p}");
        assert_eq!(u.total_bytes, Some(102_521_139));
        assert!(u.eta_seconds.is_some());
    }

    #[test]
    fn get_response_supplies_total_when_probe_missed_it() {
        use reqwest::header::{HeaderMap, HeaderValue, CONTENT_RANGE};
        let empty = HeaderMap::new();
        assert_eq!(
            response_total_bytes(&empty, Some(102_521_139), 0),
            Some(102_521_139)
        );
        assert_eq!(response_total_bytes(&empty, Some(100), 900), Some(1000));
        assert_eq!(response_total_bytes(&empty, None, 0), None);
        let mut ranged = HeaderMap::new();
        ranged.insert(
            CONTENT_RANGE,
            HeaderValue::from_static("bytes 900-999/1000"),
        );
        assert_eq!(response_total_bytes(&ranged, Some(100), 900), Some(1000));
        let mut star = HeaderMap::new();
        star.insert(CONTENT_RANGE, HeaderValue::from_static("bytes 900-999/*"));
        assert_eq!(response_total_bytes(&star, Some(100), 900), Some(1000));
    }

    #[test]
    fn part_path_appends_suffix() {
        let output = Path::new("video.mp4");
        let part = part_path_for(output);
        assert_eq!(part, PathBuf::from("video.mp4.part"));
    }

    #[test]
    fn part_path_no_extension() {
        let output = Path::new("video");
        let part = part_path_for(output);
        assert_eq!(part, PathBuf::from("video.part"));
    }

    #[test]
    fn part_path_nested() {
        let output = Path::new("downloads/curso/aula.mp4");
        let part = part_path_for(output);
        assert_eq!(part, PathBuf::from("downloads/curso/aula.mp4.part"));
    }

    #[test]
    fn is_fatal_http_400() {
        assert!(is_fatal_error(&anyhow!("HTTP 400 downloading url")));
    }

    #[test]
    fn is_fatal_http_401() {
        assert!(is_fatal_error(&anyhow!("HTTP 401 downloading url")));
    }

    #[test]
    fn is_fatal_http_403() {
        assert!(is_fatal_error(&anyhow!("HTTP 403 downloading url")));
    }

    #[test]
    fn is_fatal_http_404() {
        assert!(is_fatal_error(&anyhow!("HTTP 404 downloading url")));
    }

    #[test]
    fn is_fatal_html_response() {
        assert!(is_fatal_error(&anyhow!(
            "Server returned HTML instead of media — URL may have expired"
        )));
    }

    #[test]
    fn is_fatal_cancelled() {
        assert!(is_fatal_error(&anyhow!("Download cancelled")));
    }

    #[test]
    fn is_not_fatal_500() {
        assert!(!is_fatal_error(&anyhow!("HTTP 500 Internal Server Error")));
    }

    #[test]
    fn is_not_fatal_502() {
        assert!(!is_fatal_error(&anyhow!("HTTP 502 Bad Gateway")));
    }

    #[test]
    fn is_not_fatal_timeout() {
        assert!(!is_fatal_error(&anyhow!("connection timed out")));
    }

    #[test]
    fn is_not_fatal_network() {
        assert!(!is_fatal_error(&anyhow!("network error")));
    }

    #[test]
    fn chunk_count_for_12mb() {
        let total: u64 = 12 * 1024 * 1024;
        assert_eq!(total.div_ceil(CHUNK_SIZE), 2);
    }

    #[test]
    fn chunk_count_exact_boundary() {
        assert_eq!(CHUNK_SIZE.div_ceil(CHUNK_SIZE), 1);
    }

    #[test]
    fn chunk_count_single_byte_over() {
        assert_eq!((CHUNK_SIZE + 1).div_ceil(CHUNK_SIZE), 2);
    }

    #[test]
    fn threshold_gte_chunk_size() {
        assert!(CHUNK_THRESHOLD >= CHUNK_SIZE);
    }

    #[test]
    fn host_e_a_chave_do_historico_de_vazao() {
        assert_eq!(
            host_of("https://CDN.Exemplo.com/a/b.mp4"),
            Some("cdn.exemplo.com".to_string()),
            "host tem que normalizar para caixa baixa, senao vira duas entradas"
        );
        assert_eq!(host_of("nao e url"), None);
        assert_eq!(
            host_of("https://cdn.exemplo.com:443/x"),
            Some("cdn.exemplo.com".to_string())
        );
        assert_eq!(
            host_of("http://127.0.0.1:54321/x"),
            Some("127.0.0.1:54321".to_string())
        );
    }

    #[test]
    fn forbidden_ladder_has_exactly_two_rungs_in_order() {
        assert_eq!(
            forbidden_retry_step(0),
            Some(ForbiddenRetryStep::RangeOnly),
            "first retry only re-sends with Range"
        );
        assert_eq!(
            forbidden_retry_step(1),
            Some(ForbiddenRetryStep::RangeAndFetchHints),
            "second retry adds the sec-fetch hints"
        );
        assert_eq!(forbidden_retry_step(2), None, "the ladder ends here");
        assert_eq!(forbidden_retry_step(99), None);
    }

    #[test]
    fn forbidden_ladder_terminates_and_then_403_is_fatal() {
        let err = anyhow!("HTTP 403 downloading https://cdn.example/a.mp4");
        let mut forbidden_retries = 0u32;
        let mut requests = 1u32;

        loop {
            if !is_forbidden_error(&err) {
                break;
            }
            match forbidden_retry_step(forbidden_retries) {
                Some(_) => {
                    forbidden_retries += 1;
                    requests += 1;
                }
                None => break,
            }
        }

        assert_eq!(forbidden_retries, 2, "exactly two extra tries");
        assert_eq!(requests, 3, "three requests in total, never more");
        assert!(
            is_fatal_error(&err),
            "once the ladder is spent a 403 is fatal again"
        );
    }

    #[test]
    fn only_403_climbs_the_ladder() {
        assert!(is_forbidden_error(&anyhow!("HTTP 403 downloading url")));
        for other in ["HTTP 400", "HTTP 401", "HTTP 404", "HTTP 410", "HTTP 451"] {
            let e = anyhow!("{} downloading url", other);
            assert!(
                !is_forbidden_error(&e),
                "{other} must stay fatal on the first try"
            );
            assert!(is_fatal_error(&e));
        }
    }

    #[test]
    fn forbidden_retry_headers_escalate_and_keep_the_caller_headers() {
        let mut base = reqwest::header::HeaderMap::new();
        base.insert(
            reqwest::header::REFERER,
            reqwest::header::HeaderValue::from_static("https://example.com/"),
        );

        let first = headers_for_forbidden_retry(Some(&base), ForbiddenRetryStep::RangeOnly);
        assert_eq!(first.get(reqwest::header::RANGE).unwrap(), "bytes=0-");
        assert!(first.get("sec-fetch-mode").is_none());
        assert_eq!(
            first.get(reqwest::header::REFERER).unwrap(),
            "https://example.com/",
            "the caller's headers survive the escalation"
        );

        let second =
            headers_for_forbidden_retry(Some(&first), ForbiddenRetryStep::RangeAndFetchHints);
        assert_eq!(second.get(reqwest::header::RANGE).unwrap(), "bytes=0-");
        assert_eq!(second.get("sec-fetch-mode").unwrap(), "no-cors");
        assert_eq!(second.get("sec-fetch-site").unwrap(), "same-site");
        assert_eq!(
            second.get_all(reqwest::header::RANGE).iter().count(),
            1,
            "insert, not append: never two Range headers"
        );
    }

    #[test]
    fn forbidden_retry_headers_work_without_caller_headers() {
        let h = headers_for_forbidden_retry(None, ForbiddenRetryStep::RangeAndFetchHints);
        assert_eq!(h.get(reqwest::header::RANGE).unwrap(), "bytes=0-");
        assert_eq!(h.get("sec-fetch-site").unwrap(), "same-site");
    }

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omniget-direct-test-{}-{}",
            std::process::id(),
            name
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn html_error_page_is_rejected_before_rename() {
        let path = temp_file(
            "erro.mp4.part",
            b"<!DOCTYPE html>\n<html><body>Access denied</body></html>",
        );
        let err = reject_html_masquerading_as_media(&path).unwrap_err();
        assert!(
            err.to_string().contains("HTML instead of media"),
            "the message has to be the fatal one: {err}"
        );
        assert!(is_fatal_error(&err), "no point retrying an error page");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn real_media_passes_the_gate() {
        let mut mp4 = vec![0x00, 0x00, 0x00, 0x20];
        mp4.extend_from_slice(b"ftypisom");
        mp4.resize(2048, 0);
        let path = temp_file("ok.mp4.part", &mp4);
        assert!(reject_html_masquerading_as_media(&path).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unsniffable_containers_are_never_rejected() {
        for (name, body) in [
            (
                "legenda.srt.part",
                b"1\n00:00:01,000 --> 00:00:02,000\nOi\n".as_slice(),
            ),
            (
                "capa.jpg.part",
                &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46][..],
            ),
            (
                "pack.zip.part",
                b"PK\x03\x04....................".as_slice(),
            ),
            ("doc.pdf.part", b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".as_slice()),
            ("vazio.bin.part", b"".as_slice()),
        ] {
            let path = temp_file(name, body);
            assert!(
                reject_html_masquerading_as_media(&path).is_ok(),
                "{name} must not be rejected by the HTML gate"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    fn read_head_stops_at_the_limit() {
        let path = temp_file("grande.bin.part", &vec![7u8; SNIFF_BYTES * 4]);
        assert_eq!(read_head(&path, SNIFF_BYTES).unwrap().len(), SNIFF_BYTES);
        let _ = std::fs::remove_file(&path);

        let short = temp_file("curto.bin.part", &[1, 2, 3]);
        assert_eq!(read_head(&short, SNIFF_BYTES).unwrap(), vec![1, 2, 3]);
        let _ = std::fs::remove_file(&short);
    }

    #[tokio::test]
    async fn sem_historico_o_teto_e_respeitado() {
        let tuner = super::super::adaptive_concurrency::ConcurrencyTuner::default();
        assert_eq!(tuner.suggest("novo.host", 8, 8), 8);
        assert_eq!(tuner.suggest("novo.host", 32, 4), 4);
    }
    use crate::core::http_fetcher::test_server;

    const FAST: StreamPolicy = StreamPolicy {
        idle: Duration::from_millis(400),
        resumes: 3,
        backoff: Duration::from_millis(10),
    };

    fn payload(n: usize) -> Vec<u8> {
        (0u8..=250).cycle().take(n).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omniget-direct-srv-{}-{}-{}",
            std::process::id(),
            name,
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn sink() -> mpsc::Sender<ProgressUpdate> {
        let (tx, mut rx) = mpsc::channel(64);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        tx
    }

    async fn flaky_server(body: Vec<u8>, stall: bool) -> (String, test_server::Log) {
        test_server::spawn(move |_, req| {
            if req.method == "GET" && req.range().is_none() {
                let mut r = test_server::serve_bytes(&body, req);
                r.cut_after = Some(body.len() / 2);
                r.stall = stall;
                return r;
            }
            test_server::serve_bytes(&body, req)
        })
        .await
    }

    #[tokio::test]
    async fn worker_attempt_resumes_with_range_after_connection_drop() {
        let body = payload(300_000);
        let (base, log) = flaky_server(body.clone(), false).await;
        let out = scratch("drop.bin");
        let client = build_worker_client().unwrap();
        let got = download_attempt(
            &client,
            &format!("{base}/drop.bin"),
            &out,
            &sink(),
            None,
            None,
            true,
            &FAST,
        )
        .await;
        assert!(got.is_ok(), "{got:?}");
        assert_eq!(std::fs::read(&out).unwrap(), body);
        let reqs = log.lock().unwrap().clone();
        assert!(
            reqs.iter()
                .any(|r| r.method == "GET" && r.range() == Some((150_000, None))),
            "resume must ask for bytes=150000-: {reqs:?}"
        );
        for r in reqs.iter().filter(|r| r.method == "GET") {
            assert_eq!(r.header("accept-encoding"), Some("identity"), "{r:?}");
        }
    }

    #[tokio::test]
    async fn without_resume_budget_the_drop_fails_the_attempt() {
        let body = payload(300_000);
        let (base, _log) = flaky_server(body, false).await;
        let out = scratch("noresume.bin");
        let client = build_worker_client().unwrap();
        let policy = StreamPolicy { resumes: 0, ..FAST };
        let got = download_attempt(
            &client,
            &format!("{base}/noresume.bin"),
            &out,
            &sink(),
            None,
            None,
            true,
            &policy,
        )
        .await;
        assert!(got.is_err());
    }

    #[tokio::test]
    async fn stalled_stream_is_resumed_after_the_idle_window() {
        let body = payload(200_000);
        let (base, log) = flaky_server(body.clone(), true).await;
        let out = scratch("stall.bin");
        let client = build_worker_client().unwrap();
        let t0 = std::time::Instant::now();
        let got = download_attempt(
            &client,
            &format!("{base}/stall.bin"),
            &out,
            &sink(),
            None,
            None,
            true,
            &FAST,
        )
        .await;
        assert!(got.is_ok(), "{got:?}");
        assert!(t0.elapsed() < Duration::from_secs(10), "{:?}", t0.elapsed());
        assert_eq!(std::fs::read(&out).unwrap(), body);
        assert!(log
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.range() == Some((100_000, None))));
    }

    #[tokio::test]
    async fn worker_splits_files_above_20mb_into_range_segments() {
        let body = payload(21 * 1024 * 1024);
        let srv_body = body.clone();
        let (base, log) =
            test_server::spawn(move |_, req| test_server::serve_bytes(&srv_body, req)).await;
        let url = format!("{base}/big.bin");
        concurrency_tuner()
            .lock()
            .await
            .forget(&host_of(&url).unwrap());
        let out = scratch("big.bin");
        let client = build_worker_client().unwrap();
        let got = download_attempt(&client, &url, &out, &sink(), None, None, true, &FAST).await;
        assert!(got.is_ok(), "{got:?}");
        assert_eq!(std::fs::read(&out).unwrap(), body);
        let gets: Vec<(u64, Option<u64>)> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "GET")
            .map(|r| {
                r.range()
                    .expect("every GET is a Range segment, never one stream")
            })
            .collect();
        assert!(gets.iter().all(|(_, end)| end.is_some()), "{gets:?}");
        let starts: std::collections::HashSet<u64> = gets.iter().map(|(s, _)| *s).collect();
        assert!(
            starts.contains(&0) && starts.len() >= 2,
            "expected several Range segments: {gets:?}"
        );
    }

    async fn busy_server(body: Vec<u8>, status: u16, fail: usize) -> (String, test_server::Log) {
        test_server::spawn(move |idx, req| {
            if idx < fail {
                return test_server::Reply::new(status, b"try later".to_vec())
                    .header("Retry-After", "7");
            }
            test_server::serve_bytes(&body, req)
        })
        .await
    }

    #[tokio::test]
    async fn transient_503_at_the_probe_ends_the_worker_attempt() {
        let body = payload(50_000);
        let (base, log) = busy_server(body, 503, 1).await;
        let out = scratch("flaky.mp4");
        let client = build_worker_client().unwrap();
        let got = download_attempt(
            &client,
            &format!("{base}/flaky.mp4"),
            &out,
            &sink(),
            None,
            None,
            true,
            &FAST,
        )
        .await;
        let err = got.expect_err("a 503 must end the attempt");
        assert!(err.to_string().starts_with("HTTP 503"), "{err}");
        assert!(!is_fatal_error(&err), "503 stays retryable");
        assert_eq!(log.lock().unwrap().len(), 1, "no request after the 503");
        assert!(!out.exists());
    }

    #[tokio::test]
    async fn rate_limit_at_the_probe_is_typed_with_retry_after() {
        let (base, log) = busy_server(payload(50_000), 429, 1).await;
        let out = scratch("limited.mp4");
        let client = build_worker_client().unwrap();
        let err = download_attempt(
            &client,
            &format!("{base}/limited.mp4"),
            &out,
            &sink(),
            None,
            None,
            true,
            &FAST,
        )
        .await
        .expect_err("429 ends the attempt");
        let rl = err.downcast_ref::<RateLimitError>().expect("typed 429");
        assert_eq!(rl.retry_after_seconds, Some(7));
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn head_refused_still_falls_back_to_a_ranged_probe() {
        let body = payload(50_000);
        let srv = body.clone();
        let (base, _log) = test_server::spawn(move |_, req| {
            if req.method == "HEAD" {
                return test_server::Reply::new(405, Vec::new());
            }
            test_server::serve_bytes(&srv, req)
        })
        .await;
        let out = scratch("nohead.mp4");
        let client = build_worker_client().unwrap();
        let got = download_attempt(
            &client,
            &format!("{base}/nohead.mp4"),
            &out,
            &sink(),
            None,
            None,
            true,
            &FAST,
        )
        .await;
        assert!(got.is_ok(), "{got:?}");
        assert_eq!(std::fs::read(&out).unwrap(), body);
    }

    #[test]
    fn discard_part_takes_the_resume_sidecar_with_it() {
        let out = scratch("sidecar.mp4");
        let part = part_path_for(&out);
        let sidecar = sidecar_path_for(&part);
        let mut tmp = sidecar.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        for f in [&part, &sidecar, &tmp] {
            std::fs::write(f, b"x").unwrap();
        }
        discard_part(&part).unwrap();
        assert!(!part.exists() && !sidecar.exists() && !tmp.exists());
    }

    #[tokio::test]
    async fn failed_segmented_worker_attempt_leaves_no_stray_file() {
        let body = payload(21 * 1024 * 1024);
        let (base, _log) = test_server::spawn(move |_, req| {
            if req.method == "HEAD" {
                return test_server::serve_bytes(&body, req);
            }
            test_server::Reply::new(500, b"gone".to_vec())
        })
        .await;
        let out = scratch("seg-fail.mp4");
        let client = build_worker_client().unwrap();
        let got = download_attempt(
            &client,
            &format!("{base}/seg-fail.mp4"),
            &out,
            &sink(),
            None,
            None,
            true,
            &FAST,
        )
        .await;
        assert!(got.is_err());
        let left: Vec<_> = std::fs::read_dir(out.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            left.is_empty(),
            "stray files after a failed attempt: {left:?}"
        );
    }

    #[test]
    fn idle_timeout_message_matches_the_real_window() {
        let msg = idle_timeout_error(CHUNK_TIMEOUT).to_string();
        assert!(msg.contains("45 seconds"), "{msg}");
        assert_eq!(STREAM_POLICY.idle, CHUNK_TIMEOUT);
    }

    #[test]
    fn retry_after_delta_and_http_date() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-25T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(parse_retry_after("60", now), Some(60));
        assert_eq!(
            parse_retry_after("Fri, 25 Sep 2026 00:01:00 GMT", now),
            Some(60)
        );
        assert_eq!(
            parse_retry_after("Thu, 24 Sep 2026 23:59:00 GMT", now),
            Some(0)
        );
        assert_eq!(parse_retry_after("secret-cookie", now), None);
        assert_eq!(
            RateLimitError {
                retry_after_seconds: Some(60)
            }
            .to_string(),
            "HTTP 429 rate limited"
        );
    }
}
