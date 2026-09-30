//! Records a live HLS stream to a file, from the live edge forward, until a
//! time limit, the end of the stream, a stop request or a stream that stops
//! sending new segments.
//!
//! The normal HLS download fetches the whole playlist once, which on a live
//! stream means the replay window the server keeps (often ten minutes of the
//! past) and nothing after it. Recording is a different loop, taken from
//! Streamlink (BSD-2-Clause; reimplemented here, no code copied):
//! - start a few segments behind the live edge (`--hls-live-edge`, 3);
//! - reload the media playlist every target duration, and after half of it
//!   when nothing new arrived;
//! - write each new segment once, in playlist order;
//! - stop when the written segments add up to the limit
//!   (`--stream-segmented-duration`), at `#EXT-X-ENDLIST`, or when no new
//!   segment arrives for a while (`--stream-timeout`);
//! - a segment that fails three times is skipped and counted, not fatal.
//!
//! Unlike Streamlink, the result is remuxed into an MP4 when ffmpeg is there,
//! so the file plays everywhere, and a separate audio rendition is recorded
//! alongside the video and muxed in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use m3u8_rs::{AlternativeMediaType, KeyMethod, MasterPlaylist, MediaPlaylist, Playlist};
use reqwest::Client;
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const SEGMENT_ATTEMPTS: u32 = 3;
const SEGMENT_TIMEOUT: Duration = Duration::from_secs(20);

pub struct LiveRecordOptions {
    /// A master or media playlist (`.m3u8`).
    pub playlist_url: String,
    /// Highest video height to pick from a master playlist; `None` = best.
    pub max_height: Option<u32>,
    /// Stop after this much recorded media; `None` = until stopped or ended.
    pub max_seconds: Option<f64>,
    /// Output path without extension: `.mp4` after the remux, `.ts` without ffmpeg.
    pub output_stem: PathBuf,
    /// Extra request headers (Referer, Cookie...) the site expects.
    pub headers: Vec<(String, String)>,
    /// Segments behind the live edge to start from (Streamlink's default is 3).
    pub live_edge: usize,
    /// Give up when no new segment arrived for this long.
    pub stall_timeout: Duration,
}

impl LiveRecordOptions {
    pub fn new(playlist_url: impl Into<String>, output_stem: PathBuf) -> Self {
        Self {
            playlist_url: playlist_url.into(),
            max_height: None,
            max_seconds: None,
            output_stem,
            headers: Vec::new(),
            live_edge: 3,
            stall_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct LiveRecordProgress {
    pub recorded_s: f64,
    pub bytes: u64,
    pub segments: usize,
    pub skipped: usize,
    /// "1280x720" (or the bandwidth) of the chosen variant.
    pub variant: Option<String>,
    /// False when the playlist turned out to be a finished (VOD) stream.
    pub is_live: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LiveRecordEnd {
    TimeLimit,
    StreamEnded,
    Stopped,
    Stalled,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveRecordResult {
    pub path: String,
    pub recorded_s: f64,
    pub bytes: u64,
    pub segments: usize,
    pub skipped: usize,
    pub ended: LiveRecordEnd,
    /// True when ffmpeg turned the transport stream into an MP4.
    pub remuxed: bool,
    pub variant: Option<String>,
}

/// The variant to record and, when its audio lives in a separate rendition,
/// that rendition's URI. Highest bandwidth at or under `max_height`; when
/// every variant is taller, the smallest one. Video-less variants lose to any
/// variant with a picture.
pub fn pick_variant(
    master: &MasterPlaylist,
    max_height: Option<u32>,
) -> Option<(String, Option<String>, Option<String>)> {
    let variants: Vec<_> = master.variants.iter().filter(|v| !v.is_i_frame).collect();
    let height = |v: &&m3u8_rs::VariantStream| v.resolution.map(|r| r.height).unwrap_or(0);
    let with_video: Vec<_> = variants
        .iter()
        .copied()
        .filter(|v| {
            v.resolution.is_some()
                || v.codecs
                    .as_deref()
                    .map(|c| {
                        c.contains("avc")
                            || c.contains("hvc")
                            || c.contains("hev")
                            || c.contains("av01")
                            || c.contains("vp09")
                    })
                    .unwrap_or(false)
        })
        .collect();
    let pool = if with_video.is_empty() {
        variants
    } else {
        with_video
    };
    let fits: Vec<_> = pool
        .iter()
        .copied()
        .filter(|v| max_height.map(|m| height(v) <= m as u64).unwrap_or(true))
        .collect();
    let chosen = if fits.is_empty() {
        pool.iter()
            .copied()
            .min_by_key(|v| (height(v), v.bandwidth))?
    } else {
        fits.iter()
            .copied()
            .max_by_key(|v| (height(v), v.bandwidth))?
    };
    let audio = chosen.audio.as_ref().and_then(|group| {
        let in_group: Vec<_> = master
            .alternatives
            .iter()
            .filter(|a| {
                a.media_type == AlternativeMediaType::Audio
                    && &a.group_id == group
                    && a.uri.is_some()
            })
            .collect();
        in_group
            .iter()
            .find(|a| a.default)
            .or_else(|| in_group.first())
            .and_then(|a| a.uri.clone())
    });
    let label = chosen
        .resolution
        .map(|r| format!("{}x{}", r.width, r.height))
        .or_else(|| Some(format!("{} kbps", chosen.bandwidth / 1000)));
    Some((chosen.uri.clone(), audio, label))
}

/// Where a new recording starts in a playlist: at the first segment of a
/// finished stream, else `live_edge` segments before the end.
pub fn start_sequence(playlist: &MediaPlaylist, live_edge: usize) -> u64 {
    if playlist.end_list {
        playlist.media_sequence
    } else {
        playlist.media_sequence + playlist.segments.len().saturating_sub(live_edge.max(1)) as u64
    }
}

/// Absolute URL of `relative` against the playlist at `base`.
pub fn resolve(base: &str, relative: &str) -> String {
    match url::Url::parse(base).and_then(|b| b.join(relative)) {
        Ok(u) => u.to_string(),
        Err(_) => relative.to_string(),
    }
}

/// AES-128 IV: the explicit one, else the media sequence number, big-endian.
fn iv_for(explicit: Option<&str>, sequence: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    if let Some(hex) = explicit {
        let hex = hex.trim_start_matches("0x").trim_start_matches("0X");
        let padded = format!("{hex:0>32}");
        for (i, byte) in iv.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&padded[i * 2..i * 2 + 2], 16).unwrap_or(0);
        }
    } else {
        iv[8..].copy_from_slice(&sequence.to_be_bytes());
    }
    iv
}

fn decrypt(data: &[u8], key: &[u8], iv: &[u8; 16]) -> anyhow::Result<Vec<u8>> {
    use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
    let mut buf = data.to_vec();
    let dec =
        Aes128CbcDec::new_from_slices(key, iv).map_err(|e| anyhow::anyhow!("AES key: {e:?}"))?;
    let out = dec
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| anyhow::anyhow!("AES decrypt: {e:?}"))?;
    Ok(out.to_vec())
}

struct Fetcher {
    client: Client,
    headers: Vec<(String, String)>,
}

impl Fetcher {
    fn request(&self, url: &str, range: Option<(u64, u64)>) -> reqwest::RequestBuilder {
        let mut req = self.client.get(url).timeout(SEGMENT_TIMEOUT);
        if !self
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
        {
            req = req.header(reqwest::header::USER_AGENT, USER_AGENT);
        }
        for (k, v) in &self.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some((start, len)) = range {
            req = req.header(
                reqwest::header::RANGE,
                format!("bytes={}-{}", start, start + len.saturating_sub(1)),
            );
        }
        req
    }

    async fn text(&self, url: &str) -> anyhow::Result<String> {
        let resp = self.request(url, None).send().await?.error_for_status()?;
        Ok(resp.text().await?)
    }

    async fn bytes(&self, url: &str, range: Option<(u64, u64)>) -> anyhow::Result<Vec<u8>> {
        let mut last = None;
        for attempt in 0..SEGMENT_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
            }
            match self
                .request(url, range)
                .send()
                .await
                .and_then(|r| r.error_for_status())
            {
                Ok(resp) => match resp.bytes().await {
                    Ok(b) => return Ok(b.to_vec()),
                    Err(e) => last = Some(anyhow::Error::from(e)),
                },
                Err(e) => last = Some(anyhow::Error::from(e)),
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("segment failed")))
    }
}

struct TrackOutcome {
    part: PathBuf,
    recorded_s: f64,
    bytes: u64,
    segments: usize,
    skipped: usize,
    ended: LiveRecordEnd,
}

#[allow(clippy::too_many_arguments)]
async fn record_track(
    fetcher: &Fetcher,
    url: &str,
    part: PathBuf,
    max_seconds: Option<f64>,
    live_edge: usize,
    stall_timeout: Duration,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(f64, u64, usize, usize, bool),
) -> anyhow::Result<TrackOutcome> {
    let mut file = tokio::fs::File::create(&part).await?;
    let mut next_seq: Option<u64> = None;
    let mut last_new = Instant::now();
    let mut keys: HashMap<String, Vec<u8>> = HashMap::new();
    let mut written_init: Option<String> = None;
    let (mut recorded_s, mut bytes, mut segments, mut skipped) = (0f64, 0u64, 0usize, 0usize);
    let mut is_live;
    let ended = 'outer: loop {
        if cancel.is_cancelled() {
            break LiveRecordEnd::Stopped;
        }
        let playlist = match fetcher
            .text(url)
            .await
            .map(|t| m3u8_rs::parse_media_playlist_res(t.as_bytes()).map_err(|e| e.to_string()))
        {
            Ok(Ok(p)) => p,
            Ok(Err(e)) if next_seq.is_none() => anyhow::bail!("Not a readable HLS playlist ({e})"),
            Err(e) if next_seq.is_none() => return Err(e),
            _ => {
                // A reload failed mid-recording: the stream may come back.
                if last_new.elapsed() > stall_timeout {
                    break LiveRecordEnd::Stalled;
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(2)) => continue,
                    _ = cancel.cancelled() => break LiveRecordEnd::Stopped,
                }
            }
        };
        is_live = !playlist.end_list;
        let first = playlist.media_sequence;
        let start = match next_seq {
            None => start_sequence(&playlist, live_edge),
            Some(s) if s < first => {
                // The window moved past us (a slow reload): those segments are gone.
                skipped += (first - s) as usize;
                first
            }
            Some(s) => s,
        };
        let mut got_new = false;
        let mut key: Option<m3u8_rs::Key> = None;
        let mut map: Option<m3u8_rs::Map> = None;
        let mut range_end: HashMap<String, u64> = HashMap::new();
        for (i, seg) in playlist.segments.iter().enumerate() {
            // EXT-X-KEY and EXT-X-MAP apply to every following segment.
            if seg.key.is_some() {
                key = seg.key.clone();
            }
            if seg.map.is_some() {
                map = seg.map.clone();
            }
            let seg_url = resolve(url, &seg.uri);
            let range = seg.byte_range.as_ref().map(|r| {
                let off = r
                    .offset
                    .unwrap_or_else(|| range_end.get(&seg_url).copied().unwrap_or(0));
                range_end.insert(seg_url.clone(), off + r.length);
                (off, r.length)
            });
            let seq = first + i as u64;
            if seq < start {
                continue;
            }
            if cancel.is_cancelled() {
                break 'outer LiveRecordEnd::Stopped;
            }
            got_new = true;
            next_seq = Some(seq + 1);
            if let Some(m) = &map {
                let init_url = resolve(url, &m.uri);
                if written_init.as_deref() != Some(init_url.as_str()) {
                    let init_range = m
                        .byte_range
                        .as_ref()
                        .map(|r| (r.offset.unwrap_or(0), r.length));
                    let init = fetcher.bytes(&init_url, init_range).await?;
                    file.write_all(&init).await?;
                    bytes += init.len() as u64;
                    written_init = Some(init_url);
                }
            }
            let data = match fetcher.bytes(&seg_url, range).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("live record: segment {seq} skipped: {e}");
                    skipped += 1;
                    continue;
                }
            };
            let data = match &key {
                Some(k) if k.method == KeyMethod::AES128 => {
                    let key_url = resolve(url, k.uri.as_deref().unwrap_or_default());
                    if !keys.contains_key(&key_url) {
                        let kb = fetcher.bytes(&key_url, None).await?;
                        keys.insert(key_url.clone(), kb);
                    }
                    decrypt(&data, &keys[&key_url], &iv_for(k.iv.as_deref(), seq))?
                }
                Some(k) if k.method != KeyMethod::None => {
                    anyhow::bail!("This stream is protected (DRM); it cannot be recorded");
                }
                _ => data,
            };
            file.write_all(&data).await?;
            recorded_s += seg.duration as f64;
            bytes += data.len() as u64;
            segments += 1;
            on_progress(recorded_s, bytes, segments, skipped, is_live);
            if max_seconds.map(|m| recorded_s >= m).unwrap_or(false) {
                break 'outer LiveRecordEnd::TimeLimit;
            }
        }
        if playlist.end_list {
            break LiveRecordEnd::StreamEnded;
        }
        if got_new {
            last_new = Instant::now();
        } else if last_new.elapsed() > stall_timeout {
            break LiveRecordEnd::Stalled;
        }
        let target = Duration::from_secs(playlist.target_duration.max(1));
        let wait = if got_new { target } else { target / 2 };
        tokio::select! {
            _ = tokio::time::sleep(wait.max(Duration::from_millis(500))) => {}
            _ = cancel.cancelled() => break LiveRecordEnd::Stopped,
        }
    };
    file.flush().await?;
    Ok(TrackOutcome {
        part,
        recorded_s,
        bytes,
        segments,
        skipped,
        ended,
    })
}

/// `stem` plus `ext`, or `stem (2)`, `stem (3)`... when that file exists.
pub fn free_path(stem: &Path, ext: &str) -> PathBuf {
    let base = stem.as_os_str().to_string_lossy().into_owned();
    let mut candidate = PathBuf::from(format!("{base}.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = PathBuf::from(format!("{base} ({n}).{ext}"));
        n += 1;
    }
    candidate
}

async fn remux(video: &Path, audio: Option<&Path>, out: &Path) -> bool {
    if !crate::core::media_processor::check_ffmpeg() {
        return false;
    }
    let mut cmd = crate::core::process::command("ffmpeg");
    cmd.args(["-y", "-loglevel", "error", "-fflags", "+genpts", "-i"])
        .arg(video);
    if let Some(a) = audio {
        cmd.arg("-i").arg(a).args(["-map", "0:v?", "-map", "1:a?"]);
    }
    cmd.args(["-c", "copy", "-movflags", "+faststart"])
        .arg(out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    matches!(cmd.status().await, Ok(s) if s.success())
}

/// Records `opts.playlist_url` until the limit, the end of the stream, a stall
/// or `cancel`. What was recorded before a stop is kept.
pub async fn record(
    opts: LiveRecordOptions,
    cancel: CancellationToken,
    on_progress: impl Fn(&LiveRecordProgress) + Send + Sync,
) -> anyhow::Result<LiveRecordResult> {
    let client = crate::core::http_client::apply_global_proxy(
        Client::builder().connect_timeout(Duration::from_secs(20)),
    )
    .build()?;
    let fetcher = Fetcher {
        client,
        headers: opts.headers.clone(),
    };
    let text = fetcher.text(&opts.playlist_url).await?;
    let (video_url, audio_url, variant) = match m3u8_rs::parse_playlist_res(text.as_bytes()) {
        Ok(Playlist::MasterPlaylist(master)) => {
            let (v, a, label) = pick_variant(&master, opts.max_height)
                .ok_or_else(|| anyhow::anyhow!("The playlist lists no stream to record"))?;
            (
                resolve(&opts.playlist_url, &v),
                a.map(|a| resolve(&opts.playlist_url, &a)),
                label,
            )
        }
        Ok(Playlist::MediaPlaylist(_)) => (opts.playlist_url.clone(), None, None),
        Err(_) => anyhow::bail!("This is not an HLS (.m3u8) playlist"),
    };
    if let Some(parent) = opts.output_stem.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let stem = opts.output_stem.as_os_str().to_string_lossy().into_owned();
    let video_part = PathBuf::from(format!("{stem}.recording.ts"));
    let audio_part = PathBuf::from(format!("{stem}.recording-audio.ts"));

    let progress = std::sync::Mutex::new(LiveRecordProgress {
        variant: variant.clone(),
        is_live: true,
        ..Default::default()
    });
    let report = |recorded_s: f64, bytes: u64, segments: usize, skipped: usize, is_live: bool| {
        if let Ok(mut p) = progress.lock() {
            p.recorded_s = recorded_s;
            p.bytes = bytes;
            p.segments = segments;
            p.skipped = skipped;
            p.is_live = is_live;
            on_progress(&p);
        }
    };
    // The audio track stops with the video one: same limit, and cancelled when the video ends.
    let audio_stop = cancel.child_token();
    let video = async {
        let r = record_track(
            &fetcher,
            &video_url,
            video_part.clone(),
            opts.max_seconds,
            opts.live_edge,
            opts.stall_timeout,
            &cancel,
            report,
        )
        .await;
        audio_stop.cancel();
        r
    };
    let audio = async {
        match &audio_url {
            Some(a) => Some(
                record_track(
                    &fetcher,
                    a,
                    audio_part.clone(),
                    opts.max_seconds,
                    opts.live_edge,
                    opts.stall_timeout,
                    &audio_stop,
                    |_, _, _, _, _| {},
                )
                .await,
            ),
            None => None,
        }
    };
    let (video, audio) = tokio::join!(video, audio);
    let video = match video {
        Ok(v) => v,
        Err(e) => {
            let _ = tokio::fs::remove_file(&video_part).await;
            let _ = tokio::fs::remove_file(&audio_part).await;
            return Err(e);
        }
    };
    let audio = audio.and_then(|a| a.ok()).filter(|a| a.segments > 0);
    if video.segments == 0 {
        let _ = tokio::fs::remove_file(&video.part).await;
        let _ = tokio::fs::remove_file(&audio_part).await;
        anyhow::bail!(match video.ended {
            LiveRecordEnd::Stopped => "Stopped before the first segment arrived",
            _ => "The stream sent no segment to record",
        });
    }
    let bytes = video.bytes + audio.as_ref().map(|a| a.bytes).unwrap_or(0);
    let mp4 = free_path(&opts.output_stem, "mp4");
    let remuxed = remux(&video.part, audio.as_ref().map(|a| a.part.as_path()), &mp4).await;
    let path = if remuxed {
        let _ = tokio::fs::remove_file(&video.part).await;
        let _ = tokio::fs::remove_file(&audio_part).await;
        mp4
    } else {
        let _ = tokio::fs::remove_file(&mp4).await;
        let ts = free_path(&opts.output_stem, "ts");
        tokio::fs::rename(&video.part, &ts).await?;
        if audio.is_some() {
            let _ = tokio::fs::rename(&audio_part, free_path(&opts.output_stem, "audio.ts")).await;
        }
        ts
    };
    Ok(LiveRecordResult {
        path: path.to_string_lossy().into_owned(),
        recorded_s: (video.recorded_s * 10.0).round() / 10.0,
        bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(bytes),
        segments: video.segments,
        skipped: video.skipped,
        ended: video.ended,
        remuxed,
        variant,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn master(text: &str) -> MasterPlaylist {
        match m3u8_rs::parse_playlist_res(text.as_bytes()).unwrap() {
            Playlist::MasterPlaylist(m) => m,
            _ => panic!("not a master playlist"),
        }
    }

    fn media(text: &str) -> MediaPlaylist {
        m3u8_rs::parse_media_playlist_res(text.as_bytes()).unwrap()
    }

    const LADDER: &str = "#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",DEFAULT=YES,URI=\"audio/en.m3u8\"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Other\",DEFAULT=NO,URI=\"audio/xx.m3u8\"
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,AUDIO=\"aud\"
v360.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720,AUDIO=\"aud\"
v720.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,AUDIO=\"aud\"
v1080.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS=\"mp4a.40.2\",AUDIO=\"aud\"
audio-only.m3u8
";

    #[test]
    fn picks_the_best_variant_under_the_height_with_its_default_audio() {
        let m = master(LADDER);
        let (v, a, label) = pick_variant(&m, Some(720)).unwrap();
        assert_eq!(v, "v720.m3u8");
        assert_eq!(a.as_deref(), Some("audio/en.m3u8"));
        assert_eq!(label.as_deref(), Some("1280x720"));
        assert_eq!(pick_variant(&m, None).unwrap().0, "v1080.m3u8");
    }

    #[test]
    fn falls_back_to_the_smallest_when_every_variant_is_taller() {
        assert_eq!(
            pick_variant(&master(LADDER), Some(240)).unwrap().0,
            "v360.m3u8"
        );
    }

    #[test]
    fn audio_muxed_into_the_variant_needs_no_second_track() {
        let m = master(
            "#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"English\",DEFAULT=YES
#EXT-X-STREAM-INF:BANDWIDTH=1316000,RESOLUTION=1280x720,AUDIO=\"a\"
live-720.m3u8
",
        );
        let (v, a, _) = pick_variant(&m, None).unwrap();
        assert_eq!(v, "live-720.m3u8");
        assert_eq!(a, None);
    }

    #[test]
    fn a_live_recording_starts_three_segments_behind_the_edge() {
        let p = media(
            "#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:100
#EXTINF:2,
a.ts
#EXTINF:2,
b.ts
#EXTINF:2,
c.ts
#EXTINF:2,
d.ts
#EXTINF:2,
e.ts
",
        );
        assert_eq!(start_sequence(&p, 3), 102);
        assert_eq!(start_sequence(&p, 10), 100);
    }

    #[test]
    fn a_finished_stream_is_recorded_from_its_first_segment() {
        let p = media(
            "#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:7
#EXTINF:2,
a.ts
#EXTINF:2,
b.ts
#EXT-X-ENDLIST
",
        );
        assert_eq!(start_sequence(&p, 3), 7);
    }

    #[test]
    fn relative_segment_urls_resolve_against_the_playlist() {
        assert_eq!(
            resolve(
                "https://cdn.example/live/stream.isml/.m3u8",
                "live-1000.m3u8"
            ),
            "https://cdn.example/live/stream.isml/live-1000.m3u8"
        );
        assert_eq!(
            resolve("https://a.example/x/y.m3u8", "https://b.example/z.ts"),
            "https://b.example/z.ts"
        );
    }

    #[test]
    fn the_iv_defaults_to_the_media_sequence() {
        let iv = iv_for(None, 0x0102);
        assert_eq!(&iv[..14], &[0u8; 14]);
        assert_eq!(&iv[14..], &[0x01, 0x02]);
        assert_eq!(
            iv_for(Some("0x0000000000000000000000000000000A"), 5)[15],
            0x0a
        );
    }

    #[test]
    fn free_path_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("omniget-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stem = dir.join("rec");
        assert_eq!(free_path(&stem, "mp4"), dir.join("rec.mp4"));
        std::fs::write(dir.join("rec.mp4"), b"x").unwrap();
        assert_eq!(free_path(&stem, "mp4"), dir.join("rec (2).mp4"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
