use serde::Serialize;
use tauri::Emitter;
use tokio::sync::mpsc;

use crate::platforms::hotmart::api::Course;
use crate::platforms::hotmart::downloader::HotmartDownloader;
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

    {
        let mut set = active.lock().await;
        if !set.insert(course_id) {
            return Err("Download já em andamento para este curso".to_string());
        }
    }

    tracing::info!(
        "Iniciando download do curso '{}' em {}",
        course_name,
        output_dir
    );

    tokio::spawn(async move {
        let downloader = HotmartDownloader::new(session);
        let (tx, mut rx) = mpsc::channel(32);

        let app_clone = app.clone();
        let progress_forwarder = tokio::spawn(async move {
            while let Some(progress) = rx.recv().await {
                let _ = app_clone.emit("download-progress", &progress);
            }
        });

        let result = downloader
            .download_full_course(&course, &output_dir, tx)
            .await;

        let _ = progress_forwarder.await;

        {
            let mut set = active.lock().await;
            set.remove(&course_id);
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
pub async fn get_active_downloads(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<u64>, String> {
    let set = state.active_downloads.lock().await;
    Ok(set.iter().copied().collect())
}
