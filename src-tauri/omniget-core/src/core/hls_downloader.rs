use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use m3u8_rs::{
    parse_master_playlist, parse_media_playlist, AlternativeMedia, AlternativeMediaType,
    MasterPlaylist, VariantStream,
};
use reqwest::Client;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::models::progress::ProgressUpdate;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub struct HlsDownloadResult {
    pub path: PathBuf,
    pub file_size: u64,
    pub segments: usize,
}

// WHY: a playlist can point its segments at any host, so credentials are chosen per request URL
// instead of being default headers on the client.
pub type RequestHeaders = Arc<dyn Fn(&str) -> reqwest::header::HeaderMap + Send + Sync>;

pub struct HlsDownloader {
    client: Client,
    request_headers: Option<RequestHeaders>,
    user_agent_override: Option<String>,
    progress_tx: Option<mpsc::Sender<ProgressUpdate>>,
    prefetched_playlist: Option<(String, String)>,
}

impl Default for HlsDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl HlsDownloader {
    pub fn new() -> Self {
        let builder = crate::core::http_client::apply_global_proxy(
            Client::builder()
                .connect_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(300))
                .pool_max_idle_per_host(50)
                .pool_idle_timeout(Duration::from_secs(30)),
        );
        let client = match builder.build() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("HLS client build failed, falling back to default: {}", e);
                Client::new()
            }
        };
        Self::with_client(client)
    }

    pub fn with_client(client: Client) -> Self {
        Self {
            client,
            request_headers: None,
            user_agent_override: None,
            progress_tx: None,
            prefetched_playlist: None,
        }
    }

    pub fn with_request_headers(mut self, headers: RequestHeaders) -> Self {
        self.request_headers = Some(headers);
        self
    }

    pub fn with_user_agent_override(mut self, ua: Option<String>) -> Self {
        self.user_agent_override = ua;
        self
    }

    pub fn with_progress(mut self, tx: mpsc::Sender<ProgressUpdate>) -> Self {
        self.progress_tx = Some(tx);
        self
    }

    pub fn with_prefetched_playlist(mut self, url: String, text: String) -> Self {
        self.prefetched_playlist = Some((url, text));
        self
    }

    fn prefetched_for(&self, url: &str) -> Option<&str> {
        match &self.prefetched_playlist {
            Some((stored_url, text)) if stored_url == url => Some(text.as_str()),
            _ => None,
        }
    }

    fn effective_user_agent(&self) -> &str {
        self.user_agent_override.as_deref().unwrap_or(USER_AGENT)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn download(
        &self,
        m3u8_url: &str,
        output_path: &str,
        referer: &str,
        bytes_tx: Option<UnboundedSender<u64>>,
        cancel_token: CancellationToken,
        max_concurrent: u32,
        max_retries: u32,
    ) -> anyhow::Result<HlsDownloadResult> {
        self.download_with_quality(
            m3u8_url,
            output_path,
            referer,
            bytes_tx,
            cancel_token,
            max_concurrent,
            max_retries,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn download_with_quality(
        &self,
        m3u8_url: &str,
        output_path: &str,
        referer: &str,
        bytes_tx: Option<UnboundedSender<u64>>,
        cancel_token: CancellationToken,
        max_concurrent: u32,
        max_retries: u32,
        max_height: Option<u32>,
    ) -> anyhow::Result<HlsDownloadResult> {
        if cancel_token.is_cancelled() {
            anyhow::bail!("Download cancelled by user");
        }

        let m3u8_text = self.fetch_m3u8_with_retry(m3u8_url, referer, 3).await?;

        let m3u8_bytes = m3u8_text.as_bytes();

        if let Ok((_, master)) = parse_master_playlist(m3u8_bytes) {
            if let Some(variant) = select_best_variant(&master, max_height) {
                let available: Vec<u64> = master
                    .variants
                    .iter()
                    .filter(|v| !v.is_i_frame)
                    .map(variant_height)
                    .collect();
                tracing::info!(
                    "[hls] variant selected: {}p (requested: {}, available: {:?})",
                    variant_height(variant),
                    max_height
                        .map(|h| format!("max {}p", h))
                        .unwrap_or_else(|| "best".to_string()),
                    available
                );
                let variant_url = resolve_url(m3u8_url, &variant.uri);
                if let Some(uri) =
                    select_audio_rendition(&master, variant).and_then(|a| a.uri.as_deref())
                {
                    let audio_url = resolve_url(m3u8_url, uri);
                    return self
                        .download_separate_tracks(
                            &variant_url,
                            &audio_url,
                            output_path,
                            referer,
                            bytes_tx,
                            cancel_token,
                            max_concurrent,
                            max_retries,
                        )
                        .await;
                }
                return self
                    .download_media_playlist(
                        &variant_url,
                        output_path,
                        referer,
                        bytes_tx,
                        cancel_token,
                        max_concurrent,
                        max_retries,
                    )
                    .await;
            }
        }

        if parse_media_playlist(m3u8_bytes).is_ok() {
            return self
                .download_media_playlist(
                    m3u8_url,
                    output_path,
                    referer,
                    bytes_tx,
                    cancel_token,
                    max_concurrent,
                    max_retries,
                )
                .await;
        }

        anyhow::bail!("Failed to parse m3u8: neither master nor media playlist")
    }

    async fn fetch_m3u8_with_retry(
        &self,
        url: &str,
        referer: &str,
        max_retries: u32,
    ) -> anyhow::Result<String> {
        if let Some(text) = self.prefetched_for(url) {
            tracing::info!(
                "[hls] using prefetched playlist text ({} bytes)",
                text.len()
            );
            return Ok(text.to_string());
        }

        let mut last_err = None;
        for attempt in 0..max_retries {
            let req = apply_request_headers(
                apply_referer_headers(self.client.get(url), referer),
                url,
                self.request_headers.as_ref(),
            )
            .header("User-Agent", self.effective_user_agent());
            match req.send().await {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        last_err =
                            Some(anyhow::anyhow!("HTTP {} fetching playlist", resp.status()));
                    } else {
                        match resp.text().await {
                            Ok(text) => return Ok(text),
                            Err(e) => last_err = Some(anyhow::anyhow!(e)),
                        }
                    }
                }
                Err(e) => last_err = Some(anyhow::anyhow!(e)),
            }
            if attempt < max_retries - 1 {
                let base = 500 * (attempt as u64 + 1);
                let jitter = rand::random::<u64>() % (base / 2 + 1);
                tokio::time::sleep(Duration::from_millis(base + jitter)).await;
            }
        }
        Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!("Failed to fetch m3u8 after {} attempts", max_retries)
        }))
    }

    #[allow(clippy::too_many_arguments)]
    async fn download_separate_tracks(
        &self,
        video_url: &str,
        audio_url: &str,
        output_path: &str,
        referer: &str,
        bytes_tx: Option<UnboundedSender<u64>>,
        cancel_token: CancellationToken,
        max_concurrent: u32,
        max_retries: u32,
    ) -> anyhow::Result<HlsDownloadResult> {
        if !crate::core::media_processor::check_ffmpeg() {
            anyhow::bail!("{FFMPEG_NEEDED_TO_JOIN}");
        }
        let video_tmp = format!("{output_path}.video.mp4");
        let audio_tmp = format!("{output_path}.audio.m4a");
        let joined = async {
            let video = self
                .download_media_playlist(
                    video_url,
                    &video_tmp,
                    referer,
                    bytes_tx.clone(),
                    cancel_token.clone(),
                    max_concurrent,
                    max_retries,
                )
                .await?;
            let audio = self
                .download_media_playlist(
                    audio_url,
                    &audio_tmp,
                    referer,
                    bytes_tx.clone(),
                    cancel_token.clone(),
                    max_concurrent,
                    max_retries,
                )
                .await?;
            let output = PathBuf::from(output_path);
            join_tracks(&video.path, &audio.path, &output).await?;
            let file_size = std::fs::metadata(&output)?.len();
            Ok(HlsDownloadResult {
                path: output,
                file_size,
                segments: video.segments + audio.segments,
            })
        }
        .await;
        let _ = std::fs::remove_file(&video_tmp);
        let _ = std::fs::remove_file(&audio_tmp);
        joined
    }

    #[allow(clippy::too_many_arguments)]
    async fn download_media_playlist(
        &self,
        m3u8_url: &str,
        output_path: &str,
        referer: &str,
        bytes_tx: Option<UnboundedSender<u64>>,
        cancel_token: CancellationToken,
        max_concurrent: u32,
        max_retries: u32,
    ) -> anyhow::Result<HlsDownloadResult> {
        let text = match self.prefetched_for(m3u8_url) {
            Some(text) => {
                tracing::info!(
                    "[hls] using prefetched media playlist text ({} bytes)",
                    text.len()
                );
                text.to_string()
            }
            None => {
                let resp = apply_request_headers(
                    apply_referer_headers(self.client.get(m3u8_url), referer),
                    m3u8_url,
                    self.request_headers.as_ref(),
                )
                .header("User-Agent", self.effective_user_agent())
                .send()
                .await?;

                if !resp.status().is_success() {
                    anyhow::bail!("HTTP {} fetching playlist", resp.status());
                }

                resp.text().await?
            }
        };

        let (_, mut playlist) = parse_media_playlist(text.as_bytes())
            .map_err(|e| anyhow::anyhow!("Parse media playlist: {:?}", e))?;
        if is_live(&playlist) {
            let dropped = take_live_edge(&mut playlist, LIVE_TAKE);
            tracing::info!(
                "[hls] live playlist: keeping the latest {} segments, {dropped} older ones skipped",
                playlist.segments.len()
            );
        }

        let total_segments = playlist.segments.len();

        let encryption = self
            .fetch_encryption_info(&playlist, m3u8_url, referer)
            .await?;

        let output = PathBuf::from(output_path);
        let part_path = {
            let mut p = output.as_os_str().to_owned();
            p.push(".part");
            PathBuf::from(p)
        };
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let (jobs, init_sections) = plan_segments(&playlist, m3u8_url);
        let user_agent = self.effective_user_agent().to_string();

        let mut inits = Vec::with_capacity(init_sections.len());
        for init in &init_sections {
            let data = download_segment_with_retry(
                &self.client,
                &init.url,
                init.range,
                referer,
                &user_agent,
                self.request_headers.as_ref(),
                max_retries.max(1),
                &cancel_token,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Init section (EXT-X-MAP) download failed: {}", e))?;
            inits.push(data);
        }
        let init_plan: Vec<Option<usize>> = jobs.iter().map(|j| j.init).collect();

        let (seg_tx, seg_rx) = mpsc::channel::<(usize, Vec<u8>)>(max_concurrent as usize);

        let writer_output = part_path.clone();
        let media_sequence = playlist.media_sequence;
        let writer = tokio::spawn(async move {
            write_segments_ordered(
                seg_rx,
                &writer_output,
                &encryption,
                media_sequence,
                total_segments,
                &InitSections {
                    data: inits,
                    plan: init_plan,
                },
            )
            .await
        });

        let semaphore = Arc::new(Semaphore::new(max_concurrent as usize));
        let completed = Arc::new(AtomicUsize::new(0));
        let downloaded_bytes = Arc::new(AtomicU64::new(0));
        let fail_token = cancel_token.child_token();
        let errors: Arc<tokio::sync::Mutex<HashMap<String, u32>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        let segment_jobs: Vec<(usize, SegmentJob)> = jobs.into_iter().enumerate().collect();

        let client = &self.client;
        let request_headers = self.request_headers.as_ref();
        let errors_ref = &errors;
        let completed_ref = &completed;
        let downloaded_ref = &downloaded_bytes;
        let fail_ref = &fail_token;
        let sem_ref = &semaphore;
        let user_agent_ref = &user_agent;
        let progress_ref = &self.progress_tx;

        stream::iter(segment_jobs)
            .map(|(i, job)| {
                let bytes_tx = bytes_tx.clone();
                let seg_tx = seg_tx.clone();
                let referer = referer.to_string();
                async move {
                    let _permit = sem_ref.acquire().await.unwrap();
                    if fail_ref.is_cancelled() {
                        return;
                    }
                    match download_segment_with_retry(
                        client,
                        &job.url,
                        job.range,
                        &referer,
                        user_agent_ref,
                        request_headers,
                        max_retries,
                        fail_ref,
                    )
                    .await
                    {
                        Ok(data) => {
                            if let Some(ref btx) = bytes_tx {
                                let _ = btx.send(data.len() as u64);
                            }
                            let done = completed_ref.fetch_add(1, Ordering::Relaxed) + 1;
                            let total_dl = downloaded_ref
                                .fetch_add(data.len() as u64, Ordering::Relaxed)
                                + data.len() as u64;
                            if let Some(ptx) = progress_ref {
                                let percent = if total_segments > 0 {
                                    (done as f64 / total_segments as f64) * 100.0
                                } else {
                                    0.0
                                };
                                let _ = ptx.try_send(ProgressUpdate::rich(
                                    percent,
                                    Some(total_dl),
                                    None,
                                    None,
                                    None,
                                ));
                            }
                            let _ = seg_tx.send((i, data)).await;
                        }
                        Err(e) => {
                            let key = e.to_string();
                            let mut errs = errors_ref.lock().await;
                            *errs.entry(key).or_insert(0) += 1;
                            drop(errs);
                            fail_ref.cancel();
                        }
                    }
                }
            })
            .buffer_unordered(max_concurrent as usize)
            .collect::<()>()
            .await;

        drop(seg_tx);

        let writer_result = writer
            .await
            .map_err(|e| anyhow::anyhow!("Writer task panicked: {:?}", e))?;

        if cancel_token.is_cancelled() {
            let _ = std::fs::remove_file(&part_path);
            anyhow::bail!("Download cancelled by user");
        }

        let errs = errors.lock().await;
        if !errs.is_empty() {
            let _ = std::fs::remove_file(&part_path);
            let summary: Vec<String> = errs
                .iter()
                .map(|(msg, count)| {
                    if *count > 1 {
                        format!("{} (x{})", msg, count)
                    } else {
                        msg.clone()
                    }
                })
                .collect();
            anyhow::bail!("Segment download failed: {}", summary.join("; "));
        }
        drop(errs);

        writer_result?;

        finalize_container(&part_path, &output).await?;

        let file_size = std::fs::metadata(&output)?.len();

        Ok(HlsDownloadResult {
            path: output,
            file_size,
            segments: total_segments,
        })
    }

    async fn fetch_encryption_info(
        &self,
        playlist: &m3u8_rs::MediaPlaylist,
        m3u8_url: &str,
        referer: &str,
    ) -> anyhow::Result<Option<EncryptionInfo>> {
        for segment in &playlist.segments {
            if let Some(key) = &segment.key {
                match key.method {
                    m3u8_rs::KeyMethod::AES128 => {
                        if let Some(uri) = &key.uri {
                            let key_url = resolve_url(m3u8_url, uri);
                            let key_bytes = self.fetch_key_with_retry(&key_url, referer, 3).await?;
                            let iv = key.iv.as_ref().map(|iv_str| parse_hex_iv(iv_str));
                            return Ok(Some(EncryptionInfo { key_bytes, iv }));
                        }
                    }
                    m3u8_rs::KeyMethod::SampleAES => {
                        anyhow::bail!("HLS stream uses SAMPLE-AES (FairPlay DRM), cannot decrypt");
                    }
                    _ => {}
                }
            }
        }
        Ok(None)
    }

    async fn fetch_key_with_retry(
        &self,
        url: &str,
        referer: &str,
        max_retries: u32,
    ) -> anyhow::Result<Vec<u8>> {
        let mut last_err = None;
        for attempt in 0..max_retries {
            let req = apply_request_headers(
                apply_referer_headers(self.client.get(url), referer),
                url,
                self.request_headers.as_ref(),
            )
            .header("User-Agent", self.effective_user_agent());
            match req.send().await {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        last_err = Some(anyhow::anyhow!("HTTP {} fetching AES key", resp.status()));
                    } else {
                        match resp.bytes().await {
                            Ok(bytes) => return Ok(bytes.to_vec()),
                            Err(e) => last_err = Some(anyhow::anyhow!(e)),
                        }
                    }
                }
                Err(e) => last_err = Some(anyhow::anyhow!(e)),
            }
            if attempt < max_retries - 1 {
                let base = 500 * (attempt as u64 + 1);
                let jitter = rand::random::<u64>() % (base / 2 + 1);
                tokio::time::sleep(Duration::from_millis(base + jitter)).await;
            }
        }
        Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!("Failed to fetch AES key after {} attempts", max_retries)
        }))
    }
}

struct EncryptionInfo {
    key_bytes: Vec<u8>,
    iv: Option<[u8; 16]>,
}

fn apply_request_headers(
    req: reqwest::RequestBuilder,
    url: &str,
    headers: Option<&RequestHeaders>,
) -> reqwest::RequestBuilder {
    match headers {
        Some(h) => req.headers(h(url)),
        None => req,
    }
}

fn apply_referer_headers(req: reqwest::RequestBuilder, referer: &str) -> reqwest::RequestBuilder {
    if referer.is_empty() {
        return req;
    }
    let req = req.header("Referer", referer);
    match url_origin(referer) {
        Some(origin) => req.header("Origin", origin),
        None => req,
    }
}

fn url_origin(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let scheme = parsed.scheme();
    Some(match parsed.port() {
        Some(port) => format!("{}://{}:{}", scheme, host, port),
        None => format!("{}://{}", scheme, host),
    })
}

fn variant_height(v: &VariantStream) -> u64 {
    v.resolution.as_ref().map(|r| r.height).unwrap_or(0)
}

fn select_best_variant(master: &MasterPlaylist, max_height: Option<u32>) -> Option<&VariantStream> {
    let mut sorted: Vec<&VariantStream> =
        master.variants.iter().filter(|v| !v.is_i_frame).collect();

    if sorted.is_empty() {
        return None;
    }

    sorted.sort_by_key(|v| (variant_height(v), v.bandwidth));

    let max_h = match max_height {
        Some(h) => h as u64,
        None => return sorted.last().copied(),
    };

    sorted
        .iter()
        .rev()
        .find(|v| {
            v.resolution
                .as_ref()
                .map(|r| r.height <= max_h)
                .unwrap_or(true)
        })
        .copied()
        .or_else(|| sorted.first().copied())
}

fn select_audio_rendition<'a>(
    master: &'a MasterPlaylist,
    variant: &VariantStream,
) -> Option<&'a AlternativeMedia> {
    let group = variant.audio.as_deref()?;
    let renditions: Vec<&AlternativeMedia> = master
        .alternatives
        .iter()
        .filter(|a| a.media_type == AlternativeMediaType::Audio && a.group_id == group)
        .collect();
    // WHY: Gumlet writes DEFAULT=YES inside CHARACTERISTICS, not as its own attribute.
    let is_default = |a: &&&AlternativeMedia| {
        a.default
            || a.characteristics
                .as_deref()
                .is_some_and(|c| c.to_ascii_uppercase().contains("DEFAULT=YES"))
    };
    renditions
        .iter()
        .find(is_default)
        .or_else(|| renditions.iter().find(|a| a.autoselect))
        .or_else(|| renditions.first())
        .copied()
        .filter(|a| a.uri.is_some())
}

pub const FFMPEG_NEEDED_TO_JOIN: &str =
    "This video keeps its sound in a separate track, and FFmpeg is needed to join them. Install FFmpeg and try again.";

pub async fn join_tracks(video: &Path, audio: &Path, output: &Path) -> anyhow::Result<()> {
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();
    let joining = output.with_extension(format!("joining.{ext}"));
    let mut cmd = crate::core::process::command("ffmpeg");
    cmd.args(["-y", "-i"])
        .arg(video)
        .arg("-i")
        .arg(audio)
        .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"]);
    if matches!(ext.as_str(), "mp4" | "m4v" | "mov") {
        cmd.args(["-movflags", "+faststart"]);
    }
    let result = cmd
        .arg(&joining)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&joining);
        let stderr = String::from_utf8_lossy(&result.stderr);
        let reason = stderr.lines().last().unwrap_or("no details");
        anyhow::bail!("could not join the video and audio tracks: {reason}");
    }
    std::fs::rename(&joining, output)?;
    Ok(())
}

fn resolve_url(base: &str, relative: &str) -> String {
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.to_string();
    }
    if let Ok(mut u) = url::Url::parse(base).and_then(|b| b.join(relative)) {
        if !relative.contains('?') {
            if let Some((_, q)) = base.split_once('?') {
                u.set_query(Some(q.split('#').next().unwrap_or(q)));
            }
        }
        return u.to_string();
    }

    let (base_path, query) = match base.find('?') {
        Some(pos) => (&base[..pos], Some(&base[pos..])),
        None => (base, None),
    };

    let resolved = if let Some(pos) = base_path.rfind('/') {
        format!("{}/{}", &base_path[..pos], relative)
    } else {
        relative.to_string()
    };

    match query {
        Some(q) if !relative.contains('?') => format!("{}{}", resolved, q),
        _ => resolved,
    }
}

async fn finalize_container(part_path: &Path, output: &Path) -> anyhow::Result<()> {
    let wants_mp4 = matches!(
        output
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("mp4" | "m4v" | "m4a" | "mov")
    );

    if wants_mp4 && crate::core::media_processor::check_ffmpeg() {
        let part_str = part_path.to_string_lossy();
        let out_str = output.to_string_lossy();
        let status = crate::core::process::command("ffmpeg")
            .args([
                "-y",
                "-fflags",
                "+genpts",
                "-i",
                part_str.as_ref(),
                "-c",
                "copy",
                "-movflags",
                "+faststart",
                out_str.as_ref(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;

        match status {
            Ok(s) if s.success() => {
                let _ = std::fs::remove_file(part_path);
                return Ok(());
            }
            Ok(s) => {
                tracing::warn!(
                    "HLS remux to MP4 failed with status {}, keeping raw transport stream",
                    s
                );
                let _ = std::fs::remove_file(output);
            }
            Err(e) => {
                tracing::warn!("HLS remux to MP4 failed to spawn ffmpeg: {}", e);
            }
        }
    } else if wants_mp4 {
        tracing::warn!("ffmpeg not found; HLS output will remain MPEG-TS despite .mp4 extension");
    }

    std::fs::rename(part_path, output)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
struct SegmentJob {
    url: String,
    range: Option<(u64, u64)>,
    init: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
struct InitSection {
    url: String,
    range: Option<(u64, u64)>,
}

struct InitSections {
    data: Vec<Vec<u8>>,
    plan: Vec<Option<usize>>,
}

pub const LIVE_TAKE: usize = 16;

pub fn is_live(p: &m3u8_rs::MediaPlaylist) -> bool {
    !p.end_list && p.playlist_type != Some(m3u8_rs::MediaPlaylistType::Vod)
}

fn take_live_edge(p: &mut m3u8_rs::MediaPlaylist, keep: usize) -> usize {
    let n = p.segments.len();
    if n <= keep {
        return 0;
    }
    let drop = n - keep;
    let (mut map, mut key) = (None, None);
    for s in &p.segments[..drop] {
        if s.map.is_some() {
            map = s.map.clone();
        }
        if s.key.is_some() {
            key = s.key.clone();
        }
    }
    p.segments.drain(..drop);
    if let Some(first) = p.segments.first_mut() {
        if first.map.is_none() {
            first.map = map;
        }
        if first.key.is_none() {
            first.key = key;
        }
    }
    p.media_sequence += drop as u64;
    drop
}

fn plan_segments(
    playlist: &m3u8_rs::MediaPlaylist,
    base: &str,
) -> (Vec<SegmentJob>, Vec<InitSection>) {
    let mut jobs = Vec::with_capacity(playlist.segments.len());
    let mut inits: Vec<InitSection> = Vec::new();
    let mut current_init = None;
    let mut next_offset: HashMap<String, u64> = HashMap::new();
    for seg in &playlist.segments {
        if let Some(map) = &seg.map {
            let init = InitSection {
                url: resolve_url(base, &map.uri),
                range: map
                    .byte_range
                    .as_ref()
                    .map(|b| (b.offset.unwrap_or(0), b.length)),
            };
            current_init = Some(match inits.iter().position(|i| *i == init) {
                Some(p) => p,
                None => {
                    inits.push(init);
                    inits.len() - 1
                }
            });
        }
        let url = resolve_url(base, &seg.uri);
        let range = seg.byte_range.as_ref().map(|b| {
            let start = b
                .offset
                .unwrap_or_else(|| next_offset.get(&url).copied().unwrap_or(0));
            next_offset.insert(url.clone(), start + b.length);
            (start, b.length)
        });
        jobs.push(SegmentJob {
            url,
            range,
            init: current_init,
        });
    }
    (jobs, inits)
}

async fn write_segments_ordered(
    mut rx: mpsc::Receiver<(usize, Vec<u8>)>,
    output_path: &PathBuf,
    encryption: &Option<EncryptionInfo>,
    media_sequence: u64,
    total_segments: usize,
    init_sections: &InitSections,
) -> anyhow::Result<()> {
    use std::io::Write;
    let mut file =
        std::io::BufWriter::with_capacity(256 * 1024, std::fs::File::create(output_path)?);
    let mut next_expected: usize = 0;
    let mut pending: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    let mut written_init: Option<usize> = None;

    while let Some((idx, data)) = rx.recv().await {
        pending.insert(idx, data);

        while let Some(segment_data) = pending.remove(&next_expected) {
            if let Some(Some(init)) = init_sections.plan.get(next_expected) {
                if written_init != Some(*init) {
                    if let Some(bytes) = init_sections.data.get(*init) {
                        file.write_all(bytes)?;
                    }
                    written_init = Some(*init);
                }
            }
            let payload_start = image_wrapper_offset(&segment_data);

            if let Some(enc) = encryption {
                use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
                type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

                let iv = compute_iv(enc, next_expected, media_sequence);
                let mut buf = segment_data;
                if payload_start > 0 {
                    buf.drain(..payload_start);
                }
                let decryptor = Aes128CbcDec::new_from_slices(&enc.key_bytes, &iv)
                    .map_err(|e| anyhow::anyhow!("AES init: {:?}", e))?;
                let decrypted = decryptor
                    .decrypt_padded_mut::<Pkcs7>(&mut buf)
                    .map_err(|e| anyhow::anyhow!("AES decrypt: {:?}", e))?;
                file.write_all(decrypted)?;
            } else {
                file.write_all(&segment_data[payload_start..])?;
            }
            next_expected += 1;
        }
    }

    file.flush()?;

    if next_expected < total_segments {
        anyhow::bail!(
            "Only {} of {} segments were written",
            next_expected,
            total_segments
        );
    }

    Ok(())
}

const SEGMENT_STALL: Duration = Duration::from_secs(10);

#[allow(clippy::too_many_arguments)]
async fn download_segment_with_retry(
    client: &Client,
    url: &str,
    range: Option<(u64, u64)>,
    referer: &str,
    user_agent: &str,
    request_headers: Option<&RequestHeaders>,
    max_retries: u32,
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<u8>> {
    download_segment_watched(
        client,
        url,
        range,
        referer,
        user_agent,
        request_headers,
        max_retries,
        cancel,
        SEGMENT_STALL,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn download_segment_watched(
    client: &Client,
    url: &str,
    range: Option<(u64, u64)>,
    referer: &str,
    user_agent: &str,
    request_headers: Option<&RequestHeaders>,
    max_retries: u32,
    cancel: &CancellationToken,
    stall: Duration,
) -> anyhow::Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut resumable = false;
    let mut last_err = None;
    for attempt in 0..max_retries {
        if cancel.is_cancelled() {
            anyhow::bail!("Download cancelled");
        }
        if !resumable {
            buf.clear();
        }

        match fetch_segment_once(
            client,
            url,
            range,
            referer,
            user_agent,
            request_headers,
            stall,
            &mut buf,
            &mut resumable,
        )
        .await
        {
            Ok(()) => return Ok(buf),
            Err(e) => {
                if e.to_string().contains("(fatal)") {
                    return Err(e);
                }
                tracing::debug!(
                    "[hls] segment attempt {} failed after {} bytes: {}",
                    attempt + 1,
                    buf.len(),
                    e
                );
                last_err = Some(e);
            }
        }
        if attempt < max_retries - 1 {
            let base = 500 * (attempt as u64 + 1);
            let jitter = rand::random::<u64>() % (base / 2 + 1);
            tokio::time::sleep(std::time::Duration::from_millis(base + jitter)).await;
        }
    }
    Err(last_err.unwrap_or_else(|| {
        anyhow::anyhow!("Segment download failed after {} attempts", max_retries)
    }))
}

#[allow(clippy::too_many_arguments)]
async fn fetch_segment_once(
    client: &Client,
    url: &str,
    range: Option<(u64, u64)>,
    referer: &str,
    user_agent: &str,
    request_headers: Option<&RequestHeaders>,
    stall: Duration,
    buf: &mut Vec<u8>,
    resumable: &mut bool,
) -> anyhow::Result<()> {
    let got = buf.len() as u64;
    let start = range.map(|(s, _)| s).unwrap_or(0) + got;
    let range_header = match range {
        Some((s, len)) => Some(format!("bytes={}-{}", s + got, s + len - 1)),
        None if got > 0 => Some(format!("bytes={}-", got)),
        None => None,
    };

    let mut req = apply_request_headers(
        apply_referer_headers(client.get(url), referer),
        url,
        request_headers,
    )
    .header("User-Agent", user_agent)
    .header(reqwest::header::ACCEPT_ENCODING, "identity");
    if let Some(r) = &range_header {
        req = req.header(reqwest::header::RANGE, r.as_str());
    }
    let resp = tokio::time::timeout(stall, req.send())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for segment response"))??;

    let status = resp.status();
    if !status.is_success() {
        let code = status.as_u16();
        if (400..500).contains(&code) && code != 429 && code != 408 && code != 416 {
            return Err(anyhow::anyhow!("HTTP {} (fatal) downloading segment", code));
        }
        *resumable = false;
        return Err(anyhow::anyhow!("HTTP {} downloading segment", code));
    }

    let whole = status != reqwest::StatusCode::PARTIAL_CONTENT;
    if whole {
        buf.clear();
        *resumable = range.is_none()
            && resp
                .headers()
                .get(reqwest::header::ACCEPT_RANGES)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.contains("bytes"));
    } else {
        let answered = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().strip_prefix("bytes"))
            .and_then(|v| v.trim_start().split('-').next())
            .and_then(|v| v.trim().parse::<u64>().ok());
        if answered.is_some_and(|a| a != start) {
            buf.clear();
            *resumable = false;
            return Err(anyhow::anyhow!(
                "server answered a different range for the segment"
            ));
        }
        *resumable = true;
    }
    let expected = resp.content_length().map(|n| n + buf.len() as u64);

    let mut stream = resp.bytes_stream();
    loop {
        match tokio::time::timeout(stall, stream.next()).await {
            Ok(Some(Ok(chunk))) => buf.extend_from_slice(&chunk),
            Ok(Some(Err(e))) => return Err(anyhow::anyhow!("Segment stream error: {}", e)),
            Ok(None) => break,
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Segment stalled: no data for {} s",
                    stall.as_secs()
                ))
            }
        }
    }
    if expected.is_some_and(|e| (buf.len() as u64) < e) {
        return Err(anyhow::anyhow!("Segment ended early"));
    }

    if let (true, Some((s, len))) = (whole, range) {
        let (s, e) = (s as usize, (s + len) as usize);
        if buf.len() < e {
            buf.clear();
            anyhow::bail!("Segment shorter than its BYTERANGE");
        }
        let slice = buf[s..e].to_vec();
        *buf = slice;
        *resumable = false;
    }
    Ok(())
}

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const JPEG_SIGNATURE: [u8; 3] = [0xFF, 0xD8, 0xFF];

fn png_payload_offset(data: &[u8]) -> Option<usize> {
    let mut at = PNG_SIGNATURE.len();
    loop {
        let header_end = at.checked_add(8)?;
        if header_end > data.len() {
            return None;
        }
        let length = u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
        let kind = &data[at + 4..at + 8];
        let next = header_end.checked_add(length as usize)?.checked_add(4)?;
        if next > data.len() {
            return None;
        }
        if kind == b"IEND" {
            return Some(next);
        }
        at = next;
    }
}

fn jpeg_payload_offset(data: &[u8]) -> Option<usize> {
    find_subslice(&data[JPEG_SIGNATURE.len()..], &[0xFF, 0xD9])
        .map(|pos| JPEG_SIGNATURE.len() + pos + 2)
}

fn looks_like_jpeg(data: &[u8]) -> bool {
    if !data.starts_with(&JPEG_SIGNATURE) || data.len() < 4 {
        return false;
    }
    matches!(data[3], 0xC0..=0xCF | 0xDB | 0xDD | 0xE0..=0xEF | 0xFE)
}

fn image_wrapper_offset(data: &[u8]) -> usize {
    let start = if data.starts_with(&PNG_SIGNATURE) {
        match png_payload_offset(data) {
            Some(end) => end,
            None => return 0,
        }
    } else if looks_like_jpeg(data) {
        match jpeg_payload_offset(data) {
            Some(end) => end,
            None => return 0,
        }
    } else {
        return 0;
    };

    if start >= data.len() {
        return 0;
    }
    start
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn compute_iv(encryption: &EncryptionInfo, segment_index: usize, media_sequence: u64) -> [u8; 16] {
    if let Some(iv) = &encryption.iv {
        return *iv;
    }
    let seq = media_sequence + segment_index as u64;
    let mut iv = [0u8; 16];
    iv[8..16].copy_from_slice(&seq.to_be_bytes());
    iv
}

fn parse_hex_iv(iv_str: &str) -> [u8; 16] {
    let hex = iv_str.trim_start_matches("0x").trim_start_matches("0X");
    let mut result = [0u8; 16];
    let padded = format!("{:0>32}", hex);
    for i in 0..16 {
        result[i] = u8::from_str_radix(&padded[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use m3u8_rs::{MasterPlaylist, Resolution, VariantStream};

    #[test]
    fn url_origin_basic() {
        assert_eq!(
            url_origin("https://cdn.example.com/path/master.m3u8?token=abc").as_deref(),
            Some("https://cdn.example.com")
        );
    }

    #[test]
    fn url_origin_with_port() {
        assert_eq!(
            url_origin("http://cdn.example.com:8080/video/seg.ts").as_deref(),
            Some("http://cdn.example.com:8080")
        );
    }

    #[test]
    fn url_origin_invalid_returns_none() {
        assert_eq!(url_origin("not a url"), None);
        assert_eq!(url_origin(""), None);
    }

    #[test]
    fn resolve_url_absolute_passthrough() {
        assert_eq!(
            resolve_url(
                "https://cdn.example.com/path/master.m3u8",
                "https://other.com/video.ts"
            ),
            "https://other.com/video.ts"
        );
    }

    #[test]
    fn resolve_url_relative() {
        assert_eq!(
            resolve_url("https://cdn.example.com/path/master.m3u8", "segment0.ts"),
            "https://cdn.example.com/path/segment0.ts"
        );
    }

    #[test]
    fn resolve_url_propagates_query() {
        assert_eq!(
            resolve_url(
                "https://cdn.example.com/path/master.m3u8?token=abc",
                "segment0.ts"
            ),
            "https://cdn.example.com/path/segment0.ts?token=abc"
        );
    }

    #[test]
    fn resolve_url_relative_with_own_query_skips_base_query() {
        assert_eq!(
            resolve_url(
                "https://cdn.example.com/path/master.m3u8?token=abc",
                "segment0.ts?key=123"
            ),
            "https://cdn.example.com/path/segment0.ts?key=123"
        );
    }

    #[test]
    fn resolve_url_root_and_protocol_relative() {
        let base = "https://embed-cloudfront.wistia.com/deliveries/abc.m3u8";
        assert_eq!(
            resolve_url(base, "/deliveries/abc.m3u8/seg-1-v1-a1.ts"),
            "https://embed-cloudfront.wistia.com/deliveries/abc.m3u8/seg-1-v1-a1.ts"
        );
        assert_eq!(
            resolve_url(base, "//cdn.example.com/s/1.ts"),
            "https://cdn.example.com/s/1.ts"
        );
        assert_eq!(
            resolve_url("https://a.com/v/x/index.m3u8?t=1", "../seg.ts"),
            "https://a.com/v/seg.ts?t=1"
        );
    }

    #[test]
    fn live_window_keeps_the_edge() {
        let mut text = String::from(
            "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:100\n#EXT-X-MAP:URI=\"init.mp4\"\n",
        );
        for i in 0..40 {
            text.push_str(&format!("#EXTINF:6.0,\nseg{i}.m4s\n"));
        }
        let (_, mut p) = parse_media_playlist(text.as_bytes()).unwrap();
        assert!(is_live(&p));
        assert_eq!(take_live_edge(&mut p, LIVE_TAKE), 24);
        assert_eq!(p.segments.len(), LIVE_TAKE);
        assert_eq!(p.segments[0].uri, "seg24.m4s");
        assert_eq!(p.media_sequence, 124);
        assert_eq!(p.segments[0].map.as_ref().unwrap().uri, "init.mp4");

        text.push_str("#EXT-X-ENDLIST\n");
        let (_, vod) = parse_media_playlist(text.as_bytes()).unwrap();
        assert!(!is_live(&vod));
    }

    #[test]
    fn resolve_url_no_slash_in_base() {
        assert_eq!(resolve_url("master.m3u8", "segment0.ts"), "segment0.ts");
    }

    #[test]
    fn select_best_variant_picks_720() {
        let master = MasterPlaylist {
            variants: vec![
                VariantStream {
                    uri: "360.m3u8".into(),
                    bandwidth: 800_000,
                    resolution: Some(Resolution {
                        width: 640,
                        height: 360,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "720.m3u8".into(),
                    bandwidth: 2_500_000,
                    resolution: Some(Resolution {
                        width: 1280,
                        height: 720,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "1080.m3u8".into(),
                    bandwidth: 5_000_000,
                    resolution: Some(Resolution {
                        width: 1920,
                        height: 1080,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let best = select_best_variant(&master, Some(720)).unwrap();
        assert_eq!(best.uri, "720.m3u8");
    }

    #[test]
    fn select_best_variant_picks_1080() {
        let master = MasterPlaylist {
            variants: vec![
                VariantStream {
                    uri: "720.m3u8".into(),
                    bandwidth: 2_500_000,
                    resolution: Some(Resolution {
                        width: 1280,
                        height: 720,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "1080.m3u8".into(),
                    bandwidth: 5_000_000,
                    resolution: Some(Resolution {
                        width: 1920,
                        height: 1080,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let best = select_best_variant(&master, Some(1080)).unwrap();
        assert_eq!(best.uri, "1080.m3u8");
    }

    #[test]
    fn select_best_variant_empty_returns_none() {
        let master = MasterPlaylist {
            variants: vec![],
            ..Default::default()
        };
        assert!(select_best_variant(&master, Some(720)).is_none());
    }

    #[test]
    fn select_best_variant_skips_iframe() {
        let master = MasterPlaylist {
            variants: vec![
                VariantStream {
                    uri: "iframe.m3u8".into(),
                    bandwidth: 100_000,
                    is_i_frame: true,
                    resolution: Some(Resolution {
                        width: 320,
                        height: 180,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "720.m3u8".into(),
                    bandwidth: 2_500_000,
                    resolution: Some(Resolution {
                        width: 1280,
                        height: 720,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let best = select_best_variant(&master, Some(720)).unwrap();
        assert_eq!(best.uri, "720.m3u8");
    }

    #[test]
    fn select_best_variant_fallback_to_lowest_when_all_exceed() {
        let master = MasterPlaylist {
            variants: vec![
                VariantStream {
                    uri: "1080.m3u8".into(),
                    bandwidth: 5_000_000,
                    resolution: Some(Resolution {
                        width: 1920,
                        height: 1080,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "4k.m3u8".into(),
                    bandwidth: 15_000_000,
                    resolution: Some(Resolution {
                        width: 3840,
                        height: 2160,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let best = select_best_variant(&master, Some(360)).unwrap();
        assert_eq!(best.uri, "1080.m3u8");
    }

    #[test]
    fn select_best_variant_no_resolution_treated_as_eligible() {
        let master = MasterPlaylist {
            variants: vec![VariantStream {
                uri: "audio.m3u8".into(),
                bandwidth: 128_000,
                resolution: None,
                ..Default::default()
            }],
            ..Default::default()
        };
        let best = select_best_variant(&master, Some(720)).unwrap();
        assert_eq!(best.uri, "audio.m3u8");
    }

    fn ladder() -> MasterPlaylist {
        MasterPlaylist {
            variants: vec![
                VariantStream {
                    uri: "720.m3u8".into(),
                    bandwidth: 2_500_000,
                    resolution: Some(Resolution {
                        width: 1280,
                        height: 720,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "1080.m3u8".into(),
                    bandwidth: 5_000_000,
                    resolution: Some(Resolution {
                        width: 1920,
                        height: 1080,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "360.m3u8".into(),
                    bandwidth: 800_000,
                    resolution: Some(Resolution {
                        width: 640,
                        height: 360,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn select_best_variant_no_target_picks_highest() {
        let master = ladder();
        let best = select_best_variant(&master, None).unwrap();
        assert_eq!(best.uri, "1080.m3u8");
    }

    #[test]
    fn select_best_variant_target_above_ladder_picks_highest() {
        let master = ladder();
        let best = select_best_variant(&master, Some(2160)).unwrap();
        assert_eq!(best.uri, "1080.m3u8");
    }

    #[test]
    fn select_best_variant_target_between_rungs_rounds_down() {
        let master = ladder();
        let best = select_best_variant(&master, Some(900)).unwrap();
        assert_eq!(best.uri, "720.m3u8");
    }

    #[test]
    fn select_best_variant_same_height_prefers_higher_bandwidth() {
        let master = MasterPlaylist {
            variants: vec![
                VariantStream {
                    uri: "1080-low.m3u8".into(),
                    bandwidth: 3_000_000,
                    resolution: Some(Resolution {
                        width: 1920,
                        height: 1080,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "1080-high.m3u8".into(),
                    bandwidth: 6_000_000,
                    resolution: Some(Resolution {
                        width: 1920,
                        height: 1080,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let best = select_best_variant(&master, None).unwrap();
        assert_eq!(best.uri, "1080-high.m3u8");
    }

    #[test]
    fn parse_hex_iv_full_32_chars() {
        let iv = parse_hex_iv("0x00000000000000000000000000000001");
        let mut expected = [0u8; 16];
        expected[15] = 1;
        assert_eq!(iv, expected);
    }

    #[test]
    fn parse_hex_iv_short_padded() {
        let iv = parse_hex_iv("0xFF");
        let mut expected = [0u8; 16];
        expected[15] = 0xFF;
        assert_eq!(iv, expected);
    }

    #[test]
    fn parse_hex_iv_uppercase_prefix() {
        let iv = parse_hex_iv("0X0A0B0C0D0E0F10111213141516171819");
        assert_eq!(iv[0], 0x0A);
        assert_eq!(iv[7], 0x11);
        assert_eq!(iv[15], 0x19);
    }

    #[test]
    fn parse_hex_iv_no_prefix() {
        let iv = parse_hex_iv("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");
        assert_eq!(iv, [0xFF; 16]);
    }

    #[test]
    fn compute_iv_returns_explicit_when_present() {
        let explicit_iv = [0xAB; 16];
        let enc = EncryptionInfo {
            key_bytes: vec![0u8; 16],
            iv: Some(explicit_iv),
        };
        assert_eq!(compute_iv(&enc, 5, 100), explicit_iv);
    }

    #[test]
    fn compute_iv_derives_from_sequence() {
        let enc = EncryptionInfo {
            key_bytes: vec![0u8; 16],
            iv: None,
        };
        let result = compute_iv(&enc, 3, 100);
        let mut expected = [0u8; 16];
        expected[8..16].copy_from_slice(&103u64.to_be_bytes());
        assert_eq!(result, expected);
    }

    #[test]
    fn compute_iv_sequence_zero() {
        let enc = EncryptionInfo {
            key_bytes: vec![0u8; 16],
            iv: None,
        };
        let result = compute_iv(&enc, 0, 0);
        assert_eq!(result, [0u8; 16]);
    }

    fn stripped(data: &[u8]) -> &[u8] {
        &data[image_wrapper_offset(data)..]
    }

    fn png_wrapped(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&PNG_SIGNATURE);
        out.extend_from_slice(&[0, 0, 0, 4]);
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&[1, 2, 3, 4]);
        out.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(b"IEND");
        out.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);
        out.extend_from_slice(payload);
        out
    }

    fn jpeg_wrapped(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&JPEG_SIGNATURE);
        out.extend_from_slice(&[0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46]);
        out.extend_from_slice(&[0xFF, 0xD9]);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn image_wrapper_offset_strips_png_prefix() {
        let payload = b"\x47\x40\x11\x10 transport stream";
        let wrapped = png_wrapped(payload);
        assert_eq!(stripped(&wrapped), payload);
    }

    #[test]
    fn image_wrapper_offset_strips_jpeg_prefix() {
        let payload = b"\x47\x40\x11\x10 transport stream";
        let wrapped = jpeg_wrapped(payload);
        assert_eq!(stripped(&wrapped), payload);
    }

    fn png_with_iend_bytes_inside_idat(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&PNG_SIGNATURE);
        let idat = b"....IEND....decoy";
        out.extend_from_slice(&(idat.len() as u32).to_be_bytes());
        out.extend_from_slice(b"IDAT");
        out.extend_from_slice(idat);
        out.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(b"IEND");
        out.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn iend_bytes_inside_the_image_do_not_cut_the_segment_short() {
        let payload = b"\x47\x40\x11\x10 transport stream";
        let wrapped = png_with_iend_bytes_inside_idat(payload);
        assert_eq!(stripped(&wrapped), payload);
    }

    #[test]
    fn three_unlucky_bytes_are_not_a_jpeg() {
        let mut ciphertext = vec![0xFF, 0xD8, 0xFF, 0x1A];
        ciphertext.extend_from_slice(b"encrypted segment bytes\xFF\xD9 and more");
        assert_eq!(image_wrapper_offset(&ciphertext), 0);
        assert_eq!(stripped(&ciphertext), &ciphertext[..]);

        let payload = b"\x47\x40\x11\x10 transport stream";
        assert_eq!(stripped(&jpeg_wrapped(payload)), payload);
    }

    #[test]
    fn image_wrapper_offset_is_zero_without_a_wrapper() {
        let raw = b"\x47\x40\x11\x10 plain segment bytes";
        assert_eq!(image_wrapper_offset(raw), 0);
        assert_eq!(stripped(raw), raw);
    }

    #[test]
    fn png_without_iend_is_left_intact() {
        let mut data = Vec::new();
        data.extend_from_slice(&PNG_SIGNATURE);
        data.extend_from_slice(b"truncated png with no end chunk");
        assert_eq!(image_wrapper_offset(&data), 0);
        assert_eq!(stripped(&data), &data[..]);
    }

    #[test]
    fn jpeg_without_eoi_is_left_intact() {
        let mut data = Vec::new();
        data.extend_from_slice(&JPEG_SIGNATURE);
        data.extend_from_slice(b"truncated jpeg with no end marker");
        assert_eq!(image_wrapper_offset(&data), 0);
        assert_eq!(stripped(&data), &data[..]);
    }

    #[test]
    fn wrapper_with_no_payload_behind_it_is_left_intact() {
        let png = png_wrapped(b"");
        assert_eq!(image_wrapper_offset(&png), 0);
        let jpeg = jpeg_wrapped(b"");
        assert_eq!(image_wrapper_offset(&jpeg), 0);
    }

    #[test]
    fn short_and_empty_buffers_are_left_intact() {
        assert_eq!(image_wrapper_offset(&[]), 0);
        assert_eq!(image_wrapper_offset(&[0x89, 0x50]), 0);
        assert_eq!(image_wrapper_offset(&[0xFF, 0xD8]), 0);
    }

    #[test]
    fn find_subslice_basics() {
        assert_eq!(find_subslice(b"abcdef", b"cd"), Some(2));
        assert_eq!(find_subslice(b"abcdef", b"xy"), None);
        assert_eq!(find_subslice(b"ab", b"abc"), None);
        assert_eq!(find_subslice(b"abc", b""), None);
    }

    #[test]
    fn prefetched_for_matches_only_the_stored_url() {
        let url = "https://cdn.example.com/live/master.m3u8";
        let downloader = HlsDownloader::with_client(Client::new())
            .with_prefetched_playlist(url.to_string(), "#EXTM3U\n".to_string());

        assert_eq!(downloader.prefetched_for(url), Some("#EXTM3U\n"));
        assert_eq!(
            downloader.prefetched_for("https://cdn.example.com/live/1080.m3u8"),
            None
        );
        assert_eq!(
            downloader.prefetched_for("https://cdn.example.com/live/master.m3u8?t=1"),
            None
        );
    }

    #[test]
    fn prefetched_for_is_none_without_a_prefetched_playlist() {
        let downloader = HlsDownloader::with_client(Client::new());
        assert_eq!(
            downloader.prefetched_for("https://cdn.example.com/live/master.m3u8"),
            None
        );
    }

    use crate::core::http_fetcher::test_server;

    #[test]
    fn plan_carries_ext_x_map_forward_and_accumulates_byteranges() {
        let text = b"#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:4
#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"720@0\"
#EXTINF:4.0,
#EXT-X-BYTERANGE:1000@720
media.mp4
#EXTINF:4.0,
#EXT-X-BYTERANGE:500
media.mp4
#EXT-X-DISCONTINUITY
#EXT-X-MAP:URI=\"init2.mp4\"
#EXTINF:4.0,
seg3.m4s
#EXTINF:4.0,
seg4.m4s
#EXT-X-ENDLIST
";
        let (_, pl) = parse_media_playlist(text).unwrap();
        let (jobs, inits) = plan_segments(&pl, "https://cdn.test/v/index.m3u8");
        assert_eq!(
            inits,
            vec![
                InitSection {
                    url: "https://cdn.test/v/init.mp4".into(),
                    range: Some((0, 720))
                },
                InitSection {
                    url: "https://cdn.test/v/init2.mp4".into(),
                    range: None
                },
            ]
        );
        assert_eq!(jobs.len(), 4);
        assert_eq!(jobs[0].range, Some((720, 1000)));
        assert_eq!(jobs[1].range, Some((1720, 500)), "no @offset continues");
        assert_eq!(jobs[1].init, Some(0), "MAP applies until the next one");
        assert_eq!(jobs[2].init, Some(1));
        assert_eq!(jobs[3].init, Some(1));
        assert_eq!(jobs[3].range, None);
    }

    #[tokio::test]
    async fn init_section_is_written_once_before_its_segments() {
        let dir = std::env::temp_dir().join(format!(
            "omniget-hls-init-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("o.part");
        let (tx, rx) = mpsc::channel(8);
        for (i, d) in [b"S0".to_vec(), b"S1".to_vec(), b"S2".to_vec()]
            .into_iter()
            .enumerate()
            .rev()
        {
            tx.send((i, d)).await.unwrap();
        }
        drop(tx);
        let inits = InitSections {
            data: vec![b"IA".to_vec(), b"IB".to_vec()],
            plan: vec![Some(0), Some(0), Some(1)],
        };
        write_segments_ordered(rx, &out, &None, 0, 3, &inits)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"IAS0S1IBS2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn byterange_segment_requests_only_its_slice() {
        let body: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        let b = body.clone();
        let (base, log) = test_server::spawn(move |_, r| test_server::serve_bytes(&b, r)).await;
        let data = download_segment_watched(
            &Client::new(),
            &format!("{base}/media.mp4"),
            Some((100, 50)),
            "",
            "ua",
            None,
            3,
            &CancellationToken::new(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(data, body[100..150].to_vec());
        let reqs = log.lock().unwrap().clone();
        assert_eq!(reqs[0].header("range"), Some("bytes=100-149"));
        assert_eq!(reqs[0].header("accept-encoding"), Some("identity"));
    }

    #[tokio::test]
    async fn stalled_segment_is_retried_from_the_received_byte() {
        let body: Vec<u8> = (0u8..=255).cycle().take(64 * 1024).collect();
        let b = body.clone();
        let (base, log) = test_server::spawn(move |_, r| {
            let mut reply = test_server::serve_bytes(&b, r);
            if r.range().is_none() {
                reply.cut_after = Some(b.len() / 2);
                reply.stall = true;
            }
            reply
        })
        .await;
        let t0 = std::time::Instant::now();
        let data = download_segment_watched(
            &Client::new(),
            &format!("{base}/seg.ts"),
            None,
            "",
            "ua",
            None,
            3,
            &CancellationToken::new(),
            Duration::from_millis(300),
        )
        .await
        .unwrap();
        assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
        assert_eq!(data, body);
        let reqs = log.lock().unwrap().clone();
        assert_eq!(reqs.len(), 2, "{reqs:?}");
        assert_eq!(reqs[1].range(), Some(((body.len() / 2) as u64, None)));
    }

    const GUMLET_SHAPED_MASTER: &[u8] = br#"#EXTM3U
#EXT-X-INDEPENDENT-SEGMENTS
#EXT-X-MEDIA:TYPE=AUDIO,URI="a_0_en_128k.m3u8",GROUP-ID="audio_128k",LANGUAGE="en",NAME="English",AUTOSELECT=YES,CHARACTERISTICS="DEFAULT=YES,AUTOSELECT=YES",CHANNELS="6"
#EXT-X-MEDIA:TYPE=AUDIO,URI="a_1_tr_128k.m3u8",GROUP-ID="audio_128k",LANGUAGE="tr",NAME="Turkish",AUTOSELECT=YES,CHARACTERISTICS="DEFAULT=NO,AUTOSELECT=YES",CHANNELS="6"
#EXT-X-MEDIA:TYPE=AUDIO,URI="a_1_tr_96k.m3u8",GROUP-ID="audio_96k",LANGUAGE="tr",NAME="Turkish",AUTOSELECT=YES,CHARACTERISTICS="DEFAULT=NO,AUTOSELECT=YES",CHANNELS="6"
#EXT-X-MEDIA:TYPE=SUBTITLES,URI="s_0_en.m3u8",GROUP-ID="subs",LANGUAGE="en",NAME="English",AUTOSELECT=YES
#EXT-X-STREAM-INF:BANDWIDTH=3294249,CODECS="avc1.4d401f,mp4a.40.2",RESOLUTION=1280x720,AUDIO="audio_128k",SUBTITLES="subs"
v_0_720p.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=3265349,CODECS="avc1.4d401f,mp4a.40.2",RESOLUTION=1280x720,AUDIO="audio_96k",SUBTITLES="subs"
v_0_720p.m3u8
"#;

    #[test]
    fn gumlet_master_picks_the_default_audio_written_in_characteristics() {
        let (_, master) = parse_master_playlist(GUMLET_SHAPED_MASTER).unwrap();
        let best = select_best_variant(&master, None).unwrap();
        assert_eq!(best.audio.as_deref(), Some("audio_128k"));
        let audio = select_audio_rendition(&master, best).unwrap();
        assert_eq!(audio.uri.as_deref(), Some("a_0_en_128k.m3u8"));
    }

    #[test]
    fn audio_already_inside_the_video_is_not_fetched_again() {
        let text = br#"#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aac",NAME="Main",DEFAULT=YES
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aac",NAME="Dub",URI="dub.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=1280x720,AUDIO="aac"
v.m3u8
"#;
        let (_, master) = parse_master_playlist(text).unwrap();
        let best = select_best_variant(&master, None).unwrap();
        assert!(select_audio_rendition(&master, best).is_none());
    }

    #[tokio::test]
    async fn separate_video_and_audio_tracks_end_up_joined_in_one_file() {
        if !crate::core::media_processor::check_ffmpeg() {
            eprintln!("ffmpeg is missing: skipped");
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "omniget-hls-join-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let ffmpeg = |args: &[&str]| {
            let ok = std::process::Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error"])
                .args(args)
                .current_dir(&dir)
                .status()
                .unwrap()
                .success();
            assert!(ok, "{args:?}");
        };
        ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x120:rate=10",
            "-t",
            "2",
            "-an",
            "-c:v",
            "libx264",
            "-f",
            "hls",
            "-hls_time",
            "1",
            "-hls_list_size",
            "0",
            "-hls_segment_filename",
            "v%d.ts",
            "video.m3u8",
        ]);
        ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "2",
            "-vn",
            "-c:a",
            "aac",
            "-f",
            "hls",
            "-hls_time",
            "1",
            "-hls_list_size",
            "0",
            "-hls_segment_filename",
            "a%d.ts",
            "audio.m3u8",
        ]);
        std::fs::write(
            dir.join("main.m3u8"),
            "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio.m3u8\",GROUP-ID=\"a\",NAME=\"En\",AUTOSELECT=YES,CHARACTERISTICS=\"DEFAULT=YES\"\n#EXT-X-STREAM-INF:BANDWIDTH=9000,RESOLUTION=160x120,AUDIO=\"a\"\nvideo.m3u8\n",
        )
        .unwrap();
        let root = dir.clone();
        let (base, _) = test_server::spawn(move |_, r| {
            match std::fs::read(root.join(r.path.trim_start_matches('/'))) {
                Ok(body) => test_server::Reply::new(200, body),
                Err(_) => test_server::Reply::new(404, Vec::new()),
            }
        })
        .await;
        let out = dir.join("joined.mp4");
        HlsDownloader::with_client(Client::new())
            .download(
                &format!("{base}/main.m3u8"),
                out.to_str().unwrap(),
                "",
                None,
                CancellationToken::new(),
                4,
                2,
            )
            .await
            .unwrap();
        let streams = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "csv=p=0",
            ])
            .arg(&out)
            .output()
            .unwrap();
        let streams = String::from_utf8_lossy(&streams.stdout).to_string();
        assert!(streams.contains("video"), "{streams}");
        assert!(streams.contains("audio"), "{streams}");
        assert!(!dir.join("joined.mp4.video.mp4").exists());
        assert!(!dir.join("joined.mp4.audio.m4a").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
