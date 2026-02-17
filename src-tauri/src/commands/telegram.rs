use serde::Serialize;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::platforms::telegram::api::{self, TelegramChat, TelegramMediaItem};
use crate::platforms::telegram::auth::{self, QrPollStatus, VerifyError};
use crate::AppState;

#[derive(Clone, Serialize)]
struct GenericDownloadProgress {
    id: u64,
    title: String,
    platform: String,
    percent: f64,
}

#[derive(Clone, Serialize)]
struct GenericDownloadComplete {
    id: u64,
    title: String,
    platform: String,
    success: bool,
    error: Option<String>,
    file_path: Option<String>,
    file_size_bytes: Option<u64>,
    file_count: Option<u32>,
}

#[derive(Clone, Serialize)]
pub struct TelegramDownloadStarted {
    pub id: u64,
    pub file_name: String,
}

#[tauri::command]
pub async fn telegram_check_session(
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    auth::check_session(&state.telegram_session)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Clone, Serialize)]
pub struct QrStartResponse {
    pub svg: String,
    pub expires: i32,
}

#[tauri::command]
pub async fn telegram_qr_start(
    state: tauri::State<'_, AppState>,
) -> Result<QrStartResponse, String> {
    let result = auth::qr_login_start(&state.telegram_session)
        .await
        .map_err(|e| e.to_string())?;
    Ok(QrStartResponse {
        svg: result.qr_svg,
        expires: result.expires,
    })
}

#[tauri::command]
pub async fn telegram_qr_poll(
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    match auth::qr_login_poll(&state.telegram_session).await {
        Ok(QrPollStatus::Waiting) => Ok("waiting".to_string()),
        Ok(QrPollStatus::Success { phone }) => Ok(format!("success:{}", phone)),
        Ok(QrPollStatus::PasswordRequired { hint }) => {
            if hint.is_empty() {
                Ok("password_required".to_string())
            } else {
                Ok(format!("password_required:{}", hint))
            }
        }
        Err(e) => Err(e.to_string()),
    }
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

#[tauri::command]
pub async fn telegram_download_media(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    chat_id: i64,
    chat_type: String,
    message_id: i32,
    file_name: String,
    output_dir: String,
) -> Result<TelegramDownloadStarted, String> {
    let download_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let cancel_token = CancellationToken::new();

    {
        let mut active = state.active_generic_downloads.lock().await;
        let key = format!("tg:{}:{}", chat_id, message_id);
        if active.values().any(|(u, _)| u == &key) {
            return Err("Download já em andamento para esta mídia".to_string());
        }
        active.insert(download_id, (key, cancel_token.clone()));
    }

    let session = state.telegram_session.clone();
    let active_downloads = state.active_generic_downloads.clone();
    let file_name_clone = file_name.clone();

    tokio::spawn(async move {
        let output_path = std::path::PathBuf::from(&output_dir).join(&file_name_clone);

        let _ = app.emit("generic-download-progress", &GenericDownloadProgress {
            id: download_id,
            title: file_name_clone.clone(),
            platform: "telegram".to_string(),
            percent: 0.0,
        });

        let (tx, mut rx) = mpsc::channel::<f64>(32);

        let app_progress = app.clone();
        let file_name_progress = file_name_clone.clone();
        let progress_forwarder = tokio::spawn(async move {
            while let Some(percent) = rx.recv().await {
                let _ = app_progress.emit("generic-download-progress", &GenericDownloadProgress {
                    id: download_id,
                    title: file_name_progress.clone(),
                    platform: "telegram".to_string(),
                    percent,
                });
            }
        });

        let result = tokio::select! {
            r = api::download_media(
                &session,
                chat_id,
                &chat_type,
                message_id,
                &output_path,
                tx,
            ) => r,
            _ = cancel_token.cancelled() => {
                Err(anyhow::anyhow!("Download cancelado"))
            }
        };

        let _ = progress_forwarder.await;

        {
            active_downloads.lock().await.remove(&download_id);
        }

        match result {
            Ok(size) => {
                let _ = app.emit("generic-download-complete", &GenericDownloadComplete {
                    id: download_id,
                    title: file_name_clone,
                    platform: "telegram".to_string(),
                    success: true,
                    error: None,
                    file_path: Some(output_path.to_string_lossy().to_string()),
                    file_size_bytes: Some(size),
                    file_count: Some(1),
                });
            }
            Err(e) => {
                let _ = app.emit("generic-download-complete", &GenericDownloadComplete {
                    id: download_id,
                    title: file_name_clone,
                    platform: "telegram".to_string(),
                    success: false,
                    error: Some(e.to_string()),
                    file_path: None,
                    file_size_bytes: None,
                    file_count: None,
                });
            }
        }
    });

    Ok(TelegramDownloadStarted { id: download_id, file_name })
}
