use serde::Serialize;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::platforms::udemy::api::UdemyCourse;
use crate::platforms::udemy::downloader::UdemyDownloader;
use crate::storage::config;
use crate::AppState;

#[derive(Clone, Serialize)]
struct UdemyDownloadCompleteEvent {
    course_name: String,
    success: bool,
    error: Option<String>,
}

#[tauri::command]
pub async fn start_udemy_course_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    course_json: String,
    output_dir: String,
) -> Result<String, String> {
    let course: UdemyCourse =
        serde_json::from_str(&course_json).map_err(|e| format!("Invalid JSON: {}", e))?;

    let course_name = course.title.clone();
    let course_id = course.id;
    let session = state.udemy_session.clone();
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
        let downloader = UdemyDownloader::new(
            session,
            settings.advanced.max_concurrent_segments,
            settings.advanced.max_retries,
        );
        let (tx, mut rx) = mpsc::channel(32);

        let app_clone = app.clone();
        let progress_forwarder = tokio::spawn(async move {
            while let Some(progress) = rx.recv().await {
                let _ = app_clone.emit("udemy-download-progress", &progress);
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
                    "udemy-download-complete",
                    &UdemyDownloadCompleteEvent {
                        course_name: course.title,
                        success: true,
                        error: None,
                    },
                );
            }
            Err(e) => {
                tracing::error!("[udemy] download error for '{}': {}", course.title, e);
                let _ = app.emit(
                    "udemy-download-complete",
                    &UdemyDownloadCompleteEvent {
                        course_name: course.title,
                        success: false,
                        error: Some(e.to_string()),
                    },
                );
            }
        }
    });

    Ok(format!("Download started: {}", course_name))
}

#[tauri::command]
pub async fn cancel_udemy_course_download(
    state: tauri::State<'_, AppState>,
    course_id: u64,
) -> Result<(), String> {
    let map = state.active_downloads.lock().await;
    if let Some(token) = map.get(&course_id) {
        token.cancel();
        Ok(())
    } else {
        Err("No active download for this course".to_string())
    }
}
