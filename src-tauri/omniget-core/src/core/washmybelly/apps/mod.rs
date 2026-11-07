pub mod linux;
pub mod macos;
pub mod windows;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::elevate::ElevatedOp;
use super::plan::{self, Outcome, PlanItem};
use super::receipt::Undo;
use super::store::Ctx;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct App {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    pub source: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default)]
    pub native_id: Option<String>,
    #[serde(default)]
    pub installed: Option<i64>,
    #[serde(default)]
    pub last_used: Option<i64>,
    pub removable: bool,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub needs_admin: bool,
    #[serde(default)]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Leftover {
    pub path: String,
    pub bytes: u64,
    pub confidence: String,
    pub evidence: String,
    pub kind: String,
}

pub fn list() -> Result<Vec<App>, String> {
    #[cfg(target_os = "macos")]
    return macos::list();
    #[cfg(windows)]
    return windows::list();
    #[cfg(target_os = "linux")]
    return linux::list();
    #[allow(unreachable_code)]
    Ok(Vec::new())
}

pub fn uninstall_items(ctx: &Ctx, app: &App) -> Result<(Vec<PlanItem>, Value), String> {
    let _ = ctx;
    #[cfg(target_os = "macos")]
    return macos::uninstall_items(ctx, app);
    #[cfg(windows)]
    return windows::uninstall_items(ctx, app);
    #[cfg(target_os = "linux")]
    return linux::uninstall_items(ctx, app);
    #[allow(unreachable_code)]
    Err(format!("{}: uninstall is not available here", app.id))
}

pub async fn tool_list(a: Value) -> Result<Value, String> {
    let query = crate::core::llm::tool_table::s(&a, "query").to_lowercase();
    let source = crate::core::llm::tool_table::s(&a, "source");
    super::tools::blocking(move || {
        let mut apps = list()?;
        apps.retain(|app| {
            (query.is_empty()
                || app.name.to_lowercase().contains(&query)
                || app.id.to_lowercase().contains(&query))
                && (source.is_empty() || app.source == source)
        });
        apps.sort_by_key(|a| a.name.to_lowercase());
        let total = apps.len();
        Ok(json!({ "apps": apps, "total": total }))
    })
    .await
}

pub async fn tool_leftovers(a: Value) -> Result<Value, String> {
    let app_id = crate::core::llm::tool_table::need_str(&a, "app_id")?;
    let source = super::plan::source_of(&a);
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        let apps = list()?;
        let app = apps.into_iter().find(|x| x.id == app_id).unwrap_or_else(|| App {
            id: app_id.clone(),
            name: app_id.split(':').nth(1).unwrap_or(&app_id).to_string(),
            source: app_id.split(':').next().unwrap_or("").to_string(),
            removable: false,
            reason: Some("not installed anymore".into()),
            ..Default::default()
        });
        let (items, extra) = uninstall_items(&ctx, &app)?;
        let total: u64 = items.iter().map(|i| i.bytes).sum();
        let mut preview: Vec<Value> = items
            .iter()
            .map(|i| json!({ "id": i.id, "label": i.label, "detail": i.detail, "group": i.group, "bytes": i.bytes, "risk": i.risk, "admin": i.admin }))
            .collect();
        if let Some(extra_items) = extra.get("unticked").and_then(Value::as_array) {
            for u in extra_items {
                let mut u = u.clone();
                u["unticked"] = json!(true);
                preview.push(u);
            }
        }
        let plan_id = if items.is_empty() {
            Value::Null
        } else {
            json!(plan::offer(&ctx, "uninstall", items, source)?.id)
        };
        Ok(json!({ "app": app, "items": preview, "total_bytes": total, "plan_id": plan_id, "extra": extra }))
    })
    .await
}

pub fn plan_execute(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    match action.split_once('.') {
        Some(("mac", a)) => macos::plan_execute(ctx, receipt_id, a, args),
        Some(("win", a)) => windows::plan_execute(ctx, receipt_id, a, args),
        Some(("linux", a)) => linux::plan_execute(ctx, receipt_id, a, args),
        _ => Err(format!("apps: unknown action {action}")),
    }
}

pub fn plan_elevated(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    match action.split_once('.') {
        Some(("mac", a)) => macos::plan_elevated(ctx, receipt_id, a, args),
        Some(("win", a)) => windows::plan_elevated(ctx, receipt_id, a, args),
        Some(("linux", a)) => linux::plan_elevated(ctx, receipt_id, a, args),
        _ => Err(format!("apps: action {action} has no administrator form")),
    }
}

pub fn plan_undo(ctx: &Ctx, action: &str, args: &Value) -> Result<String, String> {
    match action.split_once('.') {
        Some(("mac", a)) => macos::plan_undo(ctx, a, args),
        Some(("win", a)) => windows::plan_undo(ctx, a, args),
        Some(("linux", a)) => linux::plan_undo(ctx, a, args),
        _ => Err(format!("apps: cannot undo {action}")),
    }
}

pub fn elevated_pkg_forget(id: &str) -> Result<String, String> {
    macos::elevated_pkg_forget(id)
}
