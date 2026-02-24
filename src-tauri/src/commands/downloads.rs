use serde::Serialize;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::core::queue::{self, emit_queue_state_from_state, QueueItemInfo};
use crate::core::url_parser;
use crate::platforms::Platform;
use crate::storage::config;
use crate::AppState;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;
#[cfg(not(target_os = "android"))]
use crate::models::media::FormatInfo;
#[cfg(not(target_os = "android"))]
use crate::platforms::hotmart::api::Course;
#[cfg(not(target_os = "android"))]
use crate::platforms::hotmart::downloader::HotmartDownloader;

#[derive(Clone, Serialize)]
struct DownloadCompleteEvent {
    course_name: String,
    success: bool,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct PlatformInfo {
    pub platform: String,
    pub supported: bool,
    pub content_id: Option<String>,
    pub content_type: Option<String>,
}

#[tauri::command]
pub async fn detect_platform(url: String) -> Result<PlatformInfo, String> {
    let _timer_start = std::time::Instant::now();
    match Platform::from_url(&url) {
        Some(platform) => {
            let parsed = url_parser::parse_url(&url);
            let result = Ok(PlatformInfo {
                platform: platform.to_string(),
                supported: true,
                content_id: parsed.as_ref().and_then(|p| p.content_id.clone()),
                content_type: parsed.map(|p| format!("{:?}", p.content_type).to_lowercase()),
            });
            tracing::info!("[perf] detect_platform took {:?}", _timer_start.elapsed());
            result
        }
        None => {
            let is_valid_url = url::Url::parse(&url)
                .map(|u| u.scheme() == "http" || u.scheme() == "https")
                .unwrap_or(false);
            let result = Ok(PlatformInfo {
                platform: if is_valid_url { "generic".to_string() } else { "unknown".to_string() },
                supported: is_valid_url,
                content_id: None,
                content_type: None,
            });
            tracing::info!("[perf] detect_platform took {:?}", _timer_start.elapsed());
            result
        }
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn get_media_formats(url: String) -> Result<Vec<FormatInfo>, String> {
    let _timer_start = std::time::Instant::now();
    let ytdlp_path = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {}", e))?;

    let json = ytdlp::get_video_info(&ytdlp_path, &url)
        .await
        .map_err(|e| format!("Failed to get formats: {}", e))?;

    tracing::info!("[perf] get_media_formats took {:?}", _timer_start.elapsed());
    Ok(ytdlp::parse_formats(&json))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn prefetch_media_info(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
) -> Result<(), String> {
    let platform = Platform::from_url(&url);
    let platform_name = platform
        .map(|p| p.to_string())
        .unwrap_or_else(|| "generic".to_string());

    let downloader = match state.registry.find_platform(&url) {
        Some(d) => d,
        None => return Err("No downloader available".to_string()),
    };

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
) -> Result<DownloadStarted, String> {
    let _timer_start = std::time::Instant::now();
    let platform = Platform::from_url(&url);

    let download_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let download_queue = state.download_queue.clone();

    {
        let settings = config::load_settings(&app);
        let mut q = download_queue.lock().await;
        q.max_concurrent = settings.advanced.max_concurrent_downloads.max(1);
        q.stagger_delay_ms = settings.advanced.stagger_delay_ms;
        if q.has_url(&url) {
            tracing::info!("[perf] download_from_url took {:?}", _timer_start.elapsed());
            return Err("Download already in progress for this URL".to_string());
        }
    }

    let downloader = match state.registry.find_platform(&url) {
        Some(d) => d,
        None => {
            tracing::info!("[perf] download_from_url took {:?}", _timer_start.elapsed());
            return Err("No downloader available for this URL".to_string());
        }
    };

    let platform_name = platform
        .map(|p| p.to_string())
        .unwrap_or_else(|| "generic".to_string());
    let title = url.clone();
    let ytdlp_path = ytdlp::find_ytdlp_cached().await;

    let cached_info = queue::try_get_cached_info(&url).await;

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
            cached_info,
            None,
            None,
            downloader,
            ytdlp_path,
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

    tracing::info!("[perf] download_from_url took {:?}", _timer_start.elapsed());
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
    let state_to_emit = {
        let mut q = state.download_queue.lock().await;
        if q.cancel(download_id) {
            Some(q.get_state())
        } else {
            None
        }
    };
    if let Some(s) = state_to_emit {
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
    let state_to_emit = {
        let mut q = state.download_queue.lock().await;
        if q.pause(download_id) {
            Some(q.get_state())
        } else {
            None
        }
    };
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
    let state_to_emit = {
        let mut q = state.download_queue.lock().await;
        if q.resume(download_id) {
            Some(q.get_state())
        } else {
            None
        }
    };
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
        if q.retry(download_id) {
            Some(q.get_state())
        } else {
            None
        }
    };
    if let Some(s) = state_to_emit {
        emit_queue_state_from_state(&app, s);
        queue::try_start_next(app, state.download_queue.clone()).await;
        Ok("Download re-queued".to_string())
    } else {
        Err("Download cannot be retried".to_string())
    }
}

#[tauri::command]
pub async fn remove_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
) -> Result<String, String> {
    let state_to_emit = {
        let mut q = state.download_queue.lock().await;
        if q.remove(download_id) {
            Some(q.get_state())
        } else {
            None
        }
    };
    if let Some(s) = state_to_emit {
        emit_queue_state_from_state(&app, s);
        queue::try_start_next(app, state.download_queue.clone()).await;
        Ok("Download removed".to_string())
    } else {
        Err("Download not found".to_string())
    }
}

#[tauri::command]
pub async fn get_queue_state(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<QueueItemInfo>, String> {
    let q = state.download_queue.lock().await;
    Ok(q.get_state())
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
        std::process::Command::new("explorer")
            .args(["/select,", &path])
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
        let file_path = std::path::Path::new(&path);
        let dir = file_path.parent().unwrap_or(file_path);

        let portal_result = tokio::process::Command::new("gdbus")
            .args([
                "call", "--session",
                "--dest", "org.freedesktop.portal.Desktop",
                "--object-path", "/org/freedesktop/portal/desktop",
                "--method", "org.freedesktop.portal.OpenURI.OpenDirectory",
                "",
                &format!("file://{}", dir.display()),
                "{}",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;

        match portal_result {
            Ok(status) if status.success() => {}
            _ => {
                std::process::Command::new("xdg-open")
                    .arg(dir)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn start_course_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    course_json: String,
    output_dir: String,
) -> Result<String, String> {
    let _timer_start = std::time::Instant::now();
    let course: Course =
        serde_json::from_str(&course_json).map_err(|e| format!("Invalid JSON: {}", e))?;

    let course_name = course.name.clone();
    let course_id = course.id;
    let session = state.hotmart_session.clone();
    let active = state.active_downloads.clone();

    let cancel_token = CancellationToken::new();

    {
        let mut map = active.lock().await;
        if map.contains_key(&course_id) {
            return Err("Download already in progress for this course".to_string());
        }
        map.insert(course_id, cancel_token.clone());
    }

    let settings = config::load_settings(&app);

    tokio::spawn(async move {
        let downloader = HotmartDownloader::new(
            session,
            settings.download,
            settings.advanced.max_concurrent_segments,
            settings.advanced.max_retries,
            settings.advanced.concurrent_fragments,
        );
        let (tx, mut rx) = mpsc::channel(32);

        let app_clone = app.clone();
        let progress_forwarder = tokio::spawn(async move {
            while let Some(progress) = rx.recv().await {
                let _ = app_clone.emit("download-progress", &progress);
            }
        });

        let result = downloader
            .download_full_course(&course, &output_dir, tx, cancel_token)
            .await;

        let _ = progress_forwarder.await;

        {
            let mut map = active.lock().await;
            map.remove(&course_id);
        }

        match result {
            Ok(()) => {
                let _ = app.emit(
                    "download-complete",
                    &DownloadCompleteEvent {
                        course_name: course.name,
                        success: true,
                        error: None,
                    },
                );
            }
            Err(e) => {
                tracing::error!("Download error for '{}': {}", course.name, e);
                let _ = app.emit(
                    "download-complete",
                    &DownloadCompleteEvent {
                        course_name: course.name,
                        success: false,
                        error: Some(e.to_string()),
                    },
                );
            }
        }
    });

    tracing::info!("[perf] start_course_download took {:?}", _timer_start.elapsed());
    Ok(format!("Download started: {}", course_name))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn cancel_course_download(
    state: tauri::State<'_, AppState>,
    course_id: u64,
) -> Result<String, String> {
    let mut map = state.active_downloads.lock().await;
    match map.remove(&course_id) {
        Some(token) => {
            token.cancel();
            Ok("Download cancelled".to_string())
        }
        None => Err("No active download for this course".to_string()),
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn get_active_downloads(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<u64>, String> {
    let map = state.active_downloads.lock().await;
    Ok(map.keys().copied().collect())
}
