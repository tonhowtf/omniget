//! Comandos da categoria Bilibili: export dos comentários flutuantes
//! (danmaku) e queima do ASS num clipe. O motor mora em
//! `crate::platforms::bilibili::danmaku`; aqui só entra a ponte com o Tauri.

use crate::platforms::bilibili::danmaku::{burn, export};

use super::{err, progress};

#[tauri::command]
pub async fn tool_bili_danmaku_export(
    app: tauri::AppHandle,
    opts: export::ExportOptions,
) -> Result<export::ExportResult, String> {
    export::run(&opts, &progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_bili_danmaku_burn(
    app: tauri::AppHandle,
    opts: burn::BurnOptions,
) -> Result<burn::BurnResult, String> {
    burn::run(&opts, &progress(&app)).await.map_err(err)
}
