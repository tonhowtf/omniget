use serde::Serialize;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

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
