use std::path::PathBuf;

use omniget_core::models::progress::ProgressUpdate;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::api::{ApiClient, BilibiliError, Result};
use super::parser::{ContentMetadata, EpisodeItem, ParsedContent};
use super::preview::{self, AudioStream, MediaContainer, PreviewInfo, VideoStream};
use super::url_kind::UrlKind;

pub mod fetch;
pub mod mux;
pub mod query;

pub struct EngineOptions {
    pub output_dir: PathBuf,
    pub container: mux::Container,
    pub video_qn_pref: u32,
    pub video_codec_pref: u32,
    pub audio_qn_pref: u32,
    pub embed_cover: bool,
    pub keep_streams: bool,
    pub filename: String,
    pub cancel: CancellationToken,
    pub danmaku_enabled: bool,
    pub danmaku_format: Option<super::danmaku::DanmakuFormat>,
    pub nfo_enabled: bool,
    pub cover_sidecar_enabled: bool,
    pub cover_format: super::cover::CoverFormat,
    pub cdn_alt_hosts: Vec<String>,
    pub cdn_prefer_alternatives: bool,
}

pub struct EngineResult {
    pub final_path: PathBuf,
    pub bytes: u64,
}

pub async fn run_single(
    client: &ApiClient,
    item: &EpisodeItem,
    kind: &UrlKind,
    opts: &EngineOptions,
    metadata: Option<&ContentMetadata>,
    progress: mpsc::Sender<ProgressUpdate>,
) -> Result<EngineResult> {
    let _ = progress.send(ProgressUpdate::percent(0.0)).await;
    let preview_info: PreviewInfo = preview::fetch(client, item, kind).await?;

    if preview_info.container != MediaContainer::Dash {
        return run_progressive(client, item, &preview_info, opts, progress).await;
    }

    let video = preview_info
        .pick_video(opts.video_qn_pref, opts.video_codec_pref)
        .ok_or(BilibiliError::ContentUnavailable)?;
    let audio = preview_info.pick_audio(opts.audio_qn_pref);

    let _ = progress.send(ProgressUpdate::percent(2.0)).await;
    let cdn_prefs = query::CdnPreferences {
        alt_hosts: opts.cdn_alt_hosts.clone(),
        prefer_alternatives: opts.cdn_prefer_alternatives,
    };
    let video_url =
        query::resolve_best_url_with_cdn(client, &video.base_url, &video.backup_urls, &cdn_prefs)
            .await?;
    let audio_resolved = if let Some(a) = audio {
        Some(
            query::resolve_best_url_with_cdn(client, &a.base_url, &a.backup_urls, &cdn_prefs)
                .await?,
        )
    } else {
        None
    };

    std::fs::create_dir_all(&opts.output_dir).ok();
    let temp_stem = sanitize_for_temp(&opts.filename);

    let temp_video = opts.output_dir.join(format!(".{}.video.tmp", temp_stem));
    let temp_audio = opts.output_dir.join(format!(".{}.audio.tmp", temp_stem));
    let temp_cover = opts.output_dir.join(format!(".{}.cover.tmp", temp_stem));

    let (ua, cookie_header) = (
        client.user_agent().to_string(),
        client.cookie_header().map(|s| s.to_string()),
    );
    let referer = item
        .url
        .clone()
        .unwrap_or_else(|| "https://www.bilibili.com".to_string());

    let total_streams = 1 + audio_resolved.as_ref().map(|_| 1).unwrap_or(0);

    let (video_tx, mut video_rx) = mpsc::channel::<ProgressUpdate>(32);
    let progress_video = progress.clone();
    let agg_total_video = video_url.size;
    let agg_total_audio = audio_resolved.as_ref().map(|a| a.size).unwrap_or(0);
    let total_size_hint = agg_total_video + agg_total_audio;
    tokio::spawn(async move {
        while let Some(p) = video_rx.recv().await {
            let scaled = scale_progress(p.percent, 0, total_streams);
            let _ = progress_video
                .send(ProgressUpdate::rich(
                    scaled,
                    p.downloaded_bytes,
                    Some(total_size_hint.max(1)),
                    p.speed_bps,
                    p.eta_seconds,
                ))
                .await;
        }
    });

    let video_handle = {
        let cancel = opts.cancel.clone();
        let url = video_url.url.clone();
        let path = temp_video.clone();
        let referer = referer.clone();
        let ua = ua.clone();
        let cookie = cookie_header.clone();
        tokio::spawn(async move {
            fetch::fetch_stream(
                fetch::FetchOptions {
                    url: &url,
                    output_path: &path,
                    referer: &referer,
                    user_agent: &ua,
                    cookie_header: cookie.as_deref(),
                    cancel: Some(cancel),
                },
                video_tx,
            )
            .await
        })
    };

    let audio_handle = if let Some(audio_res) = audio_resolved.clone() {
        let (audio_tx, mut audio_rx) = mpsc::channel::<ProgressUpdate>(32);
        let progress_audio = progress.clone();
        tokio::spawn(async move {
            while let Some(p) = audio_rx.recv().await {
                let scaled = scale_progress(p.percent, 1, total_streams);
                let _ = progress_audio
                    .send(ProgressUpdate::rich(
                        scaled,
                        p.downloaded_bytes,
                        Some(total_size_hint.max(1)),
                        p.speed_bps,
                        p.eta_seconds,
                    ))
                    .await;
            }
        });
        let cancel = opts.cancel.clone();
        let url = audio_res.url.clone();
        let path = temp_audio.clone();
        let referer = referer.clone();
        let ua = ua.clone();
        let cookie = cookie_header.clone();
        Some(tokio::spawn(async move {
            fetch::fetch_stream(
                fetch::FetchOptions {
                    url: &url,
                    output_path: &path,
                    referer: &referer,
                    user_agent: &ua,
                    cookie_header: cookie.as_deref(),
                    cancel: Some(cancel),
                },
                audio_tx,
            )
            .await
        }))
    } else {
        None
    };

    let video_result = video_handle
        .await
        .map_err(|_| BilibiliError::ContentUnavailable)??;
    let audio_result = match audio_handle {
        Some(h) => Some(h.await.map_err(|_| BilibiliError::ContentUnavailable)??),
        None => None,
    };

    if opts.cancel.is_cancelled() {
        cleanup(&[&temp_video, &temp_audio, &temp_cover]);
        return Err(BilibiliError::Cancelled);
    }

    let cover_path_opt = if opts.embed_cover {
        if let Some(cover_url) = item.cover_url.as_deref() {
            download_cover(client, cover_url, &temp_cover).await.ok()
        } else {
            None
        }
    } else {
        None
    };

    let _ = progress.send(ProgressUpdate::percent(95.0)).await;

    let final_path =
        opts.output_dir
            .join(format!("{}.{}", opts.filename, opts.container.extension()));
    if let Some(parent) = final_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    if audio_result.is_some() {
        mux::mux(mux::MuxInputs {
            video: &temp_video,
            audio: &temp_audio,
            cover: cover_path_opt.as_deref(),
            output: &final_path,
            container: opts.container,
        })
        .await?;
    } else {
        std::fs::rename(&temp_video, &final_path).map_err(|_| BilibiliError::ContentUnavailable)?;
    }

    if !opts.keep_streams {
        cleanup(&[&temp_video, &temp_audio, &temp_cover]);
    }

    let folder_for_sidecars = final_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| opts.output_dir.clone());
    let file_stem_for_sidecars = final_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| opts.filename.clone());

    if opts.danmaku_enabled {
        let cid = item.cid;
        let duration = item.duration_seconds.unwrap_or(0.0).max(0.0) as u64;
        if let (Some(cid_v), format) = (
            cid,
            opts.danmaku_format
                .unwrap_or(super::danmaku::DanmakuFormat::Xml),
        ) {
            if duration > 0 {
                match super::danmaku::fetch_elems(client, cid_v, duration).await {
                    Ok(elems) => {
                        let rendered = super::danmaku::render(&elems, format);
                        let dm_path = folder_for_sidecars.join(format!(
                            "{}.danmaku.{}",
                            file_stem_for_sidecars,
                            format.extension()
                        ));
                        if let Err(e) = std::fs::write(&dm_path, rendered) {
                            tracing::warn!("[bilibili] failed to write danmaku: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[bilibili] danmaku fetch failed: {:?}", e);
                    }
                }
            }
        }
    }

    if opts.cover_sidecar_enabled {
        if let Some(cover_url) = item.cover_url.as_deref() {
            let cover_path = folder_for_sidecars.join(format!(
                "{}.{}",
                file_stem_for_sidecars,
                opts.cover_format.extension()
            ));
            if let Err(e) =
                super::cover::download_to(client, cover_url, &cover_path, opts.cover_format).await
            {
                tracing::warn!("[bilibili] cover sidecar failed: {:?}", e);
            }
        }
    }

    if opts.nfo_enabled {
        if let Some(meta_ref) = metadata {
            let folder_for_extras = final_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| opts.output_dir.clone());
            let file_stem = final_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| opts.filename.clone());
            write_nfo_files(item, meta_ref, kind, client, &folder_for_extras, &file_stem).await;
        }
    }

    let bytes = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);

    let _ = progress.send(ProgressUpdate::percent(100.0)).await;

    let _ = video_result;
    Ok(EngineResult { final_path, bytes })
}

pub async fn run_parsed_content(
    client: &ApiClient,
    parsed: &ParsedContent,
    kind: &UrlKind,
    opts: &EngineOptions,
    progress: mpsc::Sender<ProgressUpdate>,
) -> Result<EngineResult> {
    let first = parsed
        .items
        .first()
        .ok_or(BilibiliError::ContentUnavailable)?;
    run_single(client, first, kind, opts, Some(&parsed.metadata), progress).await
}

async fn write_nfo_files(
    item: &EpisodeItem,
    metadata: &ContentMetadata,
    kind: &UrlKind,
    client: &ApiClient,
    output_dir: &std::path::Path,
    filename: &str,
) {
    use super::nfo;
    let nfo_kind = match nfo::classify(kind) {
        Some(k) => k,
        None => return,
    };

    let mut tags: Vec<String> = Vec::new();
    if nfo_kind == nfo::NfoKind::Movie {
        if let Some(b) = item.bvid.as_deref() {
            tags = nfo::fetch_tags(client, b).await;
        }
    }

    let xml = nfo::render(nfo_kind, item, metadata, &tags);
    let path = match nfo_kind {
        nfo::NfoKind::Movie => output_dir.join(format!("{}.nfo", filename)),
        nfo::NfoKind::Episode => output_dir.join(format!("{}.nfo", filename)),
        nfo::NfoKind::TvShow => output_dir.join("tvshow.nfo"),
    };
    if let Err(e) = std::fs::write(&path, xml) {
        tracing::warn!("[bilibili] nfo write failed: {}", e);
    }

    if nfo::season_uses_tvshow(kind) {
        let tvshow_path = output_dir.join("tvshow.nfo");
        if !tvshow_path.exists() {
            let tvshow_xml = nfo::render(nfo::NfoKind::TvShow, item, metadata, &[]);
            if let Err(e) = std::fs::write(&tvshow_path, tvshow_xml) {
                tracing::warn!("[bilibili] tvshow nfo write failed: {}", e);
            }
        }
    }
}

async fn run_progressive(
    client: &ApiClient,
    item: &EpisodeItem,
    info: &PreviewInfo,
    opts: &EngineOptions,
    progress: mpsc::Sender<ProgressUpdate>,
) -> Result<EngineResult> {
    let stream: &VideoStream = info
        .video_streams
        .first()
        .ok_or(BilibiliError::ContentUnavailable)?;
    let cdn_prefs = query::CdnPreferences {
        alt_hosts: opts.cdn_alt_hosts.clone(),
        prefer_alternatives: opts.cdn_prefer_alternatives,
    };
    let resolved =
        query::resolve_best_url_with_cdn(client, &stream.base_url, &stream.backup_urls, &cdn_prefs)
            .await?;
    let final_path =
        opts.output_dir
            .join(format!("{}.{}", opts.filename, opts.container.extension()));
    let referer = item
        .url
        .clone()
        .unwrap_or_else(|| "https://www.bilibili.com".to_string());
    let (ua, cookie_header) = (
        client.user_agent().to_string(),
        client.cookie_header().map(|s| s.to_string()),
    );
    let result = fetch::fetch_stream(
        fetch::FetchOptions {
            url: &resolved.url,
            output_path: &final_path,
            referer: &referer,
            user_agent: &ua,
            cookie_header: cookie_header.as_deref(),
            cancel: Some(opts.cancel.clone()),
        },
        progress.clone(),
    )
    .await?;
    let _ = progress.send(ProgressUpdate::percent(100.0)).await;
    Ok(EngineResult {
        final_path,
        bytes: result.bytes_written,
    })
}

async fn download_cover(
    client: &ApiClient,
    cover_url: &str,
    out_path: &std::path::Path,
) -> Result<std::path::PathBuf> {
    let bytes = client.get_bytes(cover_url).await?;
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(out_path, bytes).map_err(|_| BilibiliError::ContentUnavailable)?;
    Ok(out_path.to_path_buf())
}

fn cleanup(paths: &[&std::path::Path]) {
    for p in paths {
        let _ = std::fs::remove_file(p);
    }
}

fn sanitize_for_temp(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .take(80)
        .collect()
}

fn scale_progress(local_pct: f64, stream_index: u32, total_streams: u32) -> f64 {
    let denom = total_streams.max(1) as f64;
    let base = stream_index as f64 / denom;
    let scaled = base + (local_pct / 100.0) / denom;
    (scaled * 100.0).clamp(0.0, 95.0)
}

#[allow(dead_code)]
fn unused_audio_stream(_: &AudioStream) {}
