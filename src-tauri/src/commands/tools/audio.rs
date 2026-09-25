//! Tools de áudio: normalizar loudness (EBU R128), tirar ruído e etiquetar
//! metadados de música (ID3 / Vorbis / MP4).

use omniget_core::core::tools::audio_clean;

use super::{err, progress};

#[tauri::command]
pub async fn tool_audio_clean(
    app: tauri::AppHandle,
    opts: audio_clean::CleanOptions,
) -> Result<audio_clean::CleanResult, String> {
    audio_clean::run(opts, progress(&app)).await.map_err(err)
}

// ── Etiquetas de música (ID3 / Vorbis / MP4), via lofty ──────────────────
