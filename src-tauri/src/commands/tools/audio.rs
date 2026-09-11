//! Tools de áudio: normalizar loudness (EBU R128), tirar ruído e etiquetar
//! metadados de música (ID3 / Vorbis / MP4).

use omniget_core::core::tools::{audio_clean, audio_tag};

use super::{err, progress};

#[tauri::command]
pub async fn tool_audio_clean(
    app: tauri::AppHandle,
    opts: audio_clean::CleanOptions,
) -> Result<audio_clean::CleanResult, String> {
    audio_clean::run(opts, progress(&app)).await.map_err(err)
}

// ── Etiquetas de música (ID3 / Vorbis / MP4), via lofty ──────────────────

/// Lê as tags de uma pasta ou lista de arquivos e já devolve o diagnóstico.
#[tauri::command]
pub async fn tool_audio_tag_scan(
    app: tauri::AppHandle,
    opts: audio_tag::ScanOptions,
) -> Result<audio_tag::ScanResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || audio_tag::scan(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

/// Aplica a edição em lote. Com `dry_run` (o padrão) devolve só o diff, sem
/// tocar em nenhum arquivo.
#[tauri::command]
pub async fn tool_audio_tag_write(
    app: tauri::AppHandle,
    opts: audio_tag::EditOptions,
) -> Result<audio_tag::EditResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || audio_tag::edit(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

/// Capa embutida de um arquivo, em `data:` URL, para a miniatura da UI.
#[tauri::command]
pub async fn tool_audio_tag_cover(path: String) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || audio_tag::cover_data_url(&path))
        .await
        .map_err(err)?
        .map_err(err)
}
