//! Comandos da categoria Twitch. Tudo aqui usa só endpoint público: o
//! GraphQL do site com o Client-ID do player web, mais as APIs abertas de
//! BTTV, FFZ e 7TV. Nenhum login, nenhum token de app.

use omniget_core::core::tools::twitch::{chat, emotes};

use super::{err, progress};

/// Bulk-download de emotes e badges de um canal (e dos conjuntos globais).
#[tauri::command]
pub async fn tool_tw_emotes(
    app: tauri::AppHandle,
    opts: emotes::Options,
) -> Result<emotes::Result, String> {
    emotes::run(&opts, &progress(&app)).await.map_err(err)
}

/// Replay de chat de um VOD ou clipe exportado em JSON/CSV/SRT/ASS.
#[tauri::command]
pub async fn tool_tw_chat_replay(
    app: tauri::AppHandle,
    opts: chat::Options,
) -> Result<chat::Result, String> {
    chat::export(&opts, &progress(&app)).await.map_err(err)
}
