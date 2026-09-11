//! Tools de edição de vídeo que não são gravação de tela: comprimir para um
//! alvo de tamanho, virar GIF/WebP ou figurinha, cortar silêncio e mexer em
//! legenda.
//! A lógica mora em `omniget_core::core::tools`.

use omniget_core::core::tools::{
    silence_cut, sticker, subtitle, video_compress, video_gif, video_restore,
};

use super::{err, progress};

#[tauri::command]
pub async fn tool_video_compress(
    app: tauri::AppHandle,
    opts: video_compress::CompressOptions,
) -> Result<video_compress::CompressResult, String> {
    video_compress::run(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_video_gif(
    app: tauri::AppHandle,
    opts: video_gif::GifOptions,
) -> Result<video_gif::GifResult, String> {
    video_gif::run(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_video_silence(
    app: tauri::AppHandle,
    opts: silence_cut::SilenceOptions,
) -> Result<silence_cut::SilenceResult, String> {
    silence_cut::run(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_subtitle_convert(
    app: tauri::AppHandle,
    opts: subtitle::ConvertOptions,
) -> Result<subtitle::SubtitleResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || subtitle::convert(&opts, &p))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_subtitle_burn(
    app: tauri::AppHandle,
    opts: subtitle::BurnOptions,
) -> Result<String, String> {
    subtitle::burn(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_video_restore(
    app: tauri::AppHandle,
    opts: video_restore::RestoreOptions,
) -> Result<video_restore::RestoreResult, String> {
    video_restore::run(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_sticker(
    app: tauri::AppHandle,
    opts: sticker::StickerOptions,
) -> Result<sticker::StickerResult, String> {
    sticker::run(opts, progress(&app)).await.map_err(err)
}
