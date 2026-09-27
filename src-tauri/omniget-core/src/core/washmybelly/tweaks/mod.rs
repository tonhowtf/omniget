//! Windows tweaks as data, with state read from the system and snapshots (F4).
//!
//! Stub created by F0; the module owner fills it in.

use serde_json::Value;

use super::elevate::ElevatedOp;
use super::plan::Outcome;
use super::receipt::Undo;
use super::store::Ctx;

use super::tools::not_ready;

pub async fn tool_list(_a: Value) -> Result<Value, String> {
    Err(not_ready("wmb_tweaks_list"))
}

// ── Plan hooks (called by `plan::execute` for `Op::Module { module: "tweaks" }`) ──

pub fn plan_execute(_ctx: &Ctx, _receipt_id: &str, action: &str, _args: &Value) -> Result<Outcome, String> {
    Err(format!("tweaks: unknown action {action}"))
}

pub fn plan_elevated(_ctx: &Ctx, _receipt_id: &str, action: &str, _args: &Value) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    Err(format!("tweaks: action {action} has no administrator form"))
}

pub fn plan_undo(_ctx: &Ctx, action: &str, _args: &Value) -> Result<String, String> {
    Err(format!("tweaks: cannot undo {action}"))
}

pub fn elevated_reg_set(_key: &str, _name: &str, _kind: &str, _data: &str) -> Result<String, String> {
    Err("reg_set: not available in this build yet".into())
}

pub fn elevated_reg_delete(_key: &str, _name: Option<&str>) -> Result<String, String> {
    Err("reg_delete: not available in this build yet".into())
}

/// Actions only the screen may run (`washmybelly_ui` with `tweaks.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    Err(format!("tweaks: unknown screen action {action}"))
}
