use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use crate::platforms::gumroad::api::{self, GumroadProduct, GumroadSession};
use crate::platforms::gumroad::downloader;
use crate::AppState;

const SESSION_COOLDOWN: Duration = Duration::from_secs(5 * 60);
const COURSES_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Serialize)]
struct GumroadDownloadCompleteEvent {
    course_name: String,
    success: bool,
    error: Option<String>,
}

#[tauri::command]
pub async fn gumroad_login(
    state: tauri::State<'_, AppState>,
    email: String,
    password: String,
) -> Result<String, String> {
    let _ = api::delete_saved_session().await;
    state.gumroad_session.lock().await.take();
    *state.gumroad_session_validated_at.lock().await = None;
    *state.gumroad_courses_cache.lock().await = None;

    match api::authenticate(&email, &password).await {
        Ok(session) => {
            let response_email = session.email.clone();
            let _ = api::save_session(&session).await;
            let mut guard = state.gumroad_session.lock().await;
            *guard = Some(session);
            *state.gumroad_session_validated_at.lock().await = Some(Instant::now());
            Ok(response_email)
        }
        Err(e) => {
            tracing::error!("[gumroad] login failed: {}", e);
            Err(format!("Login failed: {}", e))
        }
    }
}

#[tauri::command]
pub async fn gumroad_login_token(
    state: tauri::State<'_, AppState>,
    token: String,
) -> Result<String, String> {
    let _ = api::delete_saved_session().await;
    state.gumroad_session.lock().await.take();
    *state.gumroad_session_validated_at.lock().await = None;
    *state.gumroad_courses_cache.lock().await = None;

    let parsed_token = crate::core::cookie_parser::parse_bearer_input(&token);

    let session = GumroadSession {
        token: parsed_token.clone(),
        email: String::new(),
        client: crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .user_agent("okhttp/4.8.1")
            .default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                h.insert("Authorization", format!("Bearer {}", parsed_token).parse().unwrap());
                h
            })
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| format!("Failed to build client: {}", e))?,
    };

    match api::validate_token(&session).await {
        Ok(true) => {
            let _ = api::save_session(&session).await;
            let mut guard = state.gumroad_session.lock().await;
            *guard = Some(session);
            *state.gumroad_session_validated_at.lock().await = Some(Instant::now());
            Ok("authenticated".to_string())
        }
        Ok(false) => Err("Invalid token".to_string()),
        Err(e) => Err(format!("Token validation failed: {}", e)),
    }
}

#[tauri::command]
pub async fn gumroad_check_session(
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let has_memory_session = state.gumroad_session.lock().await.is_some();

    if !has_memory_session {
        match api::load_session().await {
            Ok(Some(session)) => {
                let mut guard = state.gumroad_session.lock().await;
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

    let guard = state.gumroad_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "not_authenticated".to_string())?;
    let email = session.email.clone();

    {
        let validated_at = state.gumroad_session_validated_at.lock().await;
        if let Some(at) = *validated_at {
            if at.elapsed() < SESSION_COOLDOWN {
                return Ok(email);
            }
        }
    }

    let session_clone = session.clone();
    drop(guard);

    match api::validate_token(&session_clone).await {
        Ok(true) => {
            *state.gumroad_session_validated_at.lock().await = Some(Instant::now());
            Ok(email)
        }
        Ok(false) => {
            state.gumroad_session.lock().await.take();
            *state.gumroad_session_validated_at.lock().await = None;
            *state.gumroad_courses_cache.lock().await = None;
            let _ = api::delete_saved_session().await;
            Err("session_expired".to_string())
        }
        Err(e) => Err(format!("session_check_failed: {}", e)),
    }
}

#[tauri::command]
pub async fn gumroad_logout(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _ = api::delete_saved_session().await;
    state.gumroad_session.lock().await.take();
    *state.gumroad_session_validated_at.lock().await = None;
    *state.gumroad_courses_cache.lock().await = None;
    Ok(())
}

async fn fetch_gumroad_products(
    state: &tauri::State<'_, AppState>,
) -> Result<Vec<GumroadProduct>, String> {
    let guard = state.gumroad_session.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| "Not authenticated. Please log in first.".to_string())?;

    let products = api::list_products(session)
        .await
        .map_err(|e| e.to_string())?;

    let mut cache = state.gumroad_courses_cache.lock().await;
    *cache = Some(crate::GumroadCoursesCache {
        products: products.clone(),
        fetched_at: Instant::now(),
    });

    Ok(products)
}

#[tauri::command]
pub async fn gumroad_list_products(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GumroadProduct>, String> {
    {
        let cache = state.gumroad_courses_cache.lock().await;
        if let Some(ref cached) = *cache {
            if cached.fetched_at.elapsed() < COURSES_CACHE_TTL {
                return Ok(cached.products.clone());
            }
        }
    }

    fetch_gumroad_products(&state).await
}

#[tauri::command]
pub async fn gumroad_refresh_products(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GumroadProduct>, String> {
    {
        let mut cache = state.gumroad_courses_cache.lock().await;
        *cache = None;
    }
    fetch_gumroad_products(&state).await
}

#[tauri::command]
pub async fn start_gumroad_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    product_json: String,
    output_dir: String,
) -> Result<String, String> {
    let product: GumroadProduct =
        serde_json::from_str(&product_json).map_err(|e| format!("Invalid JSON: {}", e))?;

    let product_raw: serde_json::Value =
        serde_json::from_str(&product_json).map_err(|e| format!("Invalid raw JSON: {}", e))?;

    let product_name = product.name.clone();
    let product_id_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        product.id.hash(&mut hasher);
        hasher.finish()
    };
    let active = state.active_downloads.clone();

    let cancel_token = CancellationToken::new();

    {
        let mut map = active.lock().await;
        if map.contains_key(&product_id_hash) {
            return Err("Download already in progress for this product".to_string());
        }
        map.insert(product_id_hash, cancel_token.clone());
    }

    let session = {
        let guard = state.gumroad_session.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| "Not authenticated. Please log in first.".to_string())?
            .clone()
    };

    tokio::spawn(async move {
        let result =
            downloader::download_product(&app, &session, &product, &product_raw, &output_dir, cancel_token)
                .await;

        {
            let mut map = active.lock().await;
            map.remove(&product_id_hash);
        }

        match result {
            Ok(()) => {
                let _ = app.emit(
                    "download-complete",
                    &GumroadDownloadCompleteEvent {
                        course_name: product.name,
                        success: true,
                        error: None,
                    },
                );
            }
            Err(e) => {
                tracing::error!("[gumroad] download error for '{}': {}", product.name, e);
                let _ = app.emit(
                    "download-complete",
                    &GumroadDownloadCompleteEvent {
                        course_name: product.name,
                        success: false,
                        error: Some(e.to_string()),
                    },
                );
            }
        }
    });

    Ok(format!("Download started: {}", product_name))
}
