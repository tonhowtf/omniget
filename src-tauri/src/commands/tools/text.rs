//! Ferramentas de texto da seção Tools.

use omniget_core::core::tools::humanize;

#[tauri::command]
pub async fn tool_humanize(text: String, sample: Option<String>) -> Result<String, String> {
    humanize::humanize(&text, sample.as_deref()).await
}
