use crate::models::progress::ProgressUpdate;
use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::direct_downloader;
use crate::core::hls_downloader::HlsDownloader;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality};
use crate::platforms::traits::PlatformDownloader;

const API_BASE: &str = "https://public.api.bsky.app/xrpc/app.bsky.feed.getPostThread";
const RESOLVE_HANDLE: &str = "https://public.api.bsky.app/xrpc/com.atproto.identity.resolveHandle";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub struct BlueskyDownloader {
    client: reqwest::Client,
}

impl Default for BlueskyDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl BlueskyDownloader {
    async fn fallback_ytdlp(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let ytdlp_path = crate::core::ytdlp::ensure_ytdlp().await?;
        let json = crate::core::ytdlp::get_video_info(&ytdlp_path, url, &[]).await?;
        crate::platforms::generic_ytdlp::GenericYtdlpDownloader::parse_video_info(&json)
    }

    async fn native_get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let (user, post_id) = Self::extract_user_and_post(url)
            .ok_or_else(|| anyhow!("Could not extract user and post_id from URL"))?;

        let json = self.fetch_post(&user, &post_id).await?;

        let embed = json
            .pointer("/thread/post/embed")
            .ok_or_else(|| anyhow!("Post does not contain media"))?;

        let media = extract_media(embed).ok_or_else(|| anyhow!("Unsupported media type"))?;

        let filename_base = format!("bluesky_{}_{}", sanitize_filename::sanitize(&user), post_id);

        match media {
            BlueskyMedia::Video {
                hls_url,
                blob,
                height,
            } => {
                // Original upload first (single MP4 from the author's PDS, as
                // gallery-dl bluesky.py:63-70), HLS as fallback.
                let blob_src = match blob {
                    Some((did, cid)) => self
                        .resolve_pds(&did)
                        .await
                        .map(|pds| blob_url(&pds, &did, &cid)),
                    None => None,
                };
                Ok(MediaInfo {
                    title: filename_base,
                    author: user,
                    platform: "bluesky".to_string(),
                    duration_seconds: None,
                    thumbnail_url: None,
                    available_qualities: video_qualities(blob_src, height, hls_url),
                    media_type: MediaType::Video,
                    file_size_bytes: None,
                })
            }
            BlueskyMedia::Images { urls } => {
                let media_type = if urls.len() == 1 {
                    MediaType::Photo
                } else {
                    MediaType::Carousel
                };
                let qualities: Vec<VideoQuality> = urls
                    .iter()
                    .enumerate()
                    .map(|(i, u)| VideoQuality {
                        label: format!("{}", i + 1),
                        width: 0,
                        height: 0,
                        url: u.clone(),
                        format: "jpg".to_string(),
                    })
                    .collect();
                Ok(MediaInfo {
                    title: filename_base,
                    author: user,
                    platform: "bluesky".to_string(),
                    duration_seconds: None,
                    thumbnail_url: None,
                    available_qualities: qualities,
                    media_type,
                    file_size_bytes: None,
                })
            }
            BlueskyMedia::Gif { url: gif_url } => Ok(MediaInfo {
                title: filename_base,
                author: user,
                platform: "bluesky".to_string(),
                duration_seconds: None,
                thumbnail_url: None,
                available_qualities: vec![VideoQuality {
                    label: "original".to_string(),
                    width: 0,
                    height: 0,
                    url: gif_url,
                    format: "gif".to_string(),
                }],
                media_type: MediaType::Gif,
                file_size_bytes: None,
            }),
        }
    }

    pub fn new() -> Self {
        let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self { client }
    }

    fn extract_user_and_post(url: &str) -> Option<(String, String)> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() >= 4 && segments[0] == "profile" && segments[2] == "post" {
            return Some((segments[1].to_string(), segments[3].to_string()));
        }
        None
    }

    /// The AppView answers 400 NotFound for an at:// URI with a custom-domain
    /// handle (cinny.bun.how, 26/09) although the handle resolves; yt-dlp fails
    /// the same way. Resolve to the DID first, keep the handle on failure.
    async fn resolve_did(&self, user: &str) -> String {
        if user.starts_with("did:") {
            return user.to_string();
        }
        let url = format!("{}?handle={}", RESOLVE_HANDLE, urlencoding::encode(user));
        let did = self
            .get_json(&url)
            .await
            .and_then(|v| v.get("did")?.as_str().map(str::to_owned));
        did.filter(|d| is_did(d))
            .unwrap_or_else(|| user.to_string())
    }

    async fn resolve_pds(&self, did: &str) -> Option<String> {
        let doc = self.get_json(&did_doc_url(did)?).await?;
        pds_from_did_doc(&doc)
    }

    async fn get_json(&self, url: &str) -> Option<serde_json::Value> {
        let r = self.client.get(url).send().await.ok()?;
        if !r.status().is_success() {
            return None;
        }
        r.json().await.ok()
    }

    async fn fetch_post(&self, user: &str, post_id: &str) -> anyhow::Result<serde_json::Value> {
        let repo = self.resolve_did(user).await;
        let uri = format!("at://{}/app.bsky.feed.post/{}", repo, post_id);
        let url = format!(
            "{}?depth=0&parentHeight=0&uri={}",
            API_BASE,
            urlencoding::encode(&uri)
        );

        let response = self.client.get(&url).send().await?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(thread_http_error(status.as_u16(), &body));
        }

        let json: serde_json::Value = serde_json::from_str(&body)?;

        if let Some(error) = json.get("error").and_then(|e| e.as_str()) {
            return match error {
                "NotFound" | "InternalServerError" => Err(anyhow!("Post not available")),
                "InvalidRequest" => Err(anyhow!("Unsupported link")),
                _ => Err(anyhow!("Erro da API: {}", error)),
            };
        }

        Ok(json)
    }
}

fn is_did(s: &str) -> bool {
    (s.starts_with("did:plc:") || s.starts_with("did:web:"))
        && s.len() <= 2048
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":._-%".contains(&b))
}

fn did_doc_url(did: &str) -> Option<String> {
    if !is_did(did) {
        return None;
    }
    if let Some(host) = did.strip_prefix("did:web:") {
        return Some(format!(
            "https://{}/.well-known/did.json",
            host.replace("%3A", ":")
        ));
    }
    Some(format!("https://plc.directory/{}", did))
}

fn pds_from_did_doc(doc: &serde_json::Value) -> Option<String> {
    doc.get("service")?
        .as_array()?
        .iter()
        .find(|s| {
            s.get("id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id.ends_with("#atproto_pds"))
        })?
        .get("serviceEndpoint")?
        .as_str()
        .filter(|e| e.starts_with("https://"))
        .map(|e| e.trim_end_matches('/').to_string())
}

fn blob_url(pds: &str, did: &str, cid: &str) -> String {
    format!(
        "{}/xrpc/com.atproto.sync.getBlob?did={}&cid={}",
        pds,
        urlencoding::encode(did),
        urlencoding::encode(cid)
    )
}

/// `video.bsky.app/watch/<did>/<cid>/playlist.m3u8` names the blob's owner
/// and CID, also for quoted posts.
fn blob_ref_from_playlist(playlist: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(playlist).ok()?;
    let mut segs = parsed.path_segments()?;
    if segs.next()? != "watch" {
        return None;
    }
    let did = urlencoding::decode(segs.next()?).ok()?.into_owned();
    let cid = segs.next()?.to_string();
    if !is_did(&did) || cid.is_empty() || !cid.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    Some((did, cid))
}

fn video_from_view(view: &serde_json::Value) -> Option<BlueskyMedia> {
    let playlist = view.get("playlist")?.as_str()?;
    Some(BlueskyMedia::Video {
        hls_url: playlist.replace("video.bsky.app/watch/", "video.cdn.bsky.app/hls/"),
        blob: blob_ref_from_playlist(playlist),
        height: original_height(view),
    })
}

/// Height of the original upload from the embed's `aspectRatio`, which the
/// official clients fill with the pixel size (1920x1080 on the round-1
/// fixtures). A reduced ratio (16:9) is not a size: unknown (0).
fn original_height(view: &serde_json::Value) -> u32 {
    let dim = |k: &str| {
        view.pointer(&format!("/aspectRatio/{k}"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    };
    let (w, h) = (dim("width"), dim("height"));
    if w.min(h) >= 144 {
        u32::try_from(h).unwrap_or(0)
    } else {
        0
    }
}

/// Ceiling from the download options (`"720"` or `"720p"`).
fn ceiling_of(quality: Option<&str>) -> Option<u32> {
    quality
        .and_then(|q| q.trim_end_matches('p').parse().ok())
        .filter(|h| *h > 0)
}

/// The original blob is used only when it provably fits the ceiling.
fn blob_allowed(q: &VideoQuality, ceiling: Option<u32>) -> bool {
    match ceiling {
        None => true,
        Some(c) => q.height > 0 && q.height <= c,
    }
}

fn video_qualities(blob_src: Option<String>, height: u32, hls_url: String) -> Vec<VideoQuality> {
    let mut q = Vec::with_capacity(2);
    if let Some(url) = blob_src {
        q.push(VideoQuality {
            label: "original".to_string(),
            width: 0,
            height,
            url,
            format: "mp4".to_string(),
        });
    }
    q.push(VideoQuality {
        label: "best".to_string(),
        width: 0,
        height: 0,
        url: hls_url,
        format: "hls".to_string(),
    });
    q
}

/// Both causes survive: the native one as context, yt-dlp's as the source.
fn chain_fallback(native: anyhow::Error, ytdlp: anyhow::Error) -> anyhow::Error {
    ytdlp.context(format!("native: {native:#}; yt-dlp fallback failed"))
}

enum BlueskyMedia {
    Video {
        hls_url: String,
        blob: Option<(String, String)>,
        height: u32,
    },
    Images {
        urls: Vec<String>,
    },
    Gif {
        url: String,
    },
}

fn extract_media(embed: &serde_json::Value) -> Option<BlueskyMedia> {
    let embed_type = embed.get("$type")?.as_str()?;

    match embed_type {
        "app.bsky.embed.video#view" => video_from_view(embed),
        "app.bsky.embed.record#view" => {
            // Quote post: the media lives in the quoted record.
            embed
                .get("record")?
                .get("embeds")?
                .as_array()?
                .iter()
                .find_map(extract_media)
        }
        "app.bsky.embed.images#view" => {
            let images = embed.get("images")?.as_array()?;
            let urls: Vec<String> = images
                .iter()
                .filter_map(|img| img.get("fullsize")?.as_str().map(|s| s.to_string()))
                .collect();
            if urls.is_empty() {
                return None;
            }
            Some(BlueskyMedia::Images { urls })
        }
        "app.bsky.embed.external#view" => {
            let uri = embed.get("external")?.get("uri")?.as_str()?;
            extract_gif_from_uri(uri)
        }
        "app.bsky.embed.recordWithMedia#view" => {
            let media = embed.get("media")?;
            let media_type = media.get("$type")?.as_str()?;
            if media_type == "app.bsky.embed.external#view" {
                let uri = media.get("external")?.get("uri")?.as_str()?;
                return extract_gif_from_uri(uri);
            }
            if media_type == "app.bsky.embed.video#view" {
                return video_from_view(media);
            }
            None
        }
        _ => None,
    }
}

fn extract_gif_from_uri(uri: &str) -> Option<BlueskyMedia> {
    let parsed = url::Url::parse(uri).ok()?;
    if parsed.host_str()? == "media.tenor.com" {
        let mut clean = parsed.clone();
        clean.set_query(None);
        return Some(BlueskyMedia::Gif {
            url: clean.to_string(),
        });
    }
    None
}

#[async_trait]
impl PlatformDownloader for BlueskyDownloader {
    fn name(&self) -> &str {
        "bluesky"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "bsky.app" || host.ends_with(".bsky.app");
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        match self.native_get_media_info(url).await {
            Ok(info) => Ok(info),
            Err(native_err) => {
                tracing::warn!(
                    "[bluesky] native failed: {}, trying yt-dlp fallback",
                    native_err
                );
                self.fallback_ytdlp(url)
                    .await
                    .map_err(|ytdlp_err| chain_fallback(native_err, ytdlp_err))
            }
        }
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        if let Some(quality) = info.available_qualities.first() {
            if quality.format == "ytdlp" {
                let ytdlp_path = crate::core::ytdlp::ensure_ytdlp().await?;
                return crate::core::ytdlp::download_video(
                    &ytdlp_path,
                    &quality.url,
                    &opts.output_dir,
                    None,
                    progress,
                    opts.download_mode.as_deref(),
                    opts.format_id.as_deref(),
                    opts.filename_template.as_deref(),
                    opts.referer.as_deref().or(Some("https://bsky.app")),
                    opts.cancel_token.clone(),
                    None,
                    opts.concurrent_fragments,
                    false,
                    &[],
                    opts.audio_format.as_deref(),
                )
                .await;
            }
        }

        match info.media_type {
            MediaType::Video => {
                let filename = format!("{}.mp4", sanitize_filename::sanitize(&info.title));
                let output_path = opts.output_dir.join(&filename);
                let output_str = output_path.to_string_lossy().to_string();

                let ceiling = ceiling_of(opts.quality.as_deref());
                if let Some(original) = info
                    .available_qualities
                    .iter()
                    .find(|q| q.format == "mp4" && blob_allowed(q, ceiling))
                {
                    match direct_downloader::download_direct(
                        &self.client,
                        &original.url,
                        &output_path,
                        progress.clone(),
                        Some(&opts.cancel_token),
                    )
                    .await
                    {
                        Ok(bytes) => {
                            return Ok(DownloadResult {
                                file_path: output_path,
                                file_size_bytes: bytes,
                                duration_seconds: 0.0,
                                torrent_id: None,
                            })
                        }
                        Err(e) if opts.cancel_token.is_cancelled() => return Err(e),
                        Err(e) => {
                            tracing::warn!("[bluesky] original blob failed: {:#}, trying HLS", e);
                            let _ = tokio::fs::remove_file(&output_path).await;
                        }
                    }
                }

                let hls_url = &info
                    .available_qualities
                    .iter()
                    .find(|q| q.format != "mp4")
                    .ok_or_else(|| anyhow!("No HLS URL available"))?
                    .url;

                let downloader =
                    HlsDownloader::new().with_user_agent_override(opts.user_agent.clone());
                let _ = progress.send(ProgressUpdate::percent(0.0)).await;

                let result = downloader
                    .download_with_quality(
                        hls_url,
                        &output_str,
                        "https://bsky.app",
                        None,
                        opts.cancel_token.clone(),
                        20,
                        3,
                        ceiling,
                    )
                    .await?;

                let _ = progress.send(ProgressUpdate::percent(100.0)).await;

                Ok(DownloadResult {
                    file_path: result.path,
                    file_size_bytes: result.file_size,
                    duration_seconds: 0.0,
                    torrent_id: None,
                })
            }
            MediaType::Photo | MediaType::Carousel => {
                let mut total_bytes = 0u64;
                let count = info.available_qualities.len();
                let mut last_path = opts.output_dir.clone();

                for (i, quality) in info.available_qualities.iter().enumerate() {
                    let ext = &quality.format;
                    let filename = if count == 1 {
                        format!("{}.{}", sanitize_filename::sanitize(&info.title), ext)
                    } else {
                        format!(
                            "{}_{}.{}",
                            sanitize_filename::sanitize(&info.title),
                            i + 1,
                            ext
                        )
                    };
                    let output = opts.output_dir.join(&filename);
                    let (tx, _rx) = mpsc::channel(8);
                    let bytes = direct_downloader::download_direct(
                        &self.client,
                        &quality.url,
                        &output,
                        tx,
                        Some(&opts.cancel_token),
                    )
                    .await?;
                    total_bytes += bytes;
                    last_path = output;

                    let percent = ((i + 1) as f64 / count as f64) * 100.0;
                    let _ = progress.send(ProgressUpdate::percent(percent)).await;
                }

                Ok(DownloadResult {
                    file_path: last_path,
                    file_size_bytes: total_bytes,
                    duration_seconds: 0.0,
                    torrent_id: None,
                })
            }
            MediaType::Gif => {
                let gif_url = &info
                    .available_qualities
                    .first()
                    .ok_or_else(|| anyhow!("No GIF URL available"))?
                    .url;

                let filename = format!("{}.gif", sanitize_filename::sanitize(&info.title));
                let output = opts.output_dir.join(&filename);

                let bytes = direct_downloader::download_direct(
                    &self.client,
                    gif_url,
                    &output,
                    progress,
                    Some(&opts.cancel_token),
                )
                .await?;

                Ok(DownloadResult {
                    file_path: output,
                    file_size_bytes: bytes,
                    duration_seconds: 0.0,
                    torrent_id: None,
                })
            }
            _ => Err(anyhow!("Unsupported media type for download")),
        }
    }
}

/// getPostThread answers 400 with an XRPC error body (`NotFound` for a deleted
/// post or a handle that no longer resolves): name it so the classifier says
/// "not found" instead of a bare HTTP status.
fn thread_http_error(status: u16, body: &str) -> anyhow::Error {
    let xrpc = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error")?.as_str().map(str::to_owned));
    match xrpc.as_deref() {
        Some("NotFound") => {
            anyhow!("Post not found: it was deleted or its account no longer exists")
        }
        Some(other) => anyhow!("Bluesky API returned HTTP {status} ({other})"),
        None => anyhow!("Bluesky API returned HTTP {status}"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn thread_not_found_names_a_deleted_post() {
        let e = super::thread_http_error(400, r#"{"error":"NotFound","message":"Post not found"}"#)
            .to_string();
        assert!(e.contains("not found"), "{e}");
        assert_eq!(
            crate::core::errors::classify_download_error(&e).0,
            "not_found"
        );
        let e = super::thread_http_error(502, "<html>").to_string();
        assert!(e.contains("HTTP 502"), "{e}");
    }

    use super::*;
    use serde_json::json;

    const PLAYLIST: &str = "https://video.bsky.app/watch/did%3Aplc%3A7x6rtuenkuvxq3zsvffp2ide/bafkreielhgekjheckgjusx7x5hxkbrqryfdmzdwwp2zoxchovgnpzkxzae/playlist.m3u8";

    #[test]
    fn did_filter_accepts_only_plc_and_web() {
        assert!(is_did("did:plc:7x6rtuenkuvxq3zsvffp2ide"));
        assert!(is_did("did:web:de1.tentacle.expert"));
        assert!(!is_did("cinny.bun.how"));
        assert!(!is_did("did:plc:x/../y"));
    }

    #[tokio::test]
    async fn did_input_is_not_resolved_again() {
        let d = BlueskyDownloader::new();
        assert_eq!(d.resolve_did("did:plc:abc").await, "did:plc:abc");
    }

    #[test]
    fn blob_ref_comes_from_playlist() {
        let (did, cid) = blob_ref_from_playlist(PLAYLIST).unwrap();
        assert_eq!(did, "did:plc:7x6rtuenkuvxq3zsvffp2ide");
        assert_eq!(
            cid,
            "bafkreielhgekjheckgjusx7x5hxkbrqryfdmzdwwp2zoxchovgnpzkxzae"
        );
        assert!(blob_ref_from_playlist("https://example.com/x.m3u8").is_none());
    }

    #[test]
    fn did_doc_and_pds_endpoint() {
        assert_eq!(
            did_doc_url("did:plc:abc").as_deref(),
            Some("https://plc.directory/did:plc:abc")
        );
        assert_eq!(
            did_doc_url("did:web:de1.tentacle.expert").as_deref(),
            Some("https://de1.tentacle.expert/.well-known/did.json")
        );
        let doc = json!({"service":[{"id":"#atproto_pds","type":"AtprotoPersonalDataServer","serviceEndpoint":"https://pds2.bun.how/"}]});
        assert_eq!(
            pds_from_did_doc(&doc).as_deref(),
            Some("https://pds2.bun.how")
        );
        assert!(pds_from_did_doc(
            &json!({"service":[{"id":"#atproto_pds","serviceEndpoint":"http://x"}]})
        )
        .is_none());
        assert_eq!(
            blob_url("https://pds2.bun.how", "did:plc:abc", "bafk"),
            "https://pds2.bun.how/xrpc/com.atproto.sync.getBlob?did=did%3Aplc%3Aabc&cid=bafk"
        );
    }

    /// Quote posts (corpus bluesky-3) carry the video inside the quoted record.
    #[test]
    fn quoted_video_is_extracted() {
        let embed = json!({"$type":"app.bsky.embed.record#view","record":{"$type":"app.bsky.embed.record#viewRecord","embeds":[{"$type":"app.bsky.embed.video#view","playlist":PLAYLIST}]}});
        match extract_media(&embed) {
            Some(BlueskyMedia::Video {
                hls_url,
                blob,
                height,
            }) => {
                assert!(hls_url.contains("video.cdn.bsky.app/hls/"));
                assert!(blob.is_some());
                assert_eq!(height, 0);
            }
            _ => panic!("no video"),
        }
    }

    #[test]
    fn original_blob_precedes_hls() {
        let q = video_qualities(
            Some("https://pds/xrpc/com.atproto.sync.getBlob?did=d&cid=c".into()),
            1080,
            "https://video.cdn.bsky.app/hls/x/playlist.m3u8".into(),
        );
        assert_eq!(q[0].format, "mp4");
        assert_eq!(q[1].format, "hls");
        assert_eq!(q[0].height, 1080);
        assert_eq!(q[1].height, 0);
        let only = video_qualities(None, 0, "h".into());
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].format, "hls");
    }

    /// Round-1 regression (26/09): bluesky-1/-3 are 1920x1080 originals; the
    /// blob was downloaded and refused at maxHeight=720. The fixture is the
    /// real getPostThread embed; the HLS master has 360p and 720p.
    #[test]
    fn original_above_ceiling_is_skipped_for_hls() {
        let embed = json!({"$type":"app.bsky.embed.record#view","record":{"embeds":[{"$type":"app.bsky.embed.video#view","playlist":PLAYLIST,"aspectRatio":{"height":1080,"width":1920}}]}});
        let Some(BlueskyMedia::Video { height, .. }) = extract_media(&embed) else {
            panic!("no video")
        };
        assert_eq!(height, 1080);
        let q = video_qualities(Some("https://pds/blob".into()), height, "h".into());
        assert_eq!(ceiling_of(Some("720")), Some(720));
        assert_eq!(ceiling_of(Some("720p")), Some(720));
        assert!(!blob_allowed(&q[0], ceiling_of(Some("720"))));
        // Unknown size is not proof of fitting either.
        let no_size = video_qualities(Some("b".into()), 0, "h".into());
        assert!(!blob_allowed(&no_size[0], Some(720)));
        assert!(blob_allowed(
            &video_qualities(Some("b".into()), 720, "h".into())[0],
            Some(720)
        ));
        assert!(blob_allowed(&q[0], ceiling_of(None)));
        // A reduced ratio is not a pixel size.
        assert_eq!(
            original_height(&json!({"aspectRatio":{"width":16,"height":9}})),
            0
        );
        assert_eq!(original_height(&json!({})), 0);
    }

    #[test]
    fn fallback_error_keeps_both_causes() {
        let e = chain_fallback(
            anyhow!("HTTP 400"),
            anyhow!("yt-dlp: Unable to resolve handle"),
        );
        let s = format!("{e:#}");
        assert!(s.contains("HTTP 400"), "{s}");
        assert!(s.contains("Unable to resolve handle"), "{s}");
    }

    /// Live: handle-form at:// URI answers 400 for this custom-domain handle.
    #[tokio::test]
    #[ignore]
    async fn live_custom_domain_handle_post_resolves() {
        let d = BlueskyDownloader::new();
        let info = d
            .native_get_media_info("https://bsky.app/profile/cinny.bun.how/post/3l7rdfxhyds2f")
            .await
            .unwrap();
        assert_eq!(info.available_qualities[0].format, "mp4");
        assert!(info.available_qualities[0].url.contains("getBlob"));
    }
}
