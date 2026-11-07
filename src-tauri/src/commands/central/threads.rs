use omniget_core::core::threads::external::{self, ExternalImport};
use tauri::AppHandle;

use crate::threads_host::{self};

#[tauri::command]
pub async fn threads_import_external(
    app: AppHandle,
    tool: String,
    session_id: String,
) -> Result<ExternalImport, String> {
    let host = threads_host::get(&app)?;
    external::import_external(&host.engine, &tool, &session_id).await
}
