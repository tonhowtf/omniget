use omniget_core::core::tools::{image_resize, upscale};

use super::{err, progress};

#[tauri::command]
pub fn tool_upscale_status() -> upscale::UpscaleStatus {
    upscale::status()
}

#[tauri::command]
pub async fn tool_upscale_install(app: tauri::AppHandle) -> Result<String, String> {
    upscale::install(progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_upscale_run(
    app: tauri::AppHandle,
    opts: upscale::UpscaleOptions,
) -> Result<upscale::UpscaleResult, String> {
    upscale::run(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_resize(
    app: tauri::AppHandle,
    opts: image_resize::ResizeOptions,
) -> Result<image_resize::ResizeResult, String> {
    image_resize::run(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_ocr_status() -> omniget_core::core::tools::ocr::OcrStatus {
    omniget_core::core::tools::ocr::status().await
}

#[tauri::command]
pub async fn tool_ocr_run(
    app: tauri::AppHandle,
    inputs: Vec<String>,
    langs: String,
) -> Result<Vec<omniget_core::core::tools::ocr::OcrResult>, String> {
    omniget_core::core::tools::ocr::run(&inputs, &langs, progress(&app))
        .await
        .map_err(err)
}
