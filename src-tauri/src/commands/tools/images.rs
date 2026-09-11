use omniget_core::core::tools::{
    exif, icon_pack, image_compress, image_dupes, image_resize, image_sprite, image_stitch, img_bg,
    onnx, upscale,
};

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

// ── Metadados (img-exif) ───────────────────────────────────────────────

#[tauri::command]
pub async fn tool_exif_read(inputs: Vec<String>) -> Result<Vec<exif::ExifReport>, String> {
    tokio::task::spawn_blocking(move || exif::read_many(&inputs))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_exif_strip(
    app: tauri::AppHandle,
    opts: exif::StripOptions,
) -> Result<exif::StripResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || exif::strip(&opts, &p))
        .await
        .map_err(err)
}

// ── Pacote de ícones (img-icon) ────────────────────────────────────────

#[tauri::command]
pub async fn tool_icon_pack(
    app: tauri::AppHandle,
    opts: icon_pack::IconOptions,
) -> Result<icon_pack::IconResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || icon_pack::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

// ── Fotos duplicadas (img-dedupe) ──────────────────────────────────────

#[tauri::command]
pub async fn tool_img_dupes(
    app: tauri::AppHandle,
    opts: image_dupes::ImgDupesOptions,
) -> Result<image_dupes::ImgDupesResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || image_dupes::scan(&opts, &p))
        .await
        .map_err(err)
}

// ── Comprimir para um tamanho (img-compress) ───────────────────────────

#[tauri::command]
pub async fn tool_img_compress(
    app: tauri::AppHandle,
    opts: image_compress::CompressOptions,
) -> Result<image_compress::CompressResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || image_compress::run(&opts, &p))
        .await
        .map_err(err)
}

// ── Costurar prints (img-stitch) ───────────────────────────────────────

#[tauri::command]
pub async fn tool_img_stitch(
    app: tauri::AppHandle,
    opts: image_stitch::StitchOptions,
) -> Result<image_stitch::StitchResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || image_stitch::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

// ── Spritesheet e lote (img-sprite) ────────────────────────────────────

#[tauri::command]
pub async fn tool_img_sprite(
    app: tauri::AppHandle,
    opts: image_sprite::SpriteOptions,
) -> Result<image_sprite::SpriteResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || image_sprite::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

// ── ONNX: runtime e modelos geridos (infra-onnx) ───────────────────────

#[tauri::command]
pub fn tool_onnx_status(family: Option<String>) -> onnx::OnnxStatus {
    onnx::status(family.as_deref())
}

#[tauri::command]
pub async fn tool_onnx_runtime_install(
    app: tauri::AppHandle,
    variant: Option<String>,
) -> Result<String, String> {
    let p = progress(&app);
    omniget_core::core::onnxrt::install_runtime(variant, &p)
        .await
        .map(|path| path.to_string_lossy().to_string())
        .map_err(err)
}

/// Instala a partir de um `libonnxruntime` que o usuário já tem no disco —
/// único caminho em sistemas sem build oficial (macOS Intel).
#[tauri::command]
pub async fn tool_onnx_runtime_install_local(path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        omniget_core::core::onnxrt::install_from_path(std::path::Path::new(&path))
    })
    .await
    .map_err(err)?
    .map(|p| p.to_string_lossy().to_string())
    .map_err(err)
}

#[tauri::command]
pub async fn tool_onnx_runtime_remove() -> Result<(), String> {
    tokio::task::spawn_blocking(omniget_core::core::onnxrt::remove_managed)
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_onnx_model_download(app: tauri::AppHandle, id: String) -> Result<String, String> {
    let p = progress(&app);
    onnx::ensure_model(&id, &p)
        .await
        .map(|path| path.to_string_lossy().to_string())
        .map_err(err)
}

#[tauri::command]
pub async fn tool_onnx_model_remove(id: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || onnx::remove_model(&id))
        .await
        .map_err(err)?
        .map_err(err)
}

// ── Remover fundo (img-bg) ─────────────────────────────────────────────

#[tauri::command]
pub async fn tool_img_bg(
    app: tauri::AppHandle,
    opts: img_bg::BgOptions,
) -> Result<img_bg::BgResult, String> {
    img_bg::run(opts, progress(&app)).await.map_err(err)
}
