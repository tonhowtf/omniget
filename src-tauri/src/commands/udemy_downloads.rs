use std::sync::Arc;

use serde::Serialize;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::platforms::udemy::api::{self, UdemyCourse, UdemyCurriculum};
use crate::platforms::udemy::downloader::UdemyDownloader;
use crate::platforms::udemy::webview_api;
use crate::storage::config;
use crate::AppState;

#[derive(Clone, Serialize)]
struct UdemyDownloadCompleteEvent {
    course_name: String,
    success: bool,
    error: Option<String>,
}

async fn fetch_curriculum_via_webview(
    app: &tauri::AppHandle,
    api_webview: &Arc<tokio::sync::Mutex<Option<tauri::WebviewWindow>>>,
    result_store: &Arc<std::sync::Mutex<Option<String>>>,
    course_id: u64,
) -> Result<UdemyCurriculum, String> {
    let window = {
        let mut wv_guard = api_webview.lock().await;
        match &*wv_guard {
            Some(w) => w.clone(),
            None => {
                let w = webview_api::ensure_api_webview(app, result_store)
                    .await
                    .map_err(|e| e.to_string())?;
                *wv_guard = Some(w.clone());
                w
            }
        }
    };

    let url = format!(
        "https://www.udemy.com/api-2.0/courses/{}/subscriber-curriculum-items/?fields[lecture]=title,object_index,asset,supplementary_assets&fields[quiz]=title,object_index,type&fields[practice]=title,object_index&fields[chapter]=title,object_index&fields[asset]=title,filename,asset_type,status,is_external,media_license_token,course_is_drmed,media_sources,captions,stream_urls,download_urls,external_url,body&page_size=200",
        course_id
    );

    tracing::info!("[udemy-api] fetching curriculum via webview for course {}", course_id);

    let body = webview_api::webview_get(&window, &url, result_store)
        .await
        .map_err(|e| e.to_string())?;

    let mut data: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("JSON parse error: {}", e))?;

    let mut all_results = data.get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    loop {
        let next_url = data.get("next").and_then(|n| n.as_str()).map(|s| s.to_string());
        match next_url {
            Some(next) if !next.is_empty() => {
                tracing::info!("[udemy-api] fetching next curriculum page via webview");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;

                let page_body = webview_api::webview_get(&window, &next, result_store)
                    .await
                    .map_err(|e| e.to_string())?;

                let page_data: serde_json::Value = serde_json::from_str(&page_body)
                    .map_err(|e| format!("JSON parse error on page: {}", e))?;

                if let Some(new_results) = page_data.get("results").and_then(|r| r.as_array()) {
                    all_results.extend(new_results.iter().cloned());
                }

                data = page_data;
            }
            _ => break,
        }
    }

    tracing::info!("[udemy-api] curriculum fetched: {} items total", all_results.len());

    api::parse_curriculum(course_id, &all_results).map_err(|e| e.to_string())
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

    let api_webview = state.udemy_api_webview.clone();
    let result_store = state.udemy_api_result.clone();

    let curriculum = match fetch_curriculum_via_webview(
        &app, &api_webview, &result_store, course_id
    ).await {
        Ok(c) => c,
        Err(e) => {
            active.lock().await.remove(&course_id);
            return Err(format!("Failed to fetch curriculum: {}", e));
        }
    };

    if curriculum.drm_video_lectures > 0
        && curriculum.drm_video_lectures == curriculum.total_video_lectures
        && curriculum.total_video_lectures > 0
    {
        active.lock().await.remove(&course_id);
        return Err("drm_protected".to_string());
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
            .download_full_course(&course, &output_dir, curriculum, tx, cancel_token)
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
