use std::time::{Duration, Instant};

use crate::platforms::udemy::auth::{
    authenticate, delete_saved_session, load_saved_session, save_session,
};
use crate::AppState;

const SESSION_COOLDOWN: Duration = Duration::from_secs(5 * 60);

#[tauri::command]
pub async fn udemy_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    email: String,
) -> Result<String, String> {
    let _ = delete_saved_session().await;
    state.udemy_session.lock().await.take();
    *state.udemy_session_validated_at.lock().await = None;
    *state.udemy_courses_cache.lock().await = None;

    match authenticate(&app, &email).await {
        Ok(session) => {
            let response_email = session.email.clone();
            let _ = save_session(&session).await;
            let mut guard = state.udemy_session.lock().await;
            *guard = Some(session);
            *state.udemy_session_validated_at.lock().await = Some(Instant::now());
            Ok(response_email)
        }
        Err(e) => {
            tracing::error!("[udemy] login failed: {}", e);
            Err(format!("Login failed: {}", e))
        }
    }
}

#[tauri::command]
pub async fn udemy_check_session(
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let has_memory_session = state.udemy_session.lock().await.is_some();

    if !has_memory_session {
        match load_saved_session().await {
            Ok(session) => {
                let mut guard = state.udemy_session.lock().await;
                *guard = Some(session);
            }
            Err(_) => {
                return Err("not_authenticated".to_string());
            }
        }
    }

    let guard = state.udemy_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "not_authenticated".to_string())?;
    let email = session.email.clone();

    {
        let validated_at = state.udemy_session_validated_at.lock().await;
        if let Some(at) = *validated_at {
            if at.elapsed() < SESSION_COOLDOWN {
                return Ok(email);
            }
        }
    }

    let client = session.client.clone();
    drop(guard);

    let resp = client
        .get("https://www.udemy.com/api-2.0/users/me/")
        .send()
        .await
        .map_err(|e| format!("Validation error: {}", e))?;

    let status = resp.status();
    if status.is_success() {
        *state.udemy_session_validated_at.lock().await = Some(Instant::now());
        Ok(email)
    } else if status == reqwest::StatusCode::UNAUTHORIZED {
        state.udemy_session.lock().await.take();
        *state.udemy_session_validated_at.lock().await = None;
        *state.udemy_courses_cache.lock().await = None;
        let _ = delete_saved_session().await;
        Err("session_expired".to_string())
    } else if status == reqwest::StatusCode::FORBIDDEN {
        let body = resp.text().await.unwrap_or_default();
        let body_lower = body.to_lowercase();
        if body_lower.contains("just a moment") || body_lower.contains("cf_chl") || body_lower.contains("<!doctype") {
            tracing::warn!("[udemy] check_session got Cloudflare challenge, keeping session alive");
            *state.udemy_session_validated_at.lock().await = Some(Instant::now());
            Ok(email)
        } else {
            tracing::error!("[udemy] check_session got real 403: {}", &body[..body.len().min(300)]);
            state.udemy_session.lock().await.take();
            *state.udemy_session_validated_at.lock().await = None;
            *state.udemy_courses_cache.lock().await = None;
            let _ = delete_saved_session().await;
            Err("session_expired".to_string())
        }
    } else {
        Err(format!("session_check_failed: {}", status))
    }
}

#[tauri::command]
pub async fn udemy_logout(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _ = delete_saved_session().await;
    state.udemy_session.lock().await.take();
    *state.udemy_session_validated_at.lock().await = None;
    *state.udemy_courses_cache.lock().await = None;
    if let Some(w) = state.udemy_api_webview.lock().await.take() {
        let _ = w.close();
    }
    Ok(())
}
