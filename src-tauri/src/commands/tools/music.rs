//! Comandos da categoria Música: playlist -> m3u8/pls, letra sincronizada
//! e análise do histórico do Spotify.

use omniget_core::core::tools::music::{history, lyrics, playlist};

use super::{err, progress};

#[tauri::command]
pub async fn tool_music_playlist(
    app: tauri::AppHandle,
    opts: playlist::PlaylistOptions,
) -> Result<playlist::PlaylistResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || playlist::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_lyrics_sync(
    app: tauri::AppHandle,
    opts: lyrics::LyricsOptions,
) -> Result<lyrics::LyricsResult, String> {
    lyrics::run(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_music_history(
    app: tauri::AppHandle,
    opts: history::HistoryOptions,
) -> Result<history::HistoryResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || history::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}
