//! Comandos Tauri da seção Tools. Cada submódulo é um grupo da UI
//! (fala, IA, YouTube, documentos, imagens, arquivos, downloads, sistema,
//! celular); a lógica mora em `omniget_core::core::tools`. Progresso sai
//! pelo evento `tool-progress` com o payload `ToolProgress`.

pub mod ai;
pub mod desktop;
pub mod documents;
pub mod downloads;
pub mod files;
pub mod images;
pub mod instagram;
pub mod pdf;
pub mod phone;
pub mod pinterest;
pub mod speech;
pub mod system;
pub mod text;
pub mod x;
pub mod youtube;

use std::sync::Arc;

use omniget_core::core::tools::ProgressFn;
use tauri::Emitter;

pub(crate) fn progress(app: &tauri::AppHandle) -> ProgressFn {
    let app = app.clone();
    Arc::new(move |p| {
        let _ = app.emit("tool-progress", p);
    })
}

pub(crate) fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
