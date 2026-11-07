#[tauri::command]
pub async fn assist_mcp_derived_bot_revoke(bot: String) -> Result<(), String> {
    let db = omniget_core::core::assist::db::global()?;
    omniget_core::core::assist::external_config::retire_derived(&db, &bot)
}
