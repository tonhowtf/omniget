use std::sync::Arc;

use serde::Serialize;

use crate::core::queue::{self, emit_queue_state_from_state};
use crate::core::url_parser;
use crate::platforms::Platform;
use crate::storage::config;
use crate::AppState;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;
#[cfg(not(target_os = "android"))]
use crate::models::media::{FormatInfo, MediaType};

#[derive(Clone, Serialize)]
pub struct PlatformInfo {
    pub platform: String,
    pub supported: bool,
    pub content_id: Option<String>,
    pub content_type: Option<String>,
}

#[tauri::command]
pub fn check_cookie_error() -> bool {
    let has_error = crate::core::ytdlp::has_cookie_error();
    if has_error {
        crate::core::ytdlp::clear_cookie_error();
    }
    has_error
}

#[derive(Clone, Serialize)]
pub struct PathLimitInfo {
    pub limit: usize,
    pub current: usize,
    pub reserve: usize,
    pub ok: bool,
}

#[tauri::command]
pub fn validate_output_path(output_dir: String) -> PathLimitInfo {
    match crate::core::path_limits::validate_output_dir(&output_dir) {
        Ok(()) => PathLimitInfo {
            limit: crate::core::path_limits::MAX_PATH_LEN,
            current: output_dir.chars().count() + crate::core::path_limits::SEPARATOR_RESERVE,
            reserve: crate::core::path_limits::MIN_FILENAME_RESERVE,
            ok: true,
        },
        Err(err) => PathLimitInfo {
            limit: err.limit,
            current: err.current,
            reserve: err.reserve,
            ok: false,
        },
    }
}

#[tauri::command]
pub async fn detect_platform(url: String) -> Result<PlatformInfo, String> {
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    let _timer_start = std::time::Instant::now();
    match Platform::from_url(&url) {
        Some(platform) => {
            let parsed = url_parser::parse_url(&url);
            let platform_name = platform.to_string();
            let content_type = if platform_name == "direct_file" {
                Some("file".to_string())
            } else {
                parsed
                    .as_ref()
                    .map(|p| format!("{:?}", p.content_type).to_lowercase())
            };
            let result = Ok(PlatformInfo {
                platform: platform_name,
                supported: true,
                content_id: parsed.as_ref().and_then(|p| p.content_id.clone()),
                content_type,
            });
            tracing::debug!("[perf] detect_platform took {:?}", _timer_start.elapsed());
            result
        }
        None => {
            let is_valid_url = url::Url::parse(&url)
                .map(|u| u.scheme() == "http" || u.scheme() == "https")
                .unwrap_or(false);
            if is_valid_url
                && omniget_core::platforms::direct_file::looks_like_direct_file(&url).await
            {
                tracing::debug!("[perf] detect_platform took {:?}", _timer_start.elapsed());
                return Ok(PlatformInfo {
                    platform: "direct_file".to_string(),
                    supported: true,
                    content_id: None,
                    content_type: Some("file".to_string()),
                });
            }
            let result = Ok(PlatformInfo {
                platform: if is_valid_url {
                    "generic".to_string()
                } else {
                    "unknown".to_string()
                },
                supported: is_valid_url,
                content_id: None,
                content_type: None,
            });
            tracing::debug!("[perf] detect_platform took {:?}", _timer_start.elapsed());
            result
        }
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn get_media_formats(url: String) -> Result<Vec<FormatInfo>, String> {
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    let _timer_start = std::time::Instant::now();
    let ytdlp_path = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {}", e))?;

    let json = ytdlp::get_video_info(&ytdlp_path, &url, &[])
        .await
        .map_err(|e| format!("Failed to get formats: {}", e))?;

    tracing::debug!("[perf] get_media_formats took {:?}", _timer_start.elapsed());
    Ok(ytdlp::parse_formats(&json))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn prefetch_media_info(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
) -> Result<(), String> {
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    let settings = config::load_settings(&app);
    crate::core::http_client::init_proxy(settings.proxy);

    let resolved = match crate::core::url_resolver::resolve_downloader(&state.registry, &url).await
    {
        Some(r) => r,
        None => return Err("No downloader available".to_string()),
    };
    let downloader = resolved.downloader;
    let platform_name = resolved.platform_name;

    let ytdlp_path = ytdlp::find_ytdlp_cached().await;

    tokio::spawn(async move {
        queue::prefetch_info_with_emit(
            &url,
            &*downloader,
            &platform_name,
            ytdlp_path.as_deref(),
            Some(app),
        )
        .await;
    });

    Ok(())
}

#[derive(Clone, Serialize)]
pub struct DownloadStarted {
    pub id: u64,
    pub title: String,
}

fn is_valid_time_range(r: &str) -> bool {
    let Some((a, b)) = r.split_once('-') else {
        return false;
    };
    let part_ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_digit() || c == ':' || c == '.')
    };
    part_ok(a) && (b == "inf" || part_ok(b))
}

/// yt-dlp flags for a subtitle language list such as `en` or `en,pt-BR`.
/// Anything that is not a plain language list is ignored, so the value can
/// never smuggle another flag in.
fn subtitle_lang_args(langs: &str) -> Option<Vec<String>> {
    let langs = langs.trim();
    let valid = !langs.is_empty()
        && langs.len() <= 64
        && !langs.starts_with('-')
        && langs
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ',' | '-' | '_' | '.'));
    valid.then(|| {
        vec![
            "--write-sub".to_string(),
            "--write-auto-sub".to_string(),
            "--sub-lang".to_string(),
            langs.to_string(),
            "--convert-subs".to_string(),
            "srt".to_string(),
        ]
    })
}

#[derive(Clone, Serialize)]
pub struct PlaylistEntryInfo {
    pub index: u32,
    pub title: String,
    pub url: String,
}

#[derive(Clone, Serialize)]
pub struct TorrentFileEntry {
    pub index: usize,
    pub path: String,
    pub size_bytes: u64,
}

#[cfg(not(target_os = "android"))]
fn is_torrent_source(url: &str) -> bool {
    url.starts_with("magnet:")
        || url.ends_with(".torrent")
        || (std::path::Path::new(url).exists()
            && std::path::Path::new(url)
                .extension()
                .map(|e| e == "torrent")
                .unwrap_or(false))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn torrent_contents(url: String) -> Result<Vec<TorrentFileEntry>, String> {
    if !is_torrent_source(&url) {
        return Err("Not a torrent source".to_string());
    }

    let add_torrent = if url.starts_with("magnet:")
        || url.starts_with("http://")
        || url.starts_with("https://")
    {
        librqbit::AddTorrent::from_url(&url)
    } else {
        let path = std::path::Path::new(&url);
        if path.exists() && path.extension().map(|e| e == "torrent").unwrap_or(false) {
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|e| format!("Failed to read .torrent file: {}", e))?;
            librqbit::AddTorrent::from_bytes(bytes)
        } else {
            librqbit::AddTorrent::from_url(&url)
        }
    };

    let tmp_dir = std::env::temp_dir().join("omniget-torrent-list");
    let session = librqbit::Session::new(tmp_dir)
        .await
        .map_err(|e| format!("Failed to init torrent session: {}", e))?;

    let opts = librqbit::AddTorrentOptions {
        list_only: true,
        ..Default::default()
    };

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        session.add_torrent(add_torrent, Some(opts)),
    )
    .await
    .map_err(|_| "Timed out resolving torrent metadata".to_string())?
    .map_err(|e| format!("Failed to resolve torrent: {}", e))?;

    let list = match resp {
        librqbit::AddTorrentResponse::ListOnly(l) => l,
        _ => return Err("Torrent did not return a file listing".to_string()),
    };

    let entries: Vec<TorrentFileEntry> = match list.info.iter_file_details() {
        Ok(iter) => iter
            .enumerate()
            .map(|(index, d)| TorrentFileEntry {
                index,
                path: d
                    .filename
                    .to_string()
                    .unwrap_or_else(|_| format!("file {}", index + 1)),
                size_bytes: d.len,
            })
            .collect(),
        Err(e) => return Err(format!("Failed to read torrent files: {}", e)),
    };

    Ok(entries)
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn playlist_entries(url: String) -> Result<Vec<PlaylistEntryInfo>, String> {
    let ytdlp_path = ytdlp::find_ytdlp_cached()
        .await
        .ok_or_else(|| "yt-dlp unavailable".to_string())?;
    let (_title, entries) = ytdlp::get_playlist_info(&ytdlp_path, &url, &[])
        .await
        .map_err(|e| e.to_string())?;
    Ok(entries
        .into_iter()
        .enumerate()
        .map(|(i, e)| PlaylistEntryInfo {
            index: (i + 1) as u32,
            title: e.title,
            url: e.url,
        })
        .collect())
}

#[cfg(not(target_os = "android"))]
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn download_from_url(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
    output_dir: String,
    download_mode: Option<String>,
    quality: Option<String>,
    format_id: Option<String>,
    referer: Option<String>,
    cookie_slug: Option<String>,
    time_range: Option<String>,
    playlist_items: Option<Vec<u32>>,
    torrent_files: Option<Vec<usize>>,
    scheduled_at: Option<u64>,
    stop_at: Option<u64>,
    subtitle_lang: Option<String>,
    engine: Option<String>,
) -> Result<DownloadStarted, String> {
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    let _timer_start = std::time::Instant::now();
    let mut custom_ytdlp_args = match time_range.as_deref().map(str::trim) {
        Some(r) if !r.is_empty() && is_valid_time_range(r) => {
            Some(vec!["--download-sections".to_string(), format!("*{}", r)])
        }
        _ => None,
    };
    // Per-download subtitles (Downloads › Tools › Smart mode presets), on top
    // of the global subtitle setting.
    if let Some(flags) = subtitle_lang.as_deref().and_then(subtitle_lang_args) {
        custom_ytdlp_args.get_or_insert_with(Vec::new).extend(flags);
    }

    if let Err(err) = crate::core::path_limits::validate_output_dir(&output_dir) {
        return Err(format!(
            "PathTooLong|{}|{}|{}",
            err.limit, err.current, err.reserve
        ));
    }

    let mut download_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let download_queue = state.download_queue.clone();

    {
        let settings = config::load_settings(&app);
        crate::core::http_client::init_proxy(settings.proxy.clone());
        crate::core::http_fetcher::set_global_max_concurrent_segments(
            settings.advanced.max_concurrent_segments as usize,
        );
        let mut q = download_queue.lock().await;
        q.max_concurrent = settings.advanced.max_concurrent_downloads.max(1);
        q.stagger_delay_ms = settings.advanced.stagger_delay_ms;
        q.default_max_retries = settings.advanced.max_retries;
        if q.has_url(&url) {
            tracing::debug!("[perf] download_from_url took {:?}", _timer_start.elapsed());
            return Err("Download already in progress for this URL".to_string());
        }
        download_id = q.next_available_id(download_id);
    }

    let resolved = match crate::core::url_resolver::resolve_downloader(&state.registry, &url).await
    {
        Some(r) => r,
        None => {
            tracing::debug!("[perf] download_from_url took {:?}", _timer_start.elapsed());
            return Err("No downloader available for this URL".to_string());
        }
    };
    // Downloads › Tools › Engine per link: the person picked the engine.
    let (downloader, platform_name) =
        match crate::commands::link_router::forced_engine(&state.registry, engine.as_deref())? {
            Some(forced) => forced,
            None => (resolved.downloader, resolved.platform_name),
        };
    // Placeholder until metadata arrives: never the raw URL, which can carry
    // a token into history, events and the extension (N-3).
    let title = crate::core::flight_recorder::redact_url(&url);
    let ytdlp_path = ytdlp::find_ytdlp_cached().await;

    let cached_info = {
        let info = queue::try_get_cached_info(&url).await;
        match (info, &playlist_items) {
            (Some(mut info), Some(sel))
                if !sel.is_empty() && info.media_type == MediaType::Playlist =>
            {
                let set: std::collections::HashSet<u32> = sel.iter().copied().collect();
                let filtered: Vec<_> = info
                    .available_qualities
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| set.contains(&((*i as u32) + 1)))
                    .map(|(_, q)| q.clone())
                    .collect();
                if !filtered.is_empty() {
                    info.available_qualities = filtered;
                }
                Some(info)
            }
            (info, _) => info,
        }
    };

    let state_to_emit = {
        let mut q = download_queue.lock().await;
        q.enqueue(
            download_id,
            url,
            platform_name,
            title.clone(),
            output_dir,
            download_mode,
            quality,
            format_id,
            referer,
            None,
            None,
            None,
            cached_info,
            None,
            None,
            downloader,
            ytdlp_path,
            false,
            cookie_slug,
            custom_ytdlp_args,
            torrent_files,
            scheduled_at,
            stop_at,
        );

        let next_ids = q.next_queued_ids();
        for nid in &next_ids {
            q.mark_active(*nid);
        }
        q.get_state()
    };
    emit_queue_state_from_state(&app, state_to_emit);

    let q_clone = download_queue.clone();
    let app_clone = app.clone();
    tokio::spawn(async move {
        let ids_to_start = {
            let q = q_clone.lock().await;
            q.items
                .iter()
                .filter(|i| i.status == queue::QueueStatus::Active)
                .filter(|i| i.id == download_id)
                .map(|i| i.id)
                .collect::<Vec<_>>()
        };

        let stagger = {
            let q = q_clone.lock().await;
            q.stagger_delay_ms
        };

        for (i, nid) in ids_to_start.into_iter().enumerate() {
            if i > 0 && stagger > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(stagger)).await;
            }
            let a = app_clone.clone();
            let qc = q_clone.clone();
            tokio::spawn(async move {
                queue::spawn_download(a, qc, nid).await;
            });
        }
    });

    tracing::debug!("[perf] download_from_url took {:?}", _timer_start.elapsed());
    Ok(DownloadStarted {
        id: download_id,
        title,
    })
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn download_with_custom_args(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
    output_dir: String,
    custom_args: Vec<String>,
    cookie_slug: Option<String>,
) -> Result<DownloadStarted, String> {
    if url.trim().is_empty() {
        return Err("URL is required".to_string());
    }
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    if let Err(err) = crate::core::path_limits::validate_output_dir(&output_dir) {
        return Err(format!(
            "PathTooLong|{}|{}|{}",
            err.limit, err.current, err.reserve
        ));
    }

    let mut download_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let download_queue = state.download_queue.clone();
    {
        let settings = config::load_settings(&app);
        let mut q = download_queue.lock().await;
        q.max_concurrent = settings.advanced.max_concurrent_downloads.max(1);
        q.stagger_delay_ms = settings.advanced.stagger_delay_ms;
        q.default_max_retries = settings.advanced.max_retries;
        if q.has_url(&url) {
            return Err("Download already in progress for this URL".to_string());
        }
        download_id = q.next_available_id(download_id);
    }

    let downloader: Arc<dyn crate::platforms::traits::PlatformDownloader> =
        Arc::new(crate::platforms::generic_ytdlp::GenericYtdlpDownloader::new());

    // Placeholder until metadata arrives: never the raw URL, which can carry
    // a token into history, events and the extension (N-3).
    let title = crate::core::flight_recorder::redact_url(&url);
    let ytdlp_path = ytdlp::find_ytdlp_cached().await;

    let state_to_emit = {
        let mut q = download_queue.lock().await;
        q.enqueue(
            download_id,
            url,
            "generic".to_string(),
            title.clone(),
            output_dir,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            downloader,
            ytdlp_path,
            false,
            cookie_slug,
            Some(custom_args),
            None,
            None,
            None,
        );
        let next_ids = q.next_queued_ids();
        for nid in &next_ids {
            q.mark_active(*nid);
        }
        q.get_state()
    };
    emit_queue_state_from_state(&app, state_to_emit);

    let q_clone = download_queue.clone();
    let app_clone = app.clone();
    tokio::spawn(async move {
        let ids_to_start = {
            let q = q_clone.lock().await;
            q.items
                .iter()
                .filter(|i| i.status == queue::QueueStatus::Active)
                .filter(|i| i.id == download_id)
                .map(|i| i.id)
                .collect::<Vec<_>>()
        };
        for nid in ids_to_start {
            let a = app_clone.clone();
            let qc = q_clone.clone();
            tokio::spawn(async move {
                queue::spawn_download(a, qc, nid).await;
            });
        }
    });

    Ok(DownloadStarted {
        id: download_id,
        title,
    })
}

#[tauri::command]
pub async fn cancel_generic_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
) -> Result<String, String> {
    let (state_to_emit, seeding_torrent_id) = {
        let mut q = state.download_queue.lock().await;
        let (cancelled, torrent_id) = q.cancel(download_id);
        if cancelled {
            (Some(q.get_state()), torrent_id)
        } else {
            (None, None)
        }
    };
    let external = state
        .download_queue
        .lock()
        .await
        .items
        .iter()
        .any(|i| i.id == download_id && i.platform == "mcp_worker");
    if let Some(tid) = seeding_torrent_id {
        if let Some(session) = state.torrent_session.lock().await.as_ref() {
            let _ = session
                .delete(librqbit::api::TorrentIdOrHash::Id(tid), false)
                .await;
        }
    }
    if let Some(s) = state_to_emit {
        // An external job cancelled while paused: its attempt was settled as
        // paused with partials kept; settle it as cancelled and clean (D-06).
        if external {
            let _ = tokio::task::spawn_blocking(move || {
                crate::mcp::download_intents::cancel_paused(download_id)
            })
            .await;
        }
        emit_queue_state_from_state(&app, s);
        queue::try_start_next(app, state.download_queue.clone()).await;
        Ok("Download cancelled".to_string())
    } else {
        Err("No active download for this ID".to_string())
    }
}

#[tauri::command]
pub async fn pause_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
) -> Result<String, String> {
    let (state_to_emit, torrent_id) = {
        let mut q = state.download_queue.lock().await;
        if q.pause(download_id) {
            let tid = q
                .items
                .iter()
                .find(|i| i.id == download_id)
                .and_then(|i| i.torrent_id);
            (Some(q.get_state()), tid)
        } else {
            (None, None)
        }
    };
    if let Some(tid) = torrent_id {
        if let Some(session) = state.torrent_session.lock().await.as_ref() {
            if let Some(handle) = session.get(librqbit::api::TorrentIdOrHash::Id(tid)) {
                let _ = session.pause(&handle).await;
            }
        }
    }
    if let Some(s) = state_to_emit {
        emit_queue_state_from_state(&app, s);
        queue::try_start_next(app, state.download_queue.clone()).await;
        Ok("Download paused".to_string())
    } else {
        Err("Download cannot be paused".to_string())
    }
}

#[tauri::command]
pub async fn resume_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
) -> Result<String, String> {
    let (state_to_emit, torrent_id) = {
        let mut q = state.download_queue.lock().await;
        if q.resume(download_id) {
            let tid = q
                .items
                .iter()
                .find(|i| i.id == download_id)
                .and_then(|i| i.torrent_id);
            (Some(q.get_state()), tid)
        } else {
            (None, None)
        }
    };
    if let Some(tid) = torrent_id {
        if let Some(session) = state.torrent_session.lock().await.as_ref() {
            if let Some(handle) = session.get(librqbit::api::TorrentIdOrHash::Id(tid)) {
                let _ = session.unpause(&handle).await;
            }
        }
    }
    if let Some(s) = state_to_emit {
        emit_queue_state_from_state(&app, s);
        queue::try_start_next(app, state.download_queue.clone()).await;
        Ok("Download resumed".to_string())
    } else {
        Err("Download cannot be resumed".to_string())
    }
}

#[tauri::command]
pub async fn retry_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
) -> Result<String, String> {
    let state_to_emit = {
        let mut q = state.download_queue.lock().await;
        let result = q.retry(download_id);
        (result, q.get_state())
    };
    match state_to_emit {
        (Ok(()), s) => {
            emit_queue_state_from_state(&app, s);
            queue::try_start_next(app, state.download_queue.clone()).await;
            Ok("Download re-queued".to_string())
        }
        (Err(e), s) => {
            // A redacted link was just settled as expired: show it.
            if e == queue::LINK_EXPIRED_MESSAGE {
                emit_queue_state_from_state(&app, s);
            }
            Err(e)
        }
    }
}

// Deletes only the exact recorded final path (file → unlink, dir → recursive)
// when it exists, plus http_fetcher sidecars derived from that exact path.
// Bounded by construction: every target is derived from the stored file_path,
// so it can never touch an unrelated file. Best-effort: failures here never
// fail the list removal.
fn delete_downloaded_path(path: &str) {
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return;
    }
    match p.metadata() {
        Ok(meta) if meta.is_dir() => {
            if let Err(e) = std::fs::remove_dir_all(p) {
                tracing::warn!("[remove] failed to delete directory: {}", e);
            }
        }
        Ok(_) => {
            if let Err(e) = std::fs::remove_file(p) {
                tracing::warn!("[remove] failed to delete file: {}", e);
            }
        }
        Err(_) => {}
    }
    for suffix in [".part", ".resume.json"] {
        let sidecar = format!("{}{}", path, suffix);
        let sp = std::path::Path::new(&sidecar);
        if sp.is_file() {
            let _ = std::fs::remove_file(sp);
        }
    }
}

#[tauri::command]
pub async fn remove_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
    delete_file: Option<bool>,
) -> Result<String, String> {
    let (state_to_emit, seeding_torrent_id, file_path) = {
        let mut q = state.download_queue.lock().await;
        let path = if delete_file.unwrap_or(false) {
            q.items
                .iter()
                .find(|i| i.id == download_id)
                .and_then(|i| i.file_path.clone())
        } else {
            None
        };
        match q.remove(download_id) {
            Some(torrent_id) => (Some(q.get_state()), torrent_id, path),
            None => (None, None, None),
        }
    };
    if let Some(tid) = seeding_torrent_id {
        if let Some(session) = state.torrent_session.lock().await.as_ref() {
            let _ = session
                .delete(librqbit::api::TorrentIdOrHash::Id(tid), false)
                .await;
        }
    }
    if let Some(path) = file_path {
        delete_downloaded_path(&path);
    }
    if let Some(s) = state_to_emit {
        crate::core::download_log::clear(download_id);
        emit_queue_state_from_state(&app, s);
        queue::try_start_next(app, state.download_queue.clone()).await;
        Ok("Download removed".to_string())
    } else {
        Err("Download not found".to_string())
    }
}

/// "Editar e tentar de novo": recebe o comando como texto, tokeniza como um
/// shell faria (sem rodar shell nenhum) e re-enfileira o item com esse argv.
#[tauri::command]
pub async fn retry_download_with_command(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
    command: String,
) -> Result<String, String> {
    let argv = omniget_core::core::shell_words::split(&command)
        .map_err(|e| format!("Invalid command: {}", e))?;
    if argv.is_empty() {
        return Err("Command is empty".to_string());
    }
    let state_to_emit = {
        let mut q = state.download_queue.lock().await;
        if let Err(e) = q.retry_with_command(download_id, argv) {
            if e == queue::LINK_EXPIRED_MESSAGE {
                let s = q.get_state();
                drop(q);
                emit_queue_state_from_state(&app, s);
            }
            return Err(e);
        }
        q.get_state()
    };
    emit_queue_state_from_state(&app, state_to_emit);
    queue::try_start_next(app, state.download_queue.clone()).await;
    Ok("Download re-queued with custom command".to_string())
}

#[tauri::command]
pub fn get_download_log(download_id: u64) -> Vec<String> {
    crate::core::download_log::get(download_id)
}

#[tauri::command]
pub fn get_recovery_items() -> Vec<crate::core::recovery::RecoveryItem> {
    crate::core::recovery::list()
}

#[tauri::command]
pub fn get_download_history() -> Vec<crate::core::queue_history::HistoryEntry> {
    crate::core::queue_history::list()
}

#[tauri::command]
pub fn clear_download_history() {
    crate::core::queue_history::clear_all();
}

#[tauri::command]
pub fn discard_recovery() {
    crate::core::recovery::clear_all();
}

/// What "Resume downloads" did, so the UI says it honestly (N-2): only
/// `restored` downloads run again; `reconciled` crash-interrupted jobs were
/// settled from their folder without downloading; `attention` counts the ones
/// the person has to act on (interrupted, expired link, not restorable).
#[derive(Debug, Default, Clone, Copy, serde::Serialize, PartialEq, Eq)]
pub struct RecoveryOutcome {
    pub restored: u32,
    pub reconciled: u32,
    pub attention: u32,
}

impl RecoveryOutcome {
    /// Counts one reconciliation verdict of an interrupted external job.
    fn count_reconciled(&mut self, outcome: Result<&str, ()>) {
        match outcome {
            Ok("completed") => self.reconciled += 1,
            Ok("already_terminal") => {}
            Ok(_) => {
                self.reconciled += 1;
                self.attention += 1;
            }
            Err(()) => self.attention += 1,
        }
    }
}

#[tauri::command]
pub async fn restore_recovery(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<RecoveryOutcome, String> {
    let items = crate::core::recovery::list();
    let mut outcome = RecoveryOutcome::default();
    for item in items {
        // External jobs retain their original authority and cannot recover
        // through the personal engine/cookies. Keep evidence for reconciliation.
        if item.platform == "mcp_worker" {
            continue;
        }
        // The log keeps only the redacted URL; a link that carried a secret
        // cannot be restored. Say so in the list instead of downloading
        // `[REDACTED]`.
        if crate::core::recovery::is_expired(&item) {
            let snapshot = {
                let mut q = state.download_queue.lock().await;
                q.push_expired_link(&item);
                q.get_state()
            };
            emit_queue_state_from_state(&app, snapshot);
            outcome.attention += 1;
            continue;
        }
        let recovered_id = item.id;
        match download_from_url(
            app.clone(),
            state.clone(),
            item.url,
            item.output_dir,
            item.download_mode,
            item.quality,
            item.format_id,
            item.referer,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        {
            Ok(_) => {
                crate::core::recovery::remove(recovered_id);
                outcome.restored += 1;
            }
            Err(e) => {
                tracing::warn!("[recovery] restore failed: {}", e);
                outcome.attention += 1;
            }
        }
    }
    // Intents may precede the legacy queue WAL at a crash boundary.
    // Recover those too, with their stable ID and current external authority.
    for intent in crate::mcp::download_intents::all()? {
        match intent.stage.as_str() {
            "prepared" | "admitting" | "enqueued" => {
                match crate::mcp::downloads::restore_intent(&app, intent).await {
                    Ok(value) if value["outcome"] == "queued" => outcome.restored += 1,
                    Ok(_) => {}
                    Err(e) => tracing::warn!("[recovery] external intent retained: {}", e),
                }
            }
            // Running when the app died (D-08): settle it from its folder
            // (complete output confirms it, anything else becomes a retryable
            // interrupted attempt). Never re-downloads; the client owns retry.
            "executing" | "unknown" => {
                let id = intent.job_id;
                if state
                    .download_queue
                    .lock()
                    .await
                    .items
                    .iter()
                    .any(|i| i.id == id)
                {
                    continue;
                }
                match tokio::task::spawn_blocking(move || {
                    crate::mcp::download_intents::reconcile(None, id)
                })
                .await
                {
                    Ok(Ok((verdict, _))) => {
                        crate::core::recovery::remove(id);
                        tracing::info!(
                            "[recovery] external download {} reconciled: {}",
                            id,
                            verdict
                        );
                        // Settled, not gone: into history and the queue (N-2).
                        crate::mcp::downloads::settle_reconciled(&app, id).await;
                        outcome.count_reconciled(Ok(&verdict));
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("[recovery] external download {} not reconciled: {}", id, e);
                        outcome.count_reconciled(Err(()));
                    }
                    Err(_) => {
                        tracing::warn!("[recovery] external download {} not reconciled", id);
                        outcome.count_reconciled(Err(()));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(outcome)
}

#[tauri::command]
pub fn parse_batch_file(path: String) -> Result<Vec<String>, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("Read error: {}", e))?;
    let mut urls = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let candidate = line.split('|').next().unwrap_or(line).trim();
        if candidate.starts_with("http://")
            || candidate.starts_with("https://")
            || candidate.starts_with("magnet:")
            || candidate.starts_with("p2p:")
        {
            urls.push(candidate.to_string());
        }
    }
    Ok(urls)
}

#[tauri::command]
pub async fn update_max_concurrent(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    max: u32,
) -> Result<String, String> {
    if !(1..=10).contains(&max) {
        return Err("Value must be between 1 and 10".to_string());
    }
    let state_to_emit = {
        let mut q = state.download_queue.lock().await;
        q.max_concurrent = max;
        q.get_state()
    };
    emit_queue_state_from_state(&app, state_to_emit);
    queue::try_start_next(app, state.download_queue.clone()).await;
    Ok(format!("Max concurrent set to {}", max))
}

#[tauri::command]
pub async fn pause_all_downloads(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<u32, String> {
    let (state_to_emit, count, paused_torrents) = {
        let mut q = state.download_queue.lock().await;
        let paused = q.pause_all();
        let n = paused.len() as u32;
        let torrents: Vec<usize> = paused.iter().filter_map(|(_, tid)| *tid).collect();
        (q.get_state(), n, torrents)
    };
    if let Some(session) = state.torrent_session.lock().await.as_ref() {
        for tid in paused_torrents {
            if let Some(handle) = session.get(librqbit::api::TorrentIdOrHash::Id(tid)) {
                let _ = session.pause(&handle).await;
            }
        }
    }
    emit_queue_state_from_state(&app, state_to_emit);
    Ok(count)
}

#[tauri::command]
pub async fn resume_all_downloads(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<u32, String> {
    let (state_to_emit, count, resumed_torrents) = {
        let mut q = state.download_queue.lock().await;
        let resumed = q.resume_all();
        let n = resumed.len() as u32;
        let torrents: Vec<usize> = resumed.iter().filter_map(|(_, tid)| *tid).collect();
        (q.get_state(), n, torrents)
    };
    if let Some(session) = state.torrent_session.lock().await.as_ref() {
        for tid in resumed_torrents {
            if let Some(handle) = session.get(librqbit::api::TorrentIdOrHash::Id(tid)) {
                let _ = session.unpause(&handle).await;
            }
        }
    }
    emit_queue_state_from_state(&app, state_to_emit);
    queue::try_start_next(app, state.download_queue.clone()).await;
    Ok(count)
}

#[tauri::command]
pub async fn reorder_queue(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    ids: Vec<u64>,
) -> Result<bool, String> {
    let (changed, state_to_emit) = {
        let mut q = state.download_queue.lock().await;
        let ok = q.reorder(ids);
        (ok, q.get_state())
    };
    if changed {
        emit_queue_state_from_state(&app, state_to_emit);
    }
    Ok(changed)
}

#[tauri::command]
pub async fn clear_finished_downloads(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let state_to_emit = {
        let mut q = state.download_queue.lock().await;
        q.clear_finished();
        q.get_state()
    };
    emit_queue_state_from_state(&app, state_to_emit);
    Ok("Finished downloads cleared".to_string())
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn reveal_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path))
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        use std::path::{Path, PathBuf};
        use std::process::Stdio;

        let file_path = Path::new(&path);
        let abs_path: PathBuf = if file_path.is_absolute() {
            file_path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(file_path))
                .unwrap_or_else(|_| file_path.to_path_buf())
        };

        let dir_path = abs_path.parent().unwrap_or(&abs_path);
        let item_uri = url::Url::from_file_path(&abs_path)
            .or_else(|_| url::Url::from_file_path(file_path))
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", abs_path.display()));
        let dir_uri = url::Url::from_directory_path(dir_path)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", dir_path.display()));

        let gdbus_show_items_arg = format!(
            "[\"{}\"]",
            item_uri.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let show_items_with_gdbus = tokio::process::Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.FileManager1",
                "--object-path",
                "/org/freedesktop/FileManager1",
                "--method",
                "org.freedesktop.FileManager1.ShowItems",
                &gdbus_show_items_arg,
                "",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        let show_items_ok = if show_items_with_gdbus {
            true
        } else {
            let dbus_send_array_arg = format!("array:string:{}", item_uri);
            tokio::process::Command::new("dbus-send")
                .args([
                    "--session",
                    "--dest=org.freedesktop.FileManager1",
                    "--type=method_call",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                    &dbus_send_array_arg,
                    "string:",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false)
        };

        if !show_items_ok {
            let portal_ok = tokio::process::Command::new("gdbus")
                .args([
                    "call",
                    "--session",
                    "--dest",
                    "org.freedesktop.portal.Desktop",
                    "--object-path",
                    "/org/freedesktop/portal/desktop",
                    "--method",
                    "org.freedesktop.portal.OpenURI.OpenDirectory",
                    "",
                    &dir_uri,
                    "{}",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);

            if !portal_ok {
                std::process::Command::new("xdg-open")
                    .arg(dir_path)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn open_path_default(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &path])
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
mod recovery_outcome_tests {
    use super::RecoveryOutcome;

    // N-2: "Resume downloads" used to toast "N download(s) resumed" for jobs
    // it only settled from their folder.
    #[test]
    fn reconciled_jobs_are_not_counted_as_resumed() {
        let mut o = RecoveryOutcome::default();
        o.count_reconciled(Ok("completed"));
        o.count_reconciled(Ok("interrupted"));
        o.count_reconciled(Ok("already_terminal"));
        o.count_reconciled(Err(()));
        assert_eq!(
            o,
            RecoveryOutcome {
                restored: 0,
                reconciled: 2,
                attention: 2
            }
        );
        let json = serde_json::to_value(o).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"restored":0,"reconciled":2,"attention":2})
        );
    }
}

/// Initial presentation snapshot, including an empty queue after a WebView remount.
#[tauri::command]
pub async fn get_download_queue(app: tauri::AppHandle) -> Vec<crate::core::queue::QueueItemInfo> {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    let queue = state.download_queue.lock().await;
    crate::core::queue::redacted_for_display(queue.get_state())
}

#[cfg(test)]
mod subtitle_lang_tests {
    use super::subtitle_lang_args;

    #[test]
    fn plain_language_lists_become_subtitle_flags() {
        let args = subtitle_lang_args(" en,pt-BR ").unwrap();
        assert_eq!(args[3], "en,pt-BR");
        assert!(args.contains(&"--write-sub".to_string()));
    }

    #[test]
    fn anything_else_is_ignored() {
        assert!(subtitle_lang_args("").is_none());
        assert!(subtitle_lang_args("--exec rm").is_none());
        assert!(subtitle_lang_args("-en").is_none());
        assert!(subtitle_lang_args("en; ls").is_none());
    }
}
