use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use crate::platforms::rocketseat::api::{self, RocketseatCourse};
use crate::platforms::rocketseat::downloader;
use crate::AppState;

const SESSION_COOLDOWN: Duration = Duration::from_secs(5 * 60);
const COURSES_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Serialize)]
struct RocketseatDownloadCompleteEvent {
    course_name: String,
    success: bool,
    error: Option<String>,
}

#[tauri::command]
pub async fn rocketseat_login_token(
    state: tauri::State<'_, AppState>,
    token: String,
) -> Result<String, String> {
    let _ = api::delete_saved_session().await;
    state.rocketseat_session.lock().await.take();
    *state.rocketseat_session_validated_at.lock().await = None;
    *state.rocketseat_courses_cache.lock().await = None;

    let parsed = crate::core::cookie_parser::parse_cookie_input(&token, "skylab_next_access_token_v4");

    tracing::info!(
        "[rocketseat] parsed: token_len={}, cookie_count={}, cookie_string_len={}",
        parsed.token.len(),
        parsed.cookies.len(),
        parsed.cookie_string.len()
    );

    let parsed_token = if parsed.token.is_empty() {
        let bearer = crate::core::cookie_parser::parse_bearer_input(&token);
        tracing::info!("[rocketseat] fallback to parse_bearer_input: len={}", bearer.len());
        bearer
    } else {
        parsed.token
    };

    if parsed_token.is_empty() || parsed_token.len() < 10 {
        return Err("Could not extract token. Paste the value of cookie 'skylab_next_access_token_v4' or the full cookie JSON.".to_string());
    }

    tracing::info!("[rocketseat] using token: {}...", &parsed_token[..parsed_token.len().min(30)]);

    let session = api::create_session(&parsed_token)
        .map_err(|e| format!("Failed to create session: {}", e))?;

    match api::validate_token(&session).await {
        Ok(true) => {
            let _ = api::save_session(&session).await;
            let mut guard = state.rocketseat_session.lock().await;
            *guard = Some(session);
            *state.rocketseat_session_validated_at.lock().await = Some(Instant::now());
            Ok("authenticated".to_string())
        }
        Ok(false) => Err("Invalid token".to_string()),
        Err(e) => Err(format!("Token validation failed: {}", e)),
    }
}

#[tauri::command]
pub async fn rocketseat_check_session(
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let has_memory_session = state.rocketseat_session.lock().await.is_some();

    if !has_memory_session {
        match api::load_session().await {
            Ok(Some(session)) => {
                let mut guard = state.rocketseat_session.lock().await;
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

    let guard = state.rocketseat_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "not_authenticated".to_string())?;

    {
        let validated_at = state.rocketseat_session_validated_at.lock().await;
        if let Some(at) = *validated_at {
            if at.elapsed() < SESSION_COOLDOWN {
                return Ok("authenticated".to_string());
            }
        }
    }

    let session_clone = session.clone();
    drop(guard);

    match api::validate_token(&session_clone).await {
        Ok(true) => {
            *state.rocketseat_session_validated_at.lock().await = Some(Instant::now());
            Ok("authenticated".to_string())
        }
        Ok(false) => {
            state.rocketseat_session.lock().await.take();
            *state.rocketseat_session_validated_at.lock().await = None;
            *state.rocketseat_courses_cache.lock().await = None;
            let _ = api::delete_saved_session().await;
            Err("session_expired".to_string())
        }
        Err(e) => Err(format!("session_check_failed: {}", e)),
    }
}

#[tauri::command]
pub async fn rocketseat_logout(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _ = api::delete_saved_session().await;
    state.rocketseat_session.lock().await.take();
    *state.rocketseat_session_validated_at.lock().await = None;
    *state.rocketseat_courses_cache.lock().await = None;
    Ok(())
}

async fn fetch_rocketseat_courses(
    state: &tauri::State<'_, AppState>,
) -> Result<Vec<RocketseatCourse>, String> {
    let guard = state.rocketseat_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "Not authenticated. Please log in first.".to_string())?;

    let courses = api::list_courses(session)
        .await
        .map_err(|e| e.to_string())?;

    let mut cache = state.rocketseat_courses_cache.lock().await;
    *cache = Some(crate::RocketseatCoursesCache {
        courses: courses.clone(),
        fetched_at: Instant::now(),
    });

    Ok(courses)
}

#[tauri::command]
pub async fn rocketseat_list_courses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RocketseatCourse>, String> {
    {
        let cache = state.rocketseat_courses_cache.lock().await;
        if let Some(ref cached) = *cache {
            if cached.fetched_at.elapsed() < COURSES_CACHE_TTL {
                return Ok(cached.courses.clone());
            }
        }
    }

    fetch_rocketseat_courses(&state).await
}

#[tauri::command]
pub async fn rocketseat_search_courses(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<Vec<RocketseatCourse>, String> {
    let guard = state.rocketseat_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "Not authenticated. Please log in first.".to_string())?;

    let courses = api::search_courses(session, &query)
        .await
        .map_err(|e| e.to_string())?;

    let mut cache = state.rocketseat_courses_cache.lock().await;
    *cache = Some(crate::RocketseatCoursesCache {
        courses: courses.clone(),
        fetched_at: Instant::now(),
    });

    Ok(courses)
}

#[tauri::command]
pub async fn rocketseat_refresh_courses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RocketseatCourse>, String> {
    {
        let mut cache = state.rocketseat_courses_cache.lock().await;
        *cache = None;
    }
    fetch_rocketseat_courses(&state).await
}

#[tauri::command]
pub async fn start_rocketseat_course_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    course_json: String,
    output_dir: String,
) -> Result<String, String> {
    let course: RocketseatCourse =
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
        let guard = state.rocketseat_session.lock().await;
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
                    &RocketseatDownloadCompleteEvent {
                        course_name: course.name,
                        success: true,
                        error: None,
                    },
                );
            }
            Err(e) => {
                tracing::error!("[rocketseat] download error for '{}': {}", course.name, e);
                let _ = app.emit(
                    "download-complete",
                    &RocketseatDownloadCompleteEvent {
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
