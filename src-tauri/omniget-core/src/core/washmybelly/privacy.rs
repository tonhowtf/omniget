//! Privacy items compiled from privacy.sexy (AGPL-3.0) with state read from the system, snapshot and undo; TCC on macOS (F4).
//!
//! Stub created by F0; the module owner fills it in.

use serde_json::Value;

use super::elevate::ElevatedOp;
use super::plan::Outcome;
use super::receipt::Undo;
use super::store::Ctx;

use super::tools::not_ready;

pub async fn tool_catalog(_a: Value) -> Result<Value, String> {
    Err(not_ready("wmb_privacy_catalog"))
}

// ── Plan hooks (called by `plan::execute` for `Op::Module { module: "privacy" }`) ──

pub fn plan_execute(
    _ctx: &Ctx,
    _receipt_id: &str,
    action: &str,
    _args: &Value,
) -> Result<Outcome, String> {
    Err(format!("privacy: unknown action {action}"))
}

pub fn plan_elevated(
    _ctx: &Ctx,
    _receipt_id: &str,
    action: &str,
    _args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    Err(format!(
        "privacy: action {action} has no administrator form"
    ))
}

pub fn plan_undo(_ctx: &Ctx, action: &str, _args: &Value) -> Result<String, String> {
    Err(format!("privacy: cannot undo {action}"))
}

/// Actions only the screen may run (`washmybelly_ui` with `privacy.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    Err(format!("privacy: unknown screen action {action}"))
}
