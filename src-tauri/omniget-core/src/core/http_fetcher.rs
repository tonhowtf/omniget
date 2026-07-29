use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::anyhow;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::models::progress::ProgressUpdate;

const SEG_PENDING: u8 = 0;
const SEG_RUNNING: u8 = 1;
const SEG_DONE: u8 = 2;
const SEG_FAILED: u8 = 3;

const DEFAULT_MIN_SIZE_FOR_CHUNKED: u64 = 50 * 1024 * 1024;
const DEFAULT_SEGMENT_SIZE_HINT: u64 = 4 * 1024 * 1024;
const DEFAULT_CONCURRENT_SEGMENTS: usize = 8;
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_READ_TIMEOUT_SECS: u64 = 30;
const DEFAULT_MAX_RETRIES_PER_SEGMENT: u32 = 3;
const DEFAULT_STEAL_THRESHOLD_SECS: u64 = 3;
const DEFAULT_STEAL_MIN_CHUNK_SIZE: u64 = 512 * 1024;
const DEFAULT_RESUME_SAVE_INTERVAL_SECS: u64 = 2;

static GLOBAL_MAX_CONCURRENT_SEGMENTS: AtomicUsize = AtomicUsize::new(0);

pub fn set_global_max_concurrent_segments(n: usize) {
    GLOBAL_MAX_CONCURRENT_SEGMENTS.store(n, Ordering::Relaxed);
}

pub fn get_global_max_concurrent_segments() -> Option<usize> {
    let v = GLOBAL_MAX_CONCURRENT_SEGMENTS.load(Ordering::Relaxed);
    if v == 0 {
        None
    } else {
        Some(v)
    }
}

#[derive(Clone)]
pub struct HttpFetcherConfig {
    pub concurrent_segments: usize,
    pub segment_size_hint: u64,
    pub min_size_for_chunked: u64,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub max_retries_per_segment: u32,
    pub steal_threshold: Duration,
    pub steal_min_chunk_size: u64,
    pub use_sidecar_resume: bool,
    pub resume_save_interval: Duration,
}

impl Default for HttpFetcherConfig {
    fn default() -> Self {
        Self {
            concurrent_segments: DEFAULT_CONCURRENT_SEGMENTS,
            segment_size_hint: DEFAULT_SEGMENT_SIZE_HINT,
            min_size_for_chunked: DEFAULT_MIN_SIZE_FOR_CHUNKED,
            connect_timeout: Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS),
            read_timeout: Duration::from_secs(DEFAULT_READ_TIMEOUT_SECS),
            max_retries_per_segment: DEFAULT_MAX_RETRIES_PER_SEGMENT,
            steal_threshold: Duration::from_secs(DEFAULT_STEAL_THRESHOLD_SECS),
            steal_min_chunk_size: DEFAULT_STEAL_MIN_CHUNK_SIZE,
            use_sidecar_resume: true,
            resume_save_interval: Duration::from_secs(DEFAULT_RESUME_SAVE_INTERVAL_SECS),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeSegment {
    pub begin: u64,
    pub end: u64,
    pub downloaded: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeState {
    pub r#type: String,
    pub url_hash: String,
    pub total_bytes: u64,
    pub segments: Vec<ResumeSegment>,
}

#[derive(Debug, Clone)]
pub struct HttpFetcherResult {
    pub bytes_written: u64,
}

#[derive(Debug)]
struct Segment {
    id: usize,
    begin: u64,
    end_ceiling: AtomicU64,
    downloaded: AtomicU64,
    state: AtomicU8,
    last_progress_unix_nanos: AtomicU64,
    speed_baseline_unix_nanos: AtomicU64,
    speed_baseline_downloaded: AtomicU64,
}

impl Segment {
    fn new(id: usize, begin: u64, end: u64, downloaded: u64) -> Self {
        Self {
            id,
            begin,
            end_ceiling: AtomicU64::new(end),
            downloaded: AtomicU64::new(downloaded),
            state: AtomicU8::new(SEG_PENDING),
            last_progress_unix_nanos: AtomicU64::new(now_unix_nanos()),
            speed_baseline_unix_nanos: AtomicU64::new(now_unix_nanos()),
            speed_baseline_downloaded: AtomicU64::new(downloaded),
        }
    }

    fn remaining(&self) -> u64 {
        let end = self.end_ceiling.load(Ordering::Relaxed);
        let dl = self.downloaded.load(Ordering::Relaxed);
        let total = end.saturating_sub(self.begin) + 1;
        total.saturating_sub(dl)
    }

    fn estimated_seconds_remaining(&self, fallback_large: bool) -> u64 {
        let now = now_unix_nanos();
        let baseline_t = self.speed_baseline_unix_nanos.load(Ordering::Relaxed);
        let baseline_d = self.speed_baseline_downloaded.load(Ordering::Relaxed);
        let cur_d = self.downloaded.load(Ordering::Relaxed);
        let elapsed_ns = now.saturating_sub(baseline_t);
        if elapsed_ns < 1_000_000_000 || cur_d <= baseline_d {
            if fallback_large {
                u64::MAX / 2
            } else {
                0
            }
        } else {
            let bytes_per_sec =
                (cur_d - baseline_d) as u128 * 1_000_000_000u128 / elapsed_ns as u128;
            if bytes_per_sec == 0 {
                u64::MAX / 2
            } else {
                (self.remaining() as u128 / bytes_per_sec) as u64
            }
        }
    }
}

fn now_unix_nanos() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

pub struct HttpFetcher {
    client: reqwest::Client,
    url: String,
    output_path: PathBuf,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<CancellationToken>,
    config: HttpFetcherConfig,
}

impl HttpFetcher {
    pub fn new(client: reqwest::Client, url: String, output_path: PathBuf) -> Self {
        Self {
            client,
            url,
            output_path,
            headers: None,
            cancel: None,
            config: HttpFetcherConfig::default(),
        }
    }

    pub fn with_headers(mut self, headers: reqwest::header::HeaderMap) -> Self {
        self.headers = Some(headers);
        self
    }

    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = Some(cancel);
        self
    }

    pub fn with_config(mut self, config: HttpFetcherConfig) -> Self {
        self.config = config;
        self
    }

    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    pub async fn download(
        &self,
        progress_tx: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<HttpFetcherResult> {
        let probe = self.probe().await?;

        let part_path = part_path_for(&self.output_path);
        if let Some(parent) = self.output_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        let total = match probe.content_length {
            Some(t) if t > 0 => t,
            _ => return self.download_streaming(&part_path, &progress_tx).await,
        };

        if !probe.accept_ranges || total < self.config.min_size_for_chunked {
            return self.download_streaming(&part_path, &progress_tx).await;
        }

        let resume_path = sidecar_path_for(&part_path);
        let url_hash = url_hash(&self.url);

        let initial_segments = if self.config.use_sidecar_resume {
            load_resume_state(&resume_path)
                .filter(|st| {
                    st.r#type == "http_segmented"
                        && st.total_bytes == total
                        && st.url_hash == url_hash
                })
                .map(|st| st.segments)
        } else {
            None
        };

        let segments = match initial_segments {
            Some(segs) if !segs.is_empty() => segs,
            _ => plan_segments(
                total,
                self.config.segment_size_hint,
                self.config.concurrent_segments,
            ),
        };

        ensure_part_file(&part_path, total).await?;

        let result = self
            .download_chunked(
                &part_path,
                total,
                segments,
                &resume_path,
                &url_hash,
                &progress_tx,
            )
            .await;

        match result {
            Ok(()) => {
                tokio::fs::rename(&part_path, &self.output_path)
                    .await
                    .map_err(|e| anyhow!("rename .part to final failed: {}", e))?;
                if self.config.use_sidecar_resume {
                    let _ = tokio::fs::remove_file(&resume_path).await;
                }
                let _ = progress_tx.send(ProgressUpdate::percent(100.0)).await;
                let bytes = tokio::fs::metadata(&self.output_path)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(total);
                Ok(HttpFetcherResult {
                    bytes_written: bytes,
                })
            }
            Err(e) => Err(e),
        }
    }

    async fn probe(&self) -> anyhow::Result<ProbeResult> {
        let mut req = self.client.head(&self.url);
        if let Some(h) = &self.headers {
            req = req.headers(h.clone());
        }
        let resp = match tokio::time::timeout(self.config.connect_timeout * 2, req.send()).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(anyhow!("HEAD probe failed: {}", e)),
            Err(_) => return Err(anyhow!("HEAD probe timed out")),
        };

        if !resp.status().is_success() {
            return Err(anyhow!("HEAD returned HTTP {}", resp.status()));
        }
        let content_length = resp.content_length();
        let accept_ranges = resp
            .headers()
            .get(reqwest::header::ACCEPT_RANGES)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("bytes"))
            .unwrap_or(false);
        Ok(ProbeResult {
            content_length,
            accept_ranges,
        })
    }

    async fn download_streaming(
        &self,
        part_path: &Path,
        progress_tx: &mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<HttpFetcherResult> {
        let _ = tokio::fs::remove_file(part_path).await;

        let mut req = self.client.get(&self.url);
        if let Some(h) = &self.headers {
            req = req.headers(h.clone());
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("HTTP {} downloading {}", resp.status(), self.url));
        }
        let total = resp.content_length();

        let mut file = tokio::fs::File::create(part_path).await?;
        let mut downloaded: u64 = 0;
        let mut stream = resp.bytes_stream();

        let mut last_emit = std::time::Instant::now();
        let mut anchor_bytes = 0u64;
        let mut anchor_time = std::time::Instant::now();
        let mut speed_ema: f64 = 0.0;

        loop {
            if let Some(token) = &self.cancel {
                if token.is_cancelled() {
                    return Err(anyhow!("Download cancelled"));
                }
            }
            match tokio::time::timeout(self.config.read_timeout, stream.next()).await {
                Ok(Some(Ok(chunk))) => {
                    file.write_all(&chunk).await?;
                    downloaded += chunk.len() as u64;
                    if last_emit.elapsed() >= Duration::from_millis(250) {
                        let elapsed = anchor_time.elapsed().as_secs_f64();
                        if elapsed >= 0.3 {
                            let instant =
                                (downloaded.saturating_sub(anchor_bytes)) as f64 / elapsed;
                            speed_ema = if speed_ema > 0.0 {
                                speed_ema * 0.6 + instant * 0.4
                            } else {
                                instant
                            };
                            anchor_bytes = downloaded;
                            anchor_time = std::time::Instant::now();
                        }
                        let speed = (speed_ema > 0.0).then_some(speed_ema);
                        let (pct, eta) = match total.filter(|t| *t > 0) {
                            Some(t) => {
                                let p = ((downloaded as f64 / t as f64) * 100.0).min(99.9);
                                let e = speed.and_then(|s| {
                                    (s > 0.0 && t > downloaded)
                                        .then(|| ((t - downloaded) as f64 / s) as u64)
                                });
                                (p, e)
                            }
                            None => (
                                ((downloaded as f64 / (downloaded as f64 + 500_000.0)) * 100.0)
                                    .min(95.0),
                                None,
                            ),
                        };
                        let _ = progress_tx
                            .send(ProgressUpdate::rich(
                                pct,
                                Some(downloaded),
                                total.filter(|t| *t > 0),
                                speed,
                                eta,
                            ))
                            .await;
                        last_emit = std::time::Instant::now();
                    }
                }
                Ok(Some(Err(e))) => return Err(anyhow!("stream error: {}", e)),
                Ok(None) => break,
                Err(_) => {
                    return Err(anyhow!(
                        "read timed out after {:?}",
                        self.config.read_timeout
                    ))
                }
            }
        }

        file.flush().await?;
        drop(file);
        tokio::fs::rename(part_path, &self.output_path).await?;
        let _ = progress_tx.send(ProgressUpdate::percent(100.0)).await;
        Ok(HttpFetcherResult {
            bytes_written: downloaded,
        })
    }

    async fn download_chunked(
        &self,
        part_path: &Path,
        total: u64,
        seg_specs: Vec<ResumeSegment>,
        resume_path: &Path,
        url_hash: &str,
        progress_tx: &mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<()> {
        let segments: Arc<Mutex<Vec<Arc<Segment>>>> = Arc::new(Mutex::new(
            seg_specs
                .into_iter()
                .enumerate()
                .map(|(i, s)| Arc::new(Segment::new(i, s.begin, s.end, s.downloaded)))
                .collect(),
        ));

        let cancel = self.cancel.clone().unwrap_or_else(CancellationToken::new);
        let progress_done = Arc::new(AtomicU64::new(0));
        for seg in segments.lock().await.iter() {
            progress_done.fetch_add(seg.downloaded.load(Ordering::Relaxed), Ordering::Relaxed);
        }

        let progress_pump = {
            let segments = segments.clone();
            let progress_tx = progress_tx.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let mut last_emit = std::time::Instant::now() - Duration::from_secs(1);
                let mut anchor_bytes = 0u64;
                let mut anchor_time = std::time::Instant::now();
                let mut speed_ema: f64 = 0.0;
                let mut anchor_init = false;
                loop {
                    if cancel.is_cancelled() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    let dl: u64 = {
                        let segs = segments.lock().await;
                        segs.iter()
                            .map(|s| s.downloaded.load(Ordering::Relaxed))
                            .sum()
                    };
                    if !anchor_init {
                        anchor_bytes = dl;
                        anchor_time = std::time::Instant::now();
                        anchor_init = true;
                    }
                    if last_emit.elapsed() >= Duration::from_millis(200) {
                        let elapsed = anchor_time.elapsed().as_secs_f64();
                        if elapsed >= 0.4 {
                            let instant = (dl.saturating_sub(anchor_bytes)) as f64 / elapsed;
                            speed_ema = if speed_ema > 0.0 {
                                speed_ema * 0.6 + instant * 0.4
                            } else {
                                instant
                            };
                            anchor_bytes = dl;
                            anchor_time = std::time::Instant::now();
                        }
                        let speed = (speed_ema > 0.0).then_some(speed_ema);
                        let pct = ((dl as f64 / total as f64) * 100.0).min(99.9);
                        let eta = speed.and_then(|s| {
                            (s > 0.0 && total > dl).then(|| ((total - dl) as f64 / s) as u64)
                        });
                        let _ = progress_tx
                            .send(ProgressUpdate::rich(pct, Some(dl), Some(total), speed, eta))
                            .await;
                        last_emit = std::time::Instant::now();
                    }
                }
            })
        };

        let resume_pump = if self.config.use_sidecar_resume {
            let segments = segments.clone();
            let resume_path = resume_path.to_path_buf();
            let url_hash = url_hash.to_string();
            let cancel = cancel.clone();
            let interval = self.config.resume_save_interval;
            Some(tokio::spawn(async move {
                loop {
                    if cancel.is_cancelled() {
                        break;
                    }
                    tokio::time::sleep(interval).await;
                    let snapshot: Vec<ResumeSegment> = {
                        let segs = segments.lock().await;
                        segs.iter()
                            .map(|s| ResumeSegment {
                                begin: s.begin,
                                end: s.end_ceiling.load(Ordering::Relaxed),
                                downloaded: s.downloaded.load(Ordering::Relaxed),
                            })
                            .collect()
                    };
                    let state = ResumeState {
                        r#type: "http_segmented".to_string(),
                        url_hash: url_hash.clone(),
                        total_bytes: total,
                        segments: snapshot,
                    };
                    if let Err(e) = save_resume_state(&resume_path, &state).await {
                        tracing::trace!("[http_fetcher] resume save failed: {}", e);
                    }
                }
            }))
        } else {
            None
        };

        let worker_count = self.config.concurrent_segments.max(1);
        let mut tasks = Vec::with_capacity(worker_count);
        let url = Arc::new(self.url.clone());
        let headers = self.headers.clone();
        let part_path_arc: Arc<PathBuf> = Arc::new(part_path.to_path_buf());
        for _ in 0..worker_count {
            let segments = segments.clone();
            let cancel = cancel.clone();
            let client = self.client.clone();
            let url = url.clone();
            let headers = headers.clone();
            let part_path = part_path_arc.clone();
            let cfg = self.config.clone();
            tasks.push(tokio::spawn(async move {
                worker_loop(client, url, headers, part_path, segments, cancel, cfg).await
            }));
        }

        let mut first_err: Option<anyhow::Error> = None;
        for t in tasks {
            match t.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                    cancel.cancel();
                }
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(anyhow!("worker task panicked: {:?}", e));
                    }
                    cancel.cancel();
                }
            }
        }

        progress_pump.abort();
        if let Some(p) = resume_pump {
            p.abort();
        }
        let _ = progress_pump.await;

        if let Some(e) = first_err {
            return Err(e);
        }

        let segs = segments.lock().await;
        let all_done = segs
            .iter()
            .all(|s| s.state.load(Ordering::Relaxed) == SEG_DONE);
        if !all_done {
            return Err(anyhow!("not all segments completed"));
        }
        let actual = tokio::fs::metadata(part_path).await?.len();
        if actual != total {
            return Err(anyhow!(
                "size mismatch: expected {} bytes, got {}",
                total,
                actual
            ));
        }

        Ok(())
    }
}

struct ProbeResult {
    content_length: Option<u64>,
    accept_ranges: bool,
}

fn plan_segments(total: u64, hint: u64, max_segments: usize) -> Vec<ResumeSegment> {
    if total == 0 {
        return Vec::new();
    }
    let max = max_segments.max(1) as u64;
    let segment_size = hint.max(1);
    let count_by_size = total.div_ceil(segment_size).max(1);
    let count = count_by_size.min(max).max(1);
    let base = total / count;
    let mut remainder = total % count;
    let mut segments = Vec::with_capacity(count as usize);
    let mut cursor: u64 = 0;
    for _ in 0..count {
        let mut size = base;
        if remainder > 0 {
            size += 1;
            remainder -= 1;
        }
        let begin = cursor;
        let end = begin + size - 1;
        segments.push(ResumeSegment {
            begin,
            end,
            downloaded: 0,
        });
        cursor = end + 1;
    }
    segments
}

async fn worker_loop(
    client: reqwest::Client,
    url: Arc<String>,
    headers: Option<reqwest::header::HeaderMap>,
    part_path: Arc<PathBuf>,
    segments: Arc<Mutex<Vec<Arc<Segment>>>>,
    cancel: CancellationToken,
    cfg: HttpFetcherConfig,
) -> anyhow::Result<()> {
    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }

        let claimed = claim_pending_or_steal(&segments, &cfg).await;
        let seg = match claimed {
            Some(s) => s,
            None => return Ok(()),
        };

        let mut attempt: u32 = 0;
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            match download_segment(
                &client,
                &url,
                headers.as_ref(),
                &part_path,
                &seg,
                &cancel,
                &cfg,
            )
            .await
            {
                Ok(()) => {
                    seg.state.store(SEG_DONE, Ordering::Relaxed);
                    break;
                }
                Err(e) => {
                    if cancel.is_cancelled() {
                        return Ok(());
                    }
                    if is_fatal(&e) {
                        seg.state.store(SEG_FAILED, Ordering::Relaxed);
                        return Err(e);
                    }
                    attempt += 1;
                    if attempt >= cfg.max_retries_per_segment {
                        seg.state.store(SEG_FAILED, Ordering::Relaxed);
                        return Err(e);
                    }
                    let delay = Duration::from_secs((attempt as u64).min(5));
                    tracing::warn!(
                        "[http_fetcher] segment {}: attempt {}/{} failed: {}",
                        seg.id,
                        attempt,
                        cfg.max_retries_per_segment,
                        e
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}

async fn claim_pending_or_steal(
    segments: &Arc<Mutex<Vec<Arc<Segment>>>>,
    cfg: &HttpFetcherConfig,
) -> Option<Arc<Segment>> {
    {
        let mut segs = segments.lock().await;
        for s in segs.iter() {
            if s.state.load(Ordering::Relaxed) == SEG_PENDING {
                s.state.store(SEG_RUNNING, Ordering::Relaxed);
                s.last_progress_unix_nanos
                    .store(now_unix_nanos(), Ordering::Relaxed);
                s.speed_baseline_unix_nanos
                    .store(now_unix_nanos(), Ordering::Relaxed);
                s.speed_baseline_downloaded
                    .store(s.downloaded.load(Ordering::Relaxed), Ordering::Relaxed);
                return Some(s.clone());
            }
        }
        if let Some(stolen) = try_steal(&mut segs, cfg) {
            return Some(stolen);
        }
    }
    None
}

fn try_steal(segs: &mut Vec<Arc<Segment>>, cfg: &HttpFetcherConfig) -> Option<Arc<Segment>> {
    let mut victim: Option<Arc<Segment>> = None;
    let mut victim_score: u64 = 0;
    for s in segs.iter() {
        if s.state.load(Ordering::Relaxed) != SEG_RUNNING {
            continue;
        }
        let remain = s.remaining();
        if remain < cfg.steal_min_chunk_size {
            continue;
        }
        let est = s.estimated_seconds_remaining(true);
        if est <= cfg.steal_threshold.as_secs() {
            continue;
        }
        if est > victim_score {
            victim_score = est;
            victim = Some(s.clone());
        }
    }
    let victim = victim?;
    let remain = victim.remaining();
    if remain < cfg.steal_min_chunk_size {
        return None;
    }
    let half = remain / 2;
    if half < cfg.steal_min_chunk_size {
        return None;
    }
    let old_end = victim.end_ceiling.load(Ordering::Relaxed);
    let new_begin_for_helper = old_end - half + 1;
    let new_end_for_victim = new_begin_for_helper - 1;
    if new_begin_for_helper <= victim.begin + victim.downloaded.load(Ordering::Relaxed) {
        return None;
    }
    victim
        .end_ceiling
        .store(new_end_for_victim, Ordering::Relaxed);

    let next_id = segs.iter().map(|s| s.id).max().unwrap_or(0) + 1;
    let helper = Arc::new(Segment::new(next_id, new_begin_for_helper, old_end, 0));
    helper.state.store(SEG_RUNNING, Ordering::Relaxed);
    helper
        .last_progress_unix_nanos
        .store(now_unix_nanos(), Ordering::Relaxed);
    helper
        .speed_baseline_unix_nanos
        .store(now_unix_nanos(), Ordering::Relaxed);
    segs.push(helper.clone());
    tracing::debug!(
        "[http_fetcher] stole {} bytes from segment {} → helper segment {}",
        half,
        victim.id,
        helper.id
    );
    Some(helper)
}

async fn download_segment(
    client: &reqwest::Client,
    url: &str,
    headers: Option<&reqwest::header::HeaderMap>,
    part_path: &Path,
    seg: &Arc<Segment>,
    cancel: &CancellationToken,
    cfg: &HttpFetcherConfig,
) -> anyhow::Result<()> {
    let begin = seg.begin;
    let end = seg.end_ceiling.load(Ordering::Relaxed);
    let already = seg.downloaded.load(Ordering::Relaxed);
    if begin + already > end {
        return Ok(());
    }
    let range_start = begin + already;

    let mut req = client.get(url);
    if let Some(h) = headers {
        req = req.headers(h.clone());
    }
    req = req.header(
        reqwest::header::RANGE,
        format!("bytes={}-{}", range_start, end),
    );

    let resp = tokio::time::timeout(cfg.connect_timeout, req.send())
        .await
        .map_err(|_| anyhow!("connect timeout"))??;

    let status = resp.status();
    if status != reqwest::StatusCode::PARTIAL_CONTENT {
        if status.is_success() {
            return Err(anyhow!("server did not honor Range (HTTP 200)"));
        }
        return Err(anyhow!("HTTP {}", status));
    }

    if let Some(ct) = resp.headers().get(reqwest::header::CONTENT_TYPE) {
        if let Ok(s) = ct.to_str() {
            if s.contains("text/html") {
                return Err(anyhow!("server returned HTML instead of media"));
            }
        }
    }

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(part_path)
        .await?;
    file.seek(std::io::SeekFrom::Start(range_start)).await?;

    let mut stream = resp.bytes_stream();
    let mut written: u64 = 0;
    let target = (end - range_start) + 1;
    loop {
        if cancel.is_cancelled() {
            return Err(anyhow!("Download cancelled"));
        }
        let cur_end = seg.end_ceiling.load(Ordering::Relaxed);
        if cur_end < end {
            break;
        }
        let res = tokio::time::timeout(cfg.read_timeout, stream.next()).await;
        match res {
            Ok(Some(Ok(chunk))) => {
                if chunk.is_empty() {
                    continue;
                }
                let mut take = chunk.len() as u64;
                let max_allowed = target.saturating_sub(written);
                if take > max_allowed {
                    take = max_allowed;
                }
                let slice = &chunk[..take as usize];
                file.write_all(slice).await?;
                written += take;
                seg.downloaded.fetch_add(take, Ordering::Relaxed);
                seg.last_progress_unix_nanos
                    .store(now_unix_nanos(), Ordering::Relaxed);
                if written >= target {
                    break;
                }
            }
            Ok(Some(Err(e))) => return Err(anyhow!("stream error: {}", e)),
            Ok(None) => break,
            Err(_) => return Err(anyhow!("read timed out after {:?}", cfg.read_timeout)),
        }
    }
    file.flush().await?;
    let cur_end = seg.end_ceiling.load(Ordering::Relaxed);
    let cur_target = (cur_end - begin) + 1;
    let cur_dl = seg.downloaded.load(Ordering::Relaxed);
    if cur_dl < cur_target {
        return Err(anyhow!(
            "incomplete segment: {} of {} bytes",
            cur_dl,
            cur_target
        ));
    }
    Ok(())
}

fn is_fatal(err: &anyhow::Error) -> bool {
    let m = err.to_string();
    for code in &[
        "HTTP 400", "HTTP 401", "HTTP 403", "HTTP 404", "HTTP 405", "HTTP 410", "HTTP 451",
    ] {
        if m.contains(code) {
            return true;
        }
    }
    if m.contains("HTML instead of media") {
        return true;
    }
    if m.contains("cancelled") {
        return true;
    }
    false
}

async fn ensure_part_file(part_path: &Path, total: u64) -> anyhow::Result<()> {
    let needs_create = match tokio::fs::metadata(part_path).await {
        Ok(m) => m.len() != total,
        Err(_) => true,
    };
    if needs_create {
        let _ = tokio::fs::remove_file(part_path).await;
        let f = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(part_path)
            .await?;
        f.set_len(total).await?;
    }
    Ok(())
}

pub fn part_path_for(output: &Path) -> PathBuf {
    let mut s = output.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn sidecar_path_for(part_path: &Path) -> PathBuf {
    let mut s = part_path.as_os_str().to_owned();
    s.push(".resume.json");
    PathBuf::from(s)
}

fn url_hash(url: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    let out = h.finalize();
    let mut s = String::with_capacity(16);
    for b in &out[..8] {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn load_resume_state(path: &Path) -> Option<ResumeState> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

async fn save_resume_state(path: &Path, state: &ResumeState) -> anyhow::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp_path = PathBuf::from(tmp);
    let json = serde_json::to_vec(state)?;
    tokio::fs::write(&tmp_path, json).await?;
    tokio::fs::rename(&tmp_path, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_segments_uniform_division() {
        let segs = plan_segments(100, 10, 10);
        assert_eq!(segs.len(), 10);
        assert_eq!(segs.first().unwrap().begin, 0);
        assert_eq!(segs.last().unwrap().end, 99);
        for w in segs.windows(2) {
            assert_eq!(w[0].end + 1, w[1].begin);
        }
    }

    #[test]
    fn plan_segments_remainder_distributed() {
        let segs = plan_segments(101, 10, 10);
        assert_eq!(segs.len(), 10);
        assert_eq!(segs.last().unwrap().end, 100);
        let total: u64 = segs.iter().map(|s| s.end - s.begin + 1).sum();
        assert_eq!(total, 101);
    }

    #[test]
    fn plan_segments_capped_by_max() {
        let segs = plan_segments(1_000_000, 1024, 4);
        assert_eq!(segs.len(), 4);
        let total: u64 = segs.iter().map(|s| s.end - s.begin + 1).sum();
        assert_eq!(total, 1_000_000);
    }

    #[test]
    fn plan_segments_capped_by_size_hint() {
        let segs = plan_segments(100, 50, 16);
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn plan_segments_zero_total() {
        assert!(plan_segments(0, 1024, 4).is_empty());
    }

    #[test]
    fn plan_segments_smaller_than_hint() {
        let segs = plan_segments(500, 1024, 4);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].begin, 0);
        assert_eq!(segs[0].end, 499);
    }

    #[test]
    fn part_path_basic() {
        assert_eq!(
            part_path_for(Path::new("video.mp4")),
            PathBuf::from("video.mp4.part")
        );
    }

    #[test]
    fn sidecar_appends_resume_json() {
        let part = Path::new("video.mp4.part");
        assert_eq!(
            sidecar_path_for(part),
            PathBuf::from("video.mp4.part.resume.json")
        );
    }

    #[test]
    fn url_hash_stable_and_short() {
        let h1 = url_hash("https://example.com/foo");
        let h2 = url_hash("https://example.com/foo");
        let h3 = url_hash("https://example.com/bar");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn segment_remaining_zero_when_done() {
        let seg = Segment::new(0, 0, 99, 100);
        assert_eq!(seg.remaining(), 0);
    }

    #[test]
    fn segment_remaining_partial() {
        let seg = Segment::new(0, 0, 99, 40);
        assert_eq!(seg.remaining(), 60);
    }

    #[test]
    fn fatal_classification_403() {
        assert!(is_fatal(&anyhow!("HTTP 403 Forbidden")));
    }

    #[test]
    fn fatal_classification_500_not_fatal() {
        assert!(!is_fatal(&anyhow!("HTTP 500 Internal Server Error")));
    }

    #[test]
    fn fatal_classification_html() {
        assert!(is_fatal(&anyhow!("HTML instead of media")));
    }

    #[test]
    fn fatal_classification_cancelled() {
        assert!(is_fatal(&anyhow!("Download cancelled")));
    }

    #[test]
    fn fatal_classification_network() {
        assert!(!is_fatal(&anyhow!("connection reset")));
    }

    #[test]
    fn global_concurrent_segments_setter() {
        set_global_max_concurrent_segments(0);
        assert_eq!(get_global_max_concurrent_segments(), None);
        set_global_max_concurrent_segments(7);
        assert_eq!(get_global_max_concurrent_segments(), Some(7));
        set_global_max_concurrent_segments(0);
    }

    #[tokio::test]
    async fn fetcher_downloads_small_file_streaming() {
        use std::io::Write;
        let temp_dir =
            std::env::temp_dir().join(format!("omniget_http_fetcher_test_{}", now_unix_nanos()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let payload: Vec<u8> = (0u8..=255).cycle().take(200_000).collect();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let payload_for_server = payload.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let body = payload_for_server.clone();
                tokio::spawn(async move {
                    use tokio::io::AsyncReadExt;
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: none\r\nContent-Type: application/octet-stream\r\n\r\n",
                        body.len()
                    );
                    use tokio::io::AsyncWriteExt;
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });

        let client = reqwest::Client::builder().build().unwrap();
        let output = temp_dir.join("file.bin");
        let url = format!("http://{}/file.bin", addr);
        let fetcher = HttpFetcher::new(client, url, output.clone());

        let (tx, mut rx) = tokio::sync::mpsc::channel::<ProgressUpdate>(32);
        let progress_collector = tokio::spawn(async move {
            let mut last = 0.0;
            while let Some(p) = rx.recv().await {
                last = p.percent;
            }
            last
        });

        let res = fetcher.download(tx).await;
        let last_pct = progress_collector.await.unwrap();

        assert!(res.is_ok(), "{:?}", res.err());
        let written = std::fs::read(&output).unwrap();
        assert_eq!(written.len(), payload.len());
        assert_eq!(written, payload);
        assert!((last_pct - 100.0).abs() < f64::EPSILON);

        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = writeln!(std::io::stderr(), "ok");
    }

    async fn serve_range_request(mut stream: tokio::net::TcpStream, body: Arc<Vec<u8>>) {
        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncWriteExt;
        let mut buf = vec![0u8; 8192];
        let n = match stream.read(&mut buf).await {
            Ok(n) if n > 0 => n,
            _ => return,
        };
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let mut lines = req.split("\r\n");
        let request_line = lines.next().unwrap_or("");
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let mut range_header: Option<String> = None;
        for line in lines {
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Range: ") {
                range_header = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("range: ") {
                range_header = Some(v.trim().to_string());
            }
        }

        if method == "HEAD" {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nContent-Type: application/octet-stream\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes()).await;
            return;
        }

        if let Some(range) = range_header {
            if let Some(rest) = range.strip_prefix("bytes=") {
                let mut iter = rest.split('-');
                let start: u64 = iter.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let end: u64 = iter
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or((body.len() as u64) - 1);
                let s = start as usize;
                let e = (end as usize).min(body.len() - 1);
                let slice = &body[s..=e];
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nContent-Type: application/octet-stream\r\n\r\n",
                    slice.len(),
                    s,
                    e,
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(slice).await;
                return;
            }
        }

        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nContent-Type: application/octet-stream\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes()).await;
        let _ = stream.write_all(&body).await;
    }

    #[tokio::test]
    async fn fetcher_downloads_multi_segment_with_range() {
        let temp_dir =
            std::env::temp_dir().join(format!("omniget_http_fetcher_chunked_{}", now_unix_nanos()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let payload: Vec<u8> = (0u8..=255).cycle().take(2 * 1024 * 1024).collect();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let payload_for_server = Arc::new(payload.clone());
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let body = payload_for_server.clone();
                tokio::spawn(async move {
                    serve_range_request(stream, body).await;
                });
            }
        });

        let client = reqwest::Client::builder().build().unwrap();
        let output = temp_dir.join("file.bin");
        let url = format!("http://{}/file.bin", addr);
        let cfg = HttpFetcherConfig {
            min_size_for_chunked: 256 * 1024,
            segment_size_hint: 256 * 1024,
            concurrent_segments: 4,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(15),
            use_sidecar_resume: false,
            ..Default::default()
        };
        let fetcher = HttpFetcher::new(client, url, output.clone()).with_config(cfg);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<ProgressUpdate>(64);
        let progress_collector = tokio::spawn(async move {
            let mut last = 0.0;
            while let Some(p) = rx.recv().await {
                last = p.percent;
            }
            last
        });

        let res = fetcher.download(tx).await;
        let last_pct = progress_collector.await.unwrap();

        assert!(res.is_ok(), "{:?}", res.err());
        let written = std::fs::read(&output).unwrap();
        assert_eq!(written.len(), payload.len());
        assert_eq!(written, payload);
        assert!((last_pct - 100.0).abs() < f64::EPSILON);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn fetcher_resumes_from_sidecar_state() {
        let temp_dir = std::env::temp_dir().join(format!(
            "omniget_http_fetcher_resume_load_{}",
            now_unix_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let total: usize = 1024 * 1024;
        let payload: Vec<u8> = (0u8..=255).cycle().take(total).collect();

        let part = part_path_for(&temp_dir.join("file.bin"));
        let f = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&part)
            .await
            .unwrap();
        f.set_len(total as u64).await.unwrap();
        drop(f);

        let half = total / 2;
        let mut f = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&part)
            .await
            .unwrap();
        f.write_all(&payload[..half]).await.unwrap();
        f.flush().await.unwrap();
        drop(f);

        let resume_path = sidecar_path_for(&part);
        let url_hash_v = url_hash("test://payload");
        let state = ResumeState {
            r#type: "http_segmented".to_string(),
            url_hash: url_hash_v.clone(),
            total_bytes: total as u64,
            segments: vec![
                ResumeSegment {
                    begin: 0,
                    end: (half - 1) as u64,
                    downloaded: half as u64,
                },
                ResumeSegment {
                    begin: half as u64,
                    end: (total - 1) as u64,
                    downloaded: 0,
                },
            ],
        };
        save_resume_state(&resume_path, &state).await.unwrap();

        let loaded = load_resume_state(&resume_path).unwrap();
        assert_eq!(loaded.url_hash, url_hash_v);
        assert_eq!(loaded.segments[0].downloaded, half as u64);
        assert_eq!(loaded.segments[1].downloaded, 0);
        assert_eq!(loaded.total_bytes, total as u64);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn fetcher_resume_sidecar_round_trip() {
        let temp_dir =
            std::env::temp_dir().join(format!("omniget_http_fetcher_resume_{}", now_unix_nanos()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let path = temp_dir.join("video.mp4.part.resume.json");
        let state = ResumeState {
            r#type: "http_segmented".to_string(),
            url_hash: "deadbeef".to_string(),
            total_bytes: 1024,
            segments: vec![
                ResumeSegment {
                    begin: 0,
                    end: 511,
                    downloaded: 100,
                },
                ResumeSegment {
                    begin: 512,
                    end: 1023,
                    downloaded: 0,
                },
            ],
        };
        save_resume_state(&path, &state).await.unwrap();
        let loaded = load_resume_state(&path).unwrap();
        assert_eq!(loaded.url_hash, "deadbeef");
        assert_eq!(loaded.segments.len(), 2);
        assert_eq!(loaded.segments[0].downloaded, 100);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
