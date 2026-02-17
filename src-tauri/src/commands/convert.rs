use serde::Serialize;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::core::ffmpeg::{self, ConversionOptions, ConversionResult, MediaProbeInfo};
use crate::core::hwaccel::{self, HwAccelInfo};
use crate::AppState;

#[derive(Clone, Serialize)]
struct ConvertProgressEvent {
    id: u64,
    percent: f64,
}

#[derive(Clone, Serialize)]
struct ConvertCompleteEvent {
    id: u64,
    success: bool,
    result: Option<ConversionResult>,
    error: Option<String>,
}

#[tauri::command]
pub async fn probe_file(path: String) -> Result<MediaProbeInfo, String> {
    ffmpeg::probe(std::path::Path::new(&path))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn convert_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    options: ConversionOptions,
) -> Result<u64, String> {
    let conversion_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let cancel_token = CancellationToken::new();

    {
        let mut map = state.active_conversions.lock().await;
        map.insert(conversion_id, cancel_token.clone());
    }

    let active = state.active_conversions.clone();

    tokio::spawn(async move {
        let (tx, mut rx) = mpsc::channel::<f64>(32);

        let app_progress = app.clone();
        let cid = conversion_id;
        let progress_forwarder = tokio::spawn(async move {
            while let Some(percent) = rx.recv().await {
                let _ = app_progress.emit(
                    "convert-progress",
                    ConvertProgressEvent {
                        id: cid,
                        percent,
                    },
                );
            }
        });

        let result = ffmpeg::convert(&options, cancel_token, tx).await;

        let _ = progress_forwarder.await;

        {
            let mut map = active.lock().await;
            map.remove(&conversion_id);
        }

        match result {
            Ok(conv_result) => {
                let success = conv_result.success;
                let error = conv_result.error.clone();
                let _ = app.emit(
                    "convert-complete",
                    ConvertCompleteEvent {
                        id: conversion_id,
                        success,
                        result: Some(conv_result),
                        error,
                    },
                );
            }
            Err(e) => {
                let _ = app.emit(
                    "convert-complete",
                    ConvertCompleteEvent {
                        id: conversion_id,
                        success: false,
                        result: None,
                        error: Some(e.to_string()),
                    },
                );
            }
        }
    });

    Ok(conversion_id)
}

#[tauri::command]
pub async fn cancel_conversion(
    state: tauri::State<'_, AppState>,
    conversion_id: u64,
) -> Result<String, String> {
    let mut map = state.active_conversions.lock().await;
    match map.remove(&conversion_id) {
        Some(token) => {
            token.cancel();
            Ok("Conversão cancelada".to_string())
        }
        None => Err("Nenhuma conversão ativa para este ID".to_string()),
    }
}

#[tauri::command]
pub async fn get_hwaccel_info() -> Result<HwAccelInfo, String> {
    Ok(hwaccel::detect_hwaccel().await)
}
