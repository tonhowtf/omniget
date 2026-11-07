use omniget_core::core::sessions::api::{self, SourceInfo};
use omniget_core::core::sessions::index::{ListFilter, ListPage};

#[tauri::command]
pub async fn sessions_list(filter: Option<ListFilter>) -> Result<ListPage, String> {
    api::list(filter.unwrap_or_default()).await
}

#[tauri::command]
pub async fn sessions_sources() -> Result<Vec<SourceInfo>, String> {
    api::sources_info().await
}
