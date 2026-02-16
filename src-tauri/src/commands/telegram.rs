use crate::platforms::telegram::api::{self, TelegramChat, TelegramMediaItem};
use crate::platforms::telegram::auth::{self, VerifyError};
use crate::AppState;

#[tauri::command]
pub async fn telegram_check_session(
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    auth::check_session(&state.telegram_session)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn telegram_send_code(
    state: tauri::State<'_, AppState>,
    phone: String,
) -> Result<(), String> {
    auth::send_code(&state.telegram_session, &phone)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn telegram_verify_code(
    state: tauri::State<'_, AppState>,
    code: String,
) -> Result<String, String> {
    match auth::verify_code(&state.telegram_session, &code).await {
        Ok(phone) => Ok(phone),
        Err(VerifyError::InvalidCode) => Err("invalid_code".to_string()),
        Err(VerifyError::PasswordRequired { hint }) => {
            Err(format!("password_required:{}", hint))
        }
        Err(VerifyError::NoSession) => Err("no_session".to_string()),
        Err(VerifyError::Other(msg)) => Err(msg),
    }
}

#[tauri::command]
pub async fn telegram_verify_2fa(
    state: tauri::State<'_, AppState>,
    password: String,
) -> Result<String, String> {
    auth::verify_password(&state.telegram_session, &password)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn telegram_logout(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    auth::logout(&state.telegram_session)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn telegram_list_chats(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<TelegramChat>, String> {
    api::list_chats(&state.telegram_session)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn telegram_list_media(
    state: tauri::State<'_, AppState>,
    chat_id: i64,
    chat_type: String,
    media_type: Option<String>,
    offset: i32,
    limit: u32,
) -> Result<Vec<TelegramMediaItem>, String> {
    api::list_media(
        &state.telegram_session,
        chat_id,
        &chat_type,
        media_type.as_deref(),
        offset,
        limit,
    )
    .await
    .map_err(|e| e.to_string())
}
