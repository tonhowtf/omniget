//! FPS and frametime measurement: PresentMon (Windows), MangoHud (Linux), Metal HUD (macOS); format `omniget-frames/1` (F6).
//!
//! Stub created by F0; the module owner fills it in.

use serde_json::Value;

use super::tools::not_ready;

pub async fn tool_capabilities(_a: Value) -> Result<Value, String> {
    Err(not_ready("wmb_perf_capabilities"))
}

pub async fn tool_capture_start(_a: Value) -> Result<Value, String> {
    Err(not_ready("wmb_perf_capture_start"))
}

pub async fn tool_capture_stop(_a: Value) -> Result<Value, String> {
    Err(not_ready("wmb_perf_capture_stop"))
}

pub async fn tool_sessions(_a: Value) -> Result<Value, String> {
    Err(not_ready("wmb_perf_sessions"))
}

pub async fn tool_summary(_a: Value) -> Result<Value, String> {
    Err(not_ready("wmb_perf_summary"))
}

pub async fn tool_compare(_a: Value) -> Result<Value, String> {
    Err(not_ready("wmb_perf_compare"))
}

/// Actions only the screen may run (`washmybelly_ui` with `perf.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    Err(format!("perf: unknown screen action {action}"))
}
