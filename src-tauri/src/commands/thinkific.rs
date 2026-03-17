use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use crate::platforms::thinkific::api::{self, ThinkificCourse};
use crate::platforms::thinkific::downloader;
use crate::AppState;

const SESSION_COOLDOWN: Duration = Duration::from_secs(5 * 60);
const COURSES_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Serialize)]
struct ThinkificDownloadCompleteEvent {
    course_name: String,
    success: bool,
    error: Option<String>,
}

#[tauri::command]
pub async fn thinkific_login(
    state: tauri::State<'_, AppState>,
    cookies: String,
    site_url: String,
) -> Result<String, String> {
    let _ = api::delete_saved_session().await;
    state.thinkific_session.lock().await.take();
    *state.thinkific_session_validated_at.lock().await = None;
    *state.thinkific_courses_cache.lock().await = None;

    let parsed = crate::core::cookie_parser::parse_cookie_input(&cookies, "thinkific_session");

    let session = api::create_session(&parsed.cookie_string, &site_url)
        .map_err(|e| format!("Failed to create session: {}", e))?;

    match api::validate_session(&session).await {
        Ok(true) => {
            let site = session.site_url.clone();
            let _ = api::save_session(&session).await;
            let mut guard = state.thinkific_session.lock().await;
            *guard = Some(session);
            *state.thinkific_session_validated_at.lock().await = Some(Instant::now());
            Ok(site)
        }
        Ok(false) => Err("Invalid cookies or site URL".to_string()),
        Err(e) => Err(format!("Session validation failed: {}", e)),
    }
}

#[tauri::command]
pub async fn thinkific_check_session(
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let has_memory_session = state.thinkific_session.lock().await.is_some();

    if !has_memory_session {
        match api::load_session().await {
            Ok(Some(session)) => {
                let mut guard = state.thinkific_session.lock().await;
                *guard = Some(session);
            }
            Ok(None) => {
                return Err("not_authenticated".to_string());
            }
            Err(_) => {
                return Err("not_authenticated".to_string());
            }
        }
    }

    let guard = state.thinkific_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "not_authenticated".to_string())?;
    let site_url = session.site_url.clone();

    {
        let validated_at = state.thinkific_session_validated_at.lock().await;
        if let Some(at) = *validated_at {
            if at.elapsed() < SESSION_COOLDOWN {
                return Ok(site_url);
            }
        }
    }

    let session_clone = session.clone();
    drop(guard);

    match api::validate_session(&session_clone).await {
        Ok(true) => {
            *state.thinkific_session_validated_at.lock().await = Some(Instant::now());
            Ok(site_url)
        }
        Ok(false) => {
            state.thinkific_session.lock().await.take();
            *state.thinkific_session_validated_at.lock().await = None;
            *state.thinkific_courses_cache.lock().await = None;
            let _ = api::delete_saved_session().await;
            Err("session_expired".to_string())
        }
        Err(e) => Err(format!("session_check_failed: {}", e)),
    }
}

#[tauri::command]
pub async fn thinkific_logout(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _ = api::delete_saved_session().await;
    state.thinkific_session.lock().await.take();
    *state.thinkific_session_validated_at.lock().await = None;
    *state.thinkific_courses_cache.lock().await = None;
    Ok(())
}

async fn fetch_thinkific_courses(
    state: &tauri::State<'_, AppState>,
) -> Result<Vec<ThinkificCourse>, String> {
    let guard = state.thinkific_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "Not authenticated. Please log in first.".to_string())?;

    let courses = api::list_courses(session)
        .await
        .map_err(|e| e.to_string())?;

    let mut cache = state.thinkific_courses_cache.lock().await;
    *cache = Some(crate::ThinkificCoursesCache {
        courses: courses.clone(),
        fetched_at: Instant::now(),
    });

    Ok(courses)
}

#[tauri::command]
pub async fn thinkific_list_courses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ThinkificCourse>, String> {
    {
        let cache = state.thinkific_courses_cache.lock().await;
        if let Some(ref cached) = *cache {
            if cached.fetched_at.elapsed() < COURSES_CACHE_TTL {
                return Ok(cached.courses.clone());
            }
        }
    }

    fetch_thinkific_courses(&state).await
}

#[tauri::command]
pub async fn thinkific_refresh_courses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ThinkificCourse>, String> {
    {
        let mut cache = state.thinkific_courses_cache.lock().await;
        *cache = None;
    }
    fetch_thinkific_courses(&state).await
}

#[tauri::command]
pub async fn start_thinkific_course_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    course_json: String,
    output_dir: String,
) -> Result<String, String> {
    let course: ThinkificCourse =
        serde_json::from_str(&course_json).map_err(|e| format!("Invalid JSON: {}", e))?;

    let course_name = course.name.clone();
    let course_id_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        course.id.hash(&mut hasher);
        hasher.finish()
    };
    let active = state.active_downloads.clone();

    let cancel_token = CancellationToken::new();

    {
        let mut map = active.lock().await;
        if map.contains_key(&course_id_hash) {
            return Err("Download already in progress for this course".to_string());
        }
        map.insert(course_id_hash, cancel_token.clone());
    }

    let session = {
        let guard = state.thinkific_session.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| "Not authenticated. Please log in first.".to_string())?
            .clone()
    };

    tokio::spawn(async move {
        let result =
            downloader::download_full_course(&app, &session, &course, &output_dir, cancel_token)
                .await;

        {
            let mut map = active.lock().await;
            map.remove(&course_id_hash);
        }

        match result {
            Ok(()) => {
                let _ = app.emit(
                    "download-complete",
                    &ThinkificDownloadCompleteEvent {
                        course_name: course.name,
                        success: true,
                        error: None,
                    },
                );
            }
            Err(e) => {
                tracing::error!("[thinkific] download error for '{}': {}", course.name, e);
                let _ = app.emit(
                    "download-complete",
                    &ThinkificDownloadCompleteEvent {
                        course_name: course.name,
                        success: false,
                        error: Some(e.to_string()),
                    },
                );
            }
        }
    });

    Ok(format!("Download started: {}", course_name))
}
