//! Maintenance with a real effect: DNS cache, LaunchServices, Spotlight, Quick Look, verify disk, sfc/DISM, fstrim (F7).
//!
//! Stub created by F0; the module owner fills it in.

use serde_json::Value;

use super::elevate::ElevatedOp;
use super::plan::Outcome;
use super::receipt::Undo;
use super::store::Ctx;

// ── Plan hooks (called by `plan::execute` for `Op::Module { module: "maintenance" }`) ──

pub fn plan_execute(_ctx: &Ctx, _receipt_id: &str, action: &str, _args: &Value) -> Result<Outcome, String> {
    Err(format!("maintenance: unknown action {action}"))
}

pub fn plan_elevated(_ctx: &Ctx, _receipt_id: &str, action: &str, _args: &Value) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    Err(format!("maintenance: action {action} has no administrator form"))
}

pub fn plan_undo(_ctx: &Ctx, action: &str, _args: &Value) -> Result<String, String> {
    Err(format!("maintenance: cannot undo {action}"))
}

pub fn elevated_run(_id: &str) -> Result<String, String> {
    Err("maintenance: not available in this build yet".into())
}

/// The command line of maintenance task `id` (display only).
pub fn command_line(id: &str) -> String {
    format!("# maintenance {id}")
}

/// Actions only the screen may run (`washmybelly_ui` with `maintenance.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    Err(format!("maintenance: unknown screen action {action}"))
}
