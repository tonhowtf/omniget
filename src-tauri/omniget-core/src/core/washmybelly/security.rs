//! Persistence inventory on macOS: launchd, background tasks, helpers, profiles, extensions (F7).
//!
//! Stub created by F0; the module owner fills it in.

use serde_json::Value;

use super::tools::not_ready;

pub async fn tool_persistence(_a: Value) -> Result<Value, String> {
    Err(not_ready("wmb_persistence_list"))
}

/// Actions only the screen may run (`washmybelly_ui` with `security.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    match action {
        // Full Disk Access cannot be asked for; the person grants it in
        // System Settings, which this opens on the right pane.
        "open_fda_settings" => {
            #[cfg(target_os = "macos")]
            {
                crate::core::process::std_command("open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
                    .status()
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "opened": true }))
            }
            #[cfg(not(target_os = "macos"))]
            Err("Full Disk Access exists only on macOS".into())
        }
        _ => Err(format!("security: unknown screen action {action}")),
    }
}
