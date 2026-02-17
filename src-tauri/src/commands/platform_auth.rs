use std::collections::HashMap;

use serde::Serialize;

use crate::AppState;

#[derive(Clone, Serialize)]
pub struct AuthStatus {
    pub platform: String,
    pub authenticated: bool,
    pub user_info: HashMap<String, String>,
}

#[derive(Clone, Serialize)]
pub struct AuthResult {
    pub platform: String,
    pub success: bool,
    pub user_info: HashMap<String, String>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn platform_auth_check(
    state: tauri::State<'_, AppState>,
    platform: String,
) -> Result<AuthStatus, String> {
    let provider = state
        .auth_registry
        .get(&platform)
        .ok_or_else(|| format!("Auth provider not found: {}", platform))?;

    let authenticated = provider.is_authenticated().await;
    let mut user_info = HashMap::new();

    if authenticated {
        if let Ok(session) = crate::core::auth::load_auth_session(&platform).await {
            user_info = session.user_info;
        }
    }

    Ok(AuthStatus {
        platform,
        authenticated,
        user_info,
    })
}

#[tauri::command]
pub async fn platform_auth_login(
    state: tauri::State<'_, AppState>,
    platform: String,
    params: Option<HashMap<String, String>>,
) -> Result<AuthResult, String> {
    let provider = state
        .auth_registry
        .get(&platform)
        .ok_or_else(|| format!("Auth provider not found: {}", platform))?;

    let credentials = params.unwrap_or_default();

    match provider.authenticate(credentials).await {
        Ok(session) => Ok(AuthResult {
            platform,
            success: true,
            user_info: session.user_info,
            error: None,
        }),
        Err(e) => Ok(AuthResult {
            platform,
            success: false,
            user_info: HashMap::new(),
            error: Some(e.to_string()),
        }),
    }
}

#[tauri::command]
pub async fn platform_auth_logout(
    state: tauri::State<'_, AppState>,
    platform: String,
) -> Result<String, String> {
    let provider = state
        .auth_registry
        .get(&platform)
        .ok_or_else(|| format!("Auth provider not found: {}", platform))?;

    provider
        .logout()
        .await
        .map_err(|e| e.to_string())?;

    Ok(format!("Logged out from {}", platform))
}

#[tauri::command]
pub async fn platform_auth_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, String> {
    Ok(state
        .auth_registry
        .list()
        .into_iter()
        .map(|s| s.to_string())
        .collect())
}
