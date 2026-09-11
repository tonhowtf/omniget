use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use m3u8_rs::{parse_master_playlist, parse_media_playlist, MasterPlaylist, VariantStream};
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

pub struct HlsDownloader {
    client: Client,
    user_agent_override: Option<String>,
    /// Optional rich progress channel; receives percent (completed/total
    /// segments) plus accumulated downloaded bytes as segments finish.
    progress_tx: Option<mpsc::Sender<ProgressUpdate>>,
    /// A playlist whose text we already hold, as `(url, text)`. Set when the
    /// browser extension captured a manifest the native side could never
    /// fetch on its own (the page only ever handed it to the player through a
    /// `blob:` URL).
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
            user_agent_override: None,
            progress_tx: None,
            prefetched_playlist: None,
        }
    }

    pub fn with_user_agent_override(mut self, ua: Option<String>) -> Self {
        self.user_agent_override = ua;
        self
    }

    /// Attach a channel that receives per-segment progress updates
    /// (percent = completed / total segments, with accumulated bytes).
    pub fn with_progress(mut self, tx: mpsc::Sender<ProgressUpdate>) -> Self {
        self.progress_tx = Some(tx);
        self
    }

    /// Hand the downloader a playlist we already have the text of, so it is
    /// never fetched over the network. Used for manifests the page built in
    /// JavaScript and exposed only as a `blob:` URL, which does not resolve
    /// outside the tab that minted it.
    pub fn with_prefetched_playlist(mut self, url: String, text: String) -> Self {
        self.prefetched_playlist = Some((url, text));
        self
    }

    /// The prefetched text, but only when it belongs to `url`. A master
    /// playlist can be prefetched while its variants still have to be fetched
    /// normally, so the URL has to match exactly.
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
            let req = apply_referer_headers(self.client.get(url), referer)
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
        // The media playlist is fetched a second time here, independently of
        // `fetch_m3u8_with_retry`. Skipping this branch would throw the
        // prefetched text away and hit the network anyway.
        let text = match self.prefetched_for(m3u8_url) {
            Some(text) => {
                tracing::info!(
                    "[hls] using prefetched media playlist text ({} bytes)",
                    text.len()
                );
                text.to_string()
            }
            None => {
                let resp = apply_referer_headers(self.client.get(m3u8_url), referer)
                    .header("User-Agent", self.effective_user_agent())
                    .send()
                    .await?;

                if !resp.status().is_success() {
                    anyhow::bail!("HTTP {} fetching playlist", resp.status());
                }

                resp.text().await?
            }
        };

        let (_, playlist) = parse_media_playlist(text.as_bytes())
            .map_err(|e| anyhow::anyhow!("Parse media playlist: {:?}", e))?;

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
            )
            .await
        });

        let semaphore = Arc::new(Semaphore::new(max_concurrent as usize));
        let completed = Arc::new(AtomicUsize::new(0));
        let downloaded_bytes = Arc::new(AtomicU64::new(0));
        let fail_token = cancel_token.child_token();
        let errors: Arc<tokio::sync::Mutex<HashMap<String, u32>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        let segment_urls: Vec<(usize, String)> = playlist
            .segments
            .iter()
            .enumerate()
            .map(|(i, seg)| (i, resolve_url(m3u8_url, &seg.uri)))
            .collect();

        let client = &self.client;
        let errors_ref = &errors;
        let completed_ref = &completed;
        let downloaded_ref = &downloaded_bytes;
        let fail_ref = &fail_token;
        let sem_ref = &semaphore;
        let user_agent = self.effective_user_agent().to_string();
        let user_agent_ref = &user_agent;
        let progress_ref = &self.progress_tx;

        stream::iter(segment_urls)
            .map(|(i, url)| {
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
                        &url,
                        &referer,
                        user_agent_ref,
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
                                // try_send: progress is best-effort and must
                                // never stall segment downloads.
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
            let req = apply_referer_headers(self.client.get(url), referer)
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

/// Attach `Referer` (and a matching `Origin`) headers to a request.
/// An empty referer means "send no Referer/Origin at all" — some CDNs
/// reject requests with a wrong Referer but accept ones without any.
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

/// Origin (`scheme://host[:port]`, no path, no trailing slash) of a URL.
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

/// Picks the highest variant not above `max_height`; with no target, the
/// highest variant available. When every variant exceeds the target the
/// lowest one is the closest match, so it is returned instead of nothing.
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

fn resolve_url(base: &str, relative: &str) -> String {
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.to_string();
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

/// Concatenated MPEG-TS segments are not a valid MP4 container, so strict
/// players (QuickTime, Jellyfin) reject the file even though the streams
/// inside are compatible. When the caller asked for an MP4-family output,
/// remux the transport stream with ffmpeg (-c copy regenerating PTS) into a
/// real MP4. Falls back to a plain rename when ffmpeg is unavailable so the
/// download still completes.
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

async fn write_segments_ordered(
    mut rx: mpsc::Receiver<(usize, Vec<u8>)>,
    output_path: &PathBuf,
    encryption: &Option<EncryptionInfo>,
    media_sequence: u64,
    total_segments: usize,
) -> anyhow::Result<()> {
    use std::io::Write;
    let mut file =
        std::io::BufWriter::with_capacity(256 * 1024, std::fs::File::create(output_path)?);
    let mut next_expected: usize = 0;
    let mut pending: BTreeMap<usize, Vec<u8>> = BTreeMap::new();

    while let Some((idx, data)) = rx.recv().await {
        pending.insert(idx, data);

        while let Some(segment_data) = pending.remove(&next_expected) {
            // The image wrapper, when present, sits outside the encryption:
            // it has to come off before the AES-128 block decryption runs.
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

const SEGMENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

async fn download_segment_with_retry(
    client: &Client,
    url: &str,
    referer: &str,
    user_agent: &str,
    max_retries: u32,
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<u8>> {
    let mut last_err = None;
    for attempt in 0..max_retries {
        if cancel.is_cancelled() {
            anyhow::bail!("Download cancelled");
        }

        let result = tokio::time::timeout(SEGMENT_TIMEOUT, async {
            let resp = apply_referer_headers(client.get(url), referer)
                .header("User-Agent", user_agent)
                .send()
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let code = status.as_u16();
                if (400..500).contains(&code) && code != 429 && code != 408 {
                    return Err(anyhow::anyhow!("HTTP {} (fatal) downloading segment", code));
                }
                return Err(anyhow::anyhow!("HTTP {} downloading segment", code));
            }

            resp.bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| anyhow::anyhow!(e))
        })
        .await;

        match result {
            Ok(Ok(data)) => return Ok(data),
            Ok(Err(e)) => {
                if e.to_string().contains("(fatal)") {
                    return Err(e);
                }
                last_err = Some(e);
            }
            Err(_) => last_err = Some(anyhow::anyhow!("Timeout downloading segment")),
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

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const JPEG_SIGNATURE: [u8; 3] = [0xFF, 0xD8, 0xFF];

/// Number of leading bytes to drop from a segment that arrived disguised as an
/// image.
///
/// A handful of CDNs wrap each transport-stream segment in a real PNG or JPEG
/// file so that naive traffic inspection sees an image download. The media
/// payload is simply appended after the image ends, so the fix is to find the
/// image's end-of-file marker and start reading right after it:
///
/// * PNG ends with the `IEND` chunk — 4 bytes of type plus a 4-byte CRC32,
///   hence 8 bytes past the marker;
/// * JPEG ends with the `FF D9` EOI marker, 2 bytes long.
///
/// Anything that is not one of those two, or that carries the signature but
/// never the closing marker, returns 0 — the buffer is passed through
/// untouched. Returning a wrong offset would silently corrupt the output, so
/// every uncertain case errs towards leaving the bytes alone.
/// Where a PNG ends, found by walking its chunk table rather than by searching
/// for the bytes `IEND`.
///
/// The literal search is the obvious implementation and it is wrong: `IEND` is
/// four ordinary bytes that can occur inside the compressed data of an `IDAT`
/// chunk. Cutting there lands in the middle of the PNG, and the segment is
/// corrupt with nothing to report it. Every PNG chunk announces its own length,
/// so the real end is reachable exactly.
fn png_payload_offset(data: &[u8]) -> Option<usize> {
    let mut at = PNG_SIGNATURE.len();
    loop {
        // Each chunk is: 4-byte length, 4-byte type, payload, 4-byte CRC.
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

/// Where a JPEG ends: the first `FF D9` that is not inside the two-byte
/// signature. Unlike PNG this stays a scan — JPEG entropy-coded data escapes
/// its own `FF` bytes, so a literal `FF D9` in the stream really is the end of
/// image. The stricter marker check on the way in is what keeps random bytes
/// from ever reaching here.
fn jpeg_payload_offset(data: &[u8]) -> Option<usize> {
    find_subslice(&data[JPEG_SIGNATURE.len()..], &[0xFF, 0xD9])
        .map(|pos| JPEG_SIGNATURE.len() + pos + 2)
}

/// Whether the buffer opens like a real JPEG and not like three unlucky bytes.
///
/// This matters because the strip runs before decryption: the head of an
/// AES-128 segment is ciphertext, and `FF D8 FF` turns up in random bytes about
/// once every 16 million segments. Requiring a valid marker after the signature
/// takes that from "will happen to someone" to negligible.
fn looks_like_jpeg(data: &[u8]) -> bool {
    if !data.starts_with(&JPEG_SIGNATURE) || data.len() < 4 {
        return false;
    }
    // The byte after `FF D8 FF` is a marker code: APPn, DQT, DHT, SOF, COM…
    // Never 0x00 (a stuffed byte), never 0xFF (padding), never 0xD8 again.
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

    // A wrapper with nothing behind it is not a segment we can salvage;
    // handing back an empty buffer would corrupt the concatenated output.
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

    /// A minimal but structurally honest PNG: signature, one IHDR-ish chunk,
    /// then the IEND chunk with its 4-byte CRC.
    fn png_wrapped(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&PNG_SIGNATURE);
        out.extend_from_slice(&[0, 0, 0, 4]); // chunk length
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&[1, 2, 3, 4]); // chunk data
        out.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]); // chunk CRC
        out.extend_from_slice(&[0, 0, 0, 0]); // IEND length
        out.extend_from_slice(b"IEND");
        out.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]); // IEND CRC
        out.extend_from_slice(payload);
        out
    }

    fn jpeg_wrapped(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&JPEG_SIGNATURE);
        out.extend_from_slice(&[0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46]); // JFIF-ish
        out.extend_from_slice(&[0xFF, 0xD9]); // EOI
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

    /// A PNG whose IDAT payload happens to contain the bytes `IEND`.
    fn png_with_iend_bytes_inside_idat(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&PNG_SIGNATURE);
        let idat = b"....IEND....decoy";
        out.extend_from_slice(&(idat.len() as u32).to_be_bytes());
        out.extend_from_slice(b"IDAT");
        out.extend_from_slice(idat);
        out.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]); // chunk CRC
        out.extend_from_slice(&[0, 0, 0, 0]); // IEND length
        out.extend_from_slice(b"IEND");
        out.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]); // IEND CRC
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn iend_bytes_inside_the_image_do_not_cut_the_segment_short() {
        // `IEND` is four ordinary bytes and can occur inside compressed image
        // data. Searching for the pattern instead of walking the chunk table
        // cuts here, and the segment is corrupt with nothing to report it.
        let payload = b"\x47\x40\x11\x10 transport stream";
        let wrapped = png_with_iend_bytes_inside_idat(payload);
        assert_eq!(stripped(&wrapped), payload);
    }

    #[test]
    fn three_unlucky_bytes_are_not_a_jpeg() {
        // The strip runs before decryption, so the head of an AES-128 segment
        // is ciphertext: `FF D8 FF` shows up in random bytes roughly once every
        // 16 million segments. What follows a real signature is a marker code.
        let mut ciphertext = vec![0xFF, 0xD8, 0xFF, 0x1A];
        ciphertext.extend_from_slice(b"encrypted segment bytes\xFF\xD9 and more");
        assert_eq!(image_wrapper_offset(&ciphertext), 0);
        assert_eq!(stripped(&ciphertext), &ciphertext[..]);

        // A real JPEG still strips: APP0 follows the signature.
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
        // Query strings are part of the identity: CDNs key tokens on them.
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
}
