//! Categoria LinkedIn: quatro leitores do export oficial de dados. Tudo roda
//! em cima de arquivo local, sem nenhuma chamada de rede.

use omniget_core::core::tools::linkedin::{checklist, connections, messages, overview};

use super::{err, progress};

#[tauri::command]
pub async fn tool_li_overview(
    app: tauri::AppHandle,
    opts: overview::Options,
) -> Result<overview::Overview, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || overview::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_li_connections(
    app: tauri::AppHandle,
    opts: connections::Options,
) -> Result<connections::ConnectionsResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || connections::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_li_messages(
    app: tauri::AppHandle,
    opts: messages::Options,
) -> Result<messages::MessagesResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || messages::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_li_checklist(
    app: tauri::AppHandle,
    opts: checklist::Options,
) -> Result<checklist::ChecklistResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || checklist::run(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}
