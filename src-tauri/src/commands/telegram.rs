use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::platforms::telegram::api::{self, TelegramChat, TelegramMediaItem};
use crate::platforms::telegram::auth::{self, QrPollStatus, VerifyError};
use crate::storage::config;
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
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    chat_id: i64,
    chat_type: String,
    media_type: Option<String>,
    offset: i32,
    limit: u32,
) -> Result<Vec<TelegramMediaItem>, String> {
    let fix_extensions = config::load_settings(&app).telegram.fix_file_extensions;
    api::list_media(
        &state.telegram_session,
        chat_id,
        &chat_type,
        media_type.as_deref(),
        offset,
        limit,
        fix_extensions,
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

#[derive(Clone, Deserialize)]
pub struct BatchItem {
    pub message_id: i32,
    pub file_name: String,
    pub file_size: u64,
}

#[derive(Clone, Serialize)]
struct BatchFileStatus {
    batch_id: u64,
    message_id: i32,
    status: String, // "waiting" | "downloading" | "done" | "error" | "skipped"
    percent: f64,
    error: Option<String>,
}

#[tauri::command]
pub async fn telegram_download_batch(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    chat_id: i64,
    chat_type: String,
    chat_title: String,
    items: Vec<BatchItem>,
    output_dir: String,
) -> Result<u64, String> {
    let batch_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let cancel_token = CancellationToken::new();

    {
        let mut active = state.active_generic_downloads.lock().await;
        let key = format!("tg-batch:{}", batch_id);
        active.insert(batch_id, (key, cancel_token.clone()));
    }

    let session = state.telegram_session.clone();
    let active_downloads = state.active_generic_downloads.clone();
    let total_files = items.len() as u32;

    // Read Telegram settings
    let tg_settings = config::load_settings(&app).telegram;
    let concurrent = tg_settings.concurrent_downloads.max(1).min(10) as usize;

    // Emit initial 0% batch progress
    let _ = app.emit("generic-download-progress", &GenericDownloadProgress {
        id: batch_id,
        title: chat_title.clone(),
        platform: "telegram".to_string(),
        percent: 0.0,
    });

    tokio::spawn(async move {
        let semaphore = Arc::new(Semaphore::new(concurrent));
        let completed = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let failed = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let skipped = Arc::new(std::sync::atomic::AtomicU32::new(0));

        let mut handles = Vec::new();

        for item in items {
            let sem = semaphore.clone();
            let session = session.clone();
            let app = app.clone();
            let chat_type = chat_type.clone();
            let chat_title = chat_title.clone();
            let output_dir = output_dir.clone();
            let cancel = cancel_token.clone();
            let completed = completed.clone();
            let failed = failed.clone();
            let skipped = skipped.clone();

            let handle = tokio::spawn(async move {
                // Check cancellation before acquiring permit
                if cancel.is_cancelled() {
                    return;
                }

                let output_path = std::path::PathBuf::from(&output_dir).join(&item.file_name);

                // Skip existing files
                if let Ok(true) = tokio::fs::try_exists(&output_path).await {
                    if let Ok(meta) = tokio::fs::metadata(&output_path).await {
                        if meta.len() > 0 {
                            skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let done = completed.load(std::sync::atomic::Ordering::Relaxed)
                                + failed.load(std::sync::atomic::Ordering::Relaxed)
                                + skipped.load(std::sync::atomic::Ordering::Relaxed);
                            let percent = (done as f64 / total_files as f64) * 100.0;

                            let _ = app.emit("telegram-batch-file-status", &BatchFileStatus {
                                batch_id,
                                message_id: item.message_id,
                                status: "skipped".to_string(),
                                percent: 100.0,
                                error: None,
                            });
                            let _ = app.emit("generic-download-progress", &GenericDownloadProgress {
                                id: batch_id,
                                title: chat_title,
                                platform: "telegram".to_string(),
                                percent,
                            });
                            return;
                        }
                    }
                }

                // Acquire semaphore permit
                let _permit = tokio::select! {
                    p = sem.acquire() => match p {
                        Ok(p) => p,
                        Err(_) => return,
                    },
                    _ = cancel.cancelled() => return,
                };

                if cancel.is_cancelled() {
                    return;
                }

                // Emit downloading status
                let _ = app.emit("telegram-batch-file-status", &BatchFileStatus {
                    batch_id,
                    message_id: item.message_id,
                    status: "downloading".to_string(),
                    percent: 0.0,
                    error: None,
                });

                let (tx, mut rx) = mpsc::channel::<f64>(32);

                let app_progress = app.clone();
                let progress_forwarder = tokio::spawn(async move {
                    while let Some(percent) = rx.recv().await {
                        let _ = app_progress.emit("telegram-batch-file-status", &BatchFileStatus {
                            batch_id,
                            message_id: item.message_id,
                            status: "downloading".to_string(),
                            percent,
                            error: None,
                        });
                    }
                });

                let result = api::download_media_with_retry(
                    &session,
                    chat_id,
                    &chat_type,
                    item.message_id,
                    &output_path,
                    tx,
                    &cancel,
                ).await;

                let _ = progress_forwarder.await;

                match result {
                    Ok(_) => {
                        completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let _ = app.emit("telegram-batch-file-status", &BatchFileStatus {
                            batch_id,
                            message_id: item.message_id,
                            status: "done".to_string(),
                            percent: 100.0,
                            error: None,
                        });
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str.contains("cancelled") {
                            return;
                        }
                        failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let _ = app.emit("telegram-batch-file-status", &BatchFileStatus {
                            batch_id,
                            message_id: item.message_id,
                            status: "error".to_string(),
                            percent: 0.0,
                            error: Some(err_str),
                        });
                    }
                }

                let done = completed.load(std::sync::atomic::Ordering::Relaxed)
                    + failed.load(std::sync::atomic::Ordering::Relaxed)
                    + skipped.load(std::sync::atomic::Ordering::Relaxed);
                let percent = (done as f64 / total_files as f64) * 100.0;

                let _ = app.emit("generic-download-progress", &GenericDownloadProgress {
                    id: batch_id,
                    title: chat_title,
                    platform: "telegram".to_string(),
                    percent,
                });
            });

            handles.push(handle);
        }

        // Wait for all file downloads to complete
        for h in handles {
            let _ = h.await;
        }

        // Clean up active download entry
        {
            active_downloads.lock().await.remove(&batch_id);
        }

        let done_count = completed.load(std::sync::atomic::Ordering::Relaxed);
        let fail_count = failed.load(std::sync::atomic::Ordering::Relaxed);
        let skip_count = skipped.load(std::sync::atomic::Ordering::Relaxed);
        let success = fail_count == 0 && !cancel_token.is_cancelled();

        let _ = app.emit("generic-download-complete", &GenericDownloadComplete {
            id: batch_id,
            title: chat_title,
            platform: "telegram".to_string(),
            success,
            error: if cancel_token.is_cancelled() {
                Some("Batch cancelled".to_string())
            } else if fail_count > 0 {
                Some(format!("{} files failed", fail_count))
            } else {
                None
            },
            file_path: Some(output_dir),
            file_size_bytes: None,
            file_count: Some(done_count + skip_count),
        });
    });

    Ok(batch_id)
}

#[tauri::command]
pub async fn telegram_cancel_batch(
    state: tauri::State<'_, AppState>,
    batch_id: u64,
) -> Result<(), String> {
    let active = state.active_generic_downloads.lock().await;
    if let Some((key, token)) = active.get(&batch_id) {
        if key.starts_with("tg-batch:") {
            token.cancel();
            return Ok(());
        }
    }
    Err("Batch not found".to_string())
}
