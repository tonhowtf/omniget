use serde::Serialize;
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use omniget_core::platforms::p2p;
use crate::{AppState, P2pSendHandle};

#[derive(Clone, Serialize)]
pub struct P2pSendStarted {
    pub code: String,
    pub file_name: String,
    pub file_size: u64,
}

#[derive(Clone, Serialize)]
pub struct P2pSendProgress {
    pub code: String,
    pub progress: f64,
    pub status: String,
    pub sent_bytes: u64,
    pub total_bytes: u64,
    pub speed_bytes_per_sec: f64,
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn p2p_send_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    file_path: String,
) -> Result<P2pSendStarted, String> {
    let path = std::path::PathBuf::from(&file_path);
    let cancel_token = CancellationToken::new();

    let session = p2p::start_send(path, cancel_token.clone())
        .await
        .map_err(|e| e.to_string())?;

    let result = P2pSendStarted {
        code: session.code.clone(),
        file_name: session.file_name.clone(),
        file_size: session.file_size,
    };

    let code = session.code.clone();
    let paused_ref = session.paused.clone();
    {
        let mut sends = state.active_p2p_sends.lock().await;
        sends.insert(
            code.clone(),
            P2pSendHandle {
                cancel_token,
                paused: paused_ref,
            },
        );
    }

    let app_clone = app.clone();
    let state_sends = state.active_p2p_sends.clone();
    let progress_ref = session.progress.clone();
    let status_ref = session.status.clone();
    let code_for_task = code.clone();

    tokio::spawn(async move {
        let app_progress = app_clone.clone();
        let code_progress = code_for_task.clone();
        let progress_for_poll = progress_ref.clone();
        let status_for_poll = status_ref.clone();
        let cancel_for_poll = session.cancel_token.clone();
        let paused_for_poll = session.paused.clone();

        let total_bytes = session.file_size;
        let sent_bytes_ref = session.sent_bytes.clone();
        let progress_poller = tokio::spawn(async move {
            let mut last_bytes: u64 = 0;
            let mut last_time = std::time::Instant::now();
            let mut smoothed_speed: f64 = 0.0;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                        let pct = *progress_for_poll.lock().await;
                        let is_paused = paused_for_poll.load(std::sync::atomic::Ordering::Relaxed);
                        let status = if is_paused {
                            "paused".to_string()
                        } else {
                            status_for_poll.lock().await.clone()
                        };
                        let sent = *sent_bytes_ref.lock().await;

                        let now = std::time::Instant::now();
                        let dt = now.duration_since(last_time).as_secs_f64();
                        let instant_speed = if dt > 0.05 {
                            (sent.saturating_sub(last_bytes)) as f64 / dt
                        } else {
                            0.0
                        };
                        smoothed_speed = if smoothed_speed > 0.0 {
                            smoothed_speed * 0.7 + instant_speed * 0.3
                        } else {
                            instant_speed
                        };
                        last_bytes = sent;
                        last_time = now;

                        let _ = app_progress.emit("p2p-send-progress", P2pSendProgress {
                            code: code_progress.clone(),
                            progress: pct,
                            status,
                            sent_bytes: sent,
                            total_bytes,
                            speed_bytes_per_sec: smoothed_speed,
                        });
                        if pct >= 100.0 {
                            break;
                        }
                    }
                    _ = cancel_for_poll.cancelled() => {
                        break;
                    }
                }
            }
        });

        let result = p2p::run_sender(&session).await;
        progress_poller.abort();

        {
            let mut sends = state_sends.lock().await;
            sends.remove(&code_for_task);
        }

        match result {
            Ok(()) => {
                let _ = app_clone.emit(
                    "p2p-send-complete",
                    serde_json::json!({
                        "code": code_for_task,
                        "success": true,
                    }),
                );
            }
            Err(e) => {
                let err = e.to_string();
                tracing::error!("[p2p] send failed: {}", err);
                let _ = app_clone.emit(
                    "p2p-send-complete",
                    serde_json::json!({
                        "code": code_for_task,
                        "success": false,
                        "error": err,
                    }),
                );
            }
        }
    });

    Ok(result)
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn p2p_cancel_send(
    state: tauri::State<'_, AppState>,
    code: String,
) -> Result<String, String> {
    let mut sends = state.active_p2p_sends.lock().await;
    match sends.remove(&code) {
        Some(handle) => {
            handle.cancel_token.cancel();
            Ok("Send cancelled".to_string())
        }
        None => Err("No active send with this code".to_string()),
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn p2p_pause_send(
    state: tauri::State<'_, AppState>,
    code: String,
) -> Result<String, String> {
    let sends = state.active_p2p_sends.lock().await;
    match sends.get(&code) {
        Some(handle) => {
            handle
                .paused
                .store(true, std::sync::atomic::Ordering::Relaxed);
            Ok("Send paused".to_string())
        }
        None => Err("No active send with this code".to_string()),
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn p2p_resume_send(
    state: tauri::State<'_, AppState>,
    code: String,
) -> Result<String, String> {
    let sends = state.active_p2p_sends.lock().await;
    match sends.get(&code) {
        Some(handle) => {
            handle
                .paused
                .store(false, std::sync::atomic::Ordering::Relaxed);
            Ok("Send resumed".to_string())
        }
        None => Err("No active send with this code".to_string()),
    }
}
