use serde::Serialize;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::core::url_parser;
use crate::platforms::Platform;
use crate::platforms::hotmart::api::Course;
use crate::platforms::hotmart::downloader::HotmartDownloader;
use crate::storage::config;
use crate::AppState;

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
pub async fn detect_platform(
    url: String,
) -> Result<PlatformInfo, String> {
    match Platform::from_url(&url) {
        Some(platform) => {
            let parsed = url_parser::parse_url(&url);
            Ok(PlatformInfo {
                platform: platform.to_string(),
                supported: true,
                content_id: parsed.as_ref().and_then(|p| p.content_id.clone()),
                content_type: parsed.map(|p| format!("{:?}", p.content_type).to_lowercase()),
            })
        }
        None => Ok(PlatformInfo {
            platform: "unknown".to_string(),
            supported: false,
            content_id: None,
            content_type: None,
        }),
    }
}

#[derive(Clone, Serialize)]
pub struct DownloadStarted {
    pub id: u64,
    pub title: String,
}

#[derive(Clone, Serialize)]
struct GenericDownloadProgress {
    id: u64,
    title: String,
    platform: String,
    percent: f64,
}

#[derive(Clone, Serialize)]
struct GenericDownloadComplete {
    id: u64,
    title: String,
    platform: String,
    success: bool,
    error: Option<String>,
    file_path: Option<String>,
    file_size_bytes: Option<u64>,
    file_count: Option<u32>,
}

#[tauri::command]
pub async fn download_from_url(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
    output_dir: String,
) -> Result<DownloadStarted, String> {
    let platform = Platform::from_url(&url)
        .ok_or_else(|| "Plataforma não reconhecida".to_string())?;

    let download_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let cancel_token = CancellationToken::new();

    {
        let active = state.active_generic_downloads.lock().await;
        if active.values().any(|(u, _)| u == &url) {
            return Err("Download já em andamento para esta URL".to_string());
        }
    }

    {
        let mut active = state.active_generic_downloads.lock().await;
        active.insert(download_id, (url.clone(), cancel_token.clone()));
    }

    let active_downloads = state.active_generic_downloads.clone();
    let cleanup = {
        let active_downloads = active_downloads.clone();
        move || async move {
            active_downloads.lock().await.remove(&download_id);
        }
    };

    let downloader = match state.registry.find_platform(&url) {
        Some(d) => d,
        None => {
            cleanup().await;
            return Err(format!("Nenhum downloader registrado para {}", platform));
        }
    };

    let settings = config::load_settings(&app);
    let platform_name = platform.to_string();

    let info = match downloader.get_media_info(&url).await {
        Ok(info) => info,
        Err(e) => {
            cleanup().await;
            return Err(format!("Erro ao obter informações: {}", e));
        }
    };

    let title = info.title.clone();
    let return_title = title.clone();
    let file_count = if info.media_type == crate::models::media::MediaType::Carousel {
        info.available_qualities.len() as u32
    } else {
        1
    };

    tracing::info!("Iniciando download '{}' ({}) em {}", title, platform_name, output_dir);

    tokio::spawn(async move {
        let opts = crate::models::media::DownloadOptions {
            quality: Some(settings.download.video_quality.clone()),
            output_dir: std::path::PathBuf::from(&output_dir),
            filename_template: None,
            download_subtitles: false,
        };

        let _ = app.emit("generic-download-progress", &GenericDownloadProgress {
            id: download_id,
            title: title.clone(),
            platform: platform_name.clone(),
            percent: 0.0,
        });

        let (tx, mut rx) = mpsc::channel::<f64>(32);

        let app_progress = app.clone();
        let title_progress = title.clone();
        let platform_progress = platform_name.clone();
        let progress_forwarder = tokio::spawn(async move {
            while let Some(percent) = rx.recv().await {
                let _ = app_progress.emit("generic-download-progress", &GenericDownloadProgress {
                    id: download_id,
                    title: title_progress.clone(),
                    platform: platform_progress.clone(),
                    percent,
                });
            }
        });

        let result = tokio::select! {
            r = downloader.download(&info, &opts, tx) => r,
            _ = cancel_token.cancelled() => {
                Err(anyhow::anyhow!("Download cancelado"))
            }
        };

        let _ = progress_forwarder.await;

        {
            active_downloads.lock().await.remove(&download_id);
        }

        match result {
            Ok(dl) => {
                tracing::info!("Download concluído: {}", title);
                let _ = app.emit("generic-download-complete", &GenericDownloadComplete {
                    id: download_id,
                    title: title.clone(),
                    platform: platform_name,
                    success: true,
                    error: None,
                    file_path: Some(dl.file_path.to_string_lossy().to_string()),
                    file_size_bytes: Some(dl.file_size_bytes),
                    file_count: Some(file_count),
                });
            }
            Err(e) => {
                let err_msg = e.to_string();
                tracing::error!("Erro no download de '{}': {}", title, err_msg);
                let _ = app.emit("generic-download-complete", &GenericDownloadComplete {
                    id: download_id,
                    title: title.clone(),
                    platform: platform_name,
                    success: false,
                    error: Some(err_msg),
                    file_path: None,
                    file_size_bytes: None,
                    file_count: None,
                });
            }
        }
    });

    Ok(DownloadStarted { id: download_id, title: return_title })
}

#[tauri::command]
pub async fn cancel_generic_download(
    state: tauri::State<'_, AppState>,
    download_id: u64,
) -> Result<String, String> {
    let mut map = state.active_generic_downloads.lock().await;
    match map.remove(&download_id) {
        Some((_, token)) => {
            token.cancel();
            tracing::info!("Download genérico cancelado para id={}", download_id);
            Ok("Download cancelado".to_string())
        }
        None => Err("Nenhum download ativo para este ID".to_string()),
    }
}

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
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub async fn start_course_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    course_json: String,
    output_dir: String,
) -> Result<String, String> {
    let course: Course =
        serde_json::from_str(&course_json).map_err(|e| format!("JSON inválido: {}", e))?;

    let course_name = course.name.clone();
    let course_id = course.id;
    let session = state.hotmart_session.clone();
    let active = state.active_downloads.clone();

    let cancel_token = CancellationToken::new();

    {
        let mut map = active.lock().await;
        if map.contains_key(&course_id) {
            return Err("Download já em andamento para este curso".to_string());
        }
        map.insert(course_id, cancel_token.clone());
    }

    tracing::info!(
        "Iniciando download do curso '{}' em {}",
        course_name,
        output_dir
    );

    let settings = config::load_settings(&app);

    tokio::spawn(async move {
        let downloader = HotmartDownloader::new(
            session,
            settings.download,
            settings.advanced.max_concurrent_segments,
            settings.advanced.max_retries,
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
                tracing::info!("Download completo: {}", course.name);
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
                tracing::error!("Erro no download de '{}': {}", course.name, e);
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

    Ok(format!("Download iniciado: {}", course_name))
}

#[tauri::command]
pub async fn cancel_course_download(
    state: tauri::State<'_, AppState>,
    course_id: u64,
) -> Result<String, String> {
    let mut map = state.active_downloads.lock().await;
    match map.remove(&course_id) {
        Some(token) => {
            token.cancel();
            tracing::info!("Download cancelado para course_id={}", course_id);
            Ok("Download cancelado".to_string())
        }
        None => Err("Nenhum download ativo para este curso".to_string()),
    }
}

#[tauri::command]
pub async fn get_active_downloads(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<u64>, String> {
    let map = state.active_downloads.lock().await;
    Ok(map.keys().copied().collect())
}
