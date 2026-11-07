pub mod approvals;
pub mod instagram;
pub mod model;
pub mod page;
pub mod tiktok;
pub mod tools;
pub mod x;

use serde_json::{json, Value};
use tauri::AppHandle;

use model::Platform;

pub mod args {
    use serde_json::Value;

    pub fn opt_str(a: &Value, key: &str) -> Option<String> {
        match &a[key] {
            Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    pub fn req_str(a: &Value, key: &str) -> Result<String, String> {
        opt_str(a, key).ok_or_else(|| format!("INVALID_ARGUMENTS: {key} is required"))
    }

    pub fn undo(a: &Value) -> bool {
        a["undo"].as_bool().unwrap_or(false)
    }
}

#[derive(Clone, Debug)]
pub enum Caller {
    Screen,
    Agent(String),
}

pub async fn status(p: Platform) -> Value {
    let Some(b) = page::running().await else {
        return json!({ "platform": p.id(), "browser": false, "logged_in": Value::Null });
    };
    json!({ "platform": p.id(), "browser": true, "logged_in": page::logged_in(&b, p).await })
}

pub async fn status_all(start: bool) -> Value {
    if start {
        let _ = page::ensure_browser().await;
    }
    let mut out = Vec::new();
    for p in [Platform::X, Platform::Instagram, Platform::Tiktok] {
        out.push(status(p).await);
    }
    json!({ "platforms": out })
}

pub async fn call(caller: Caller, tool: &str, a: Value) -> Result<Value, String> {
    if !a.is_object() {
        return Err("INVALID_ARGUMENTS: arguments must be an object".into());
    }
    if tool == "social_status" {
        return Ok(status_all(matches!(caller, Caller::Agent(_))).await);
    }
    let spec = tools::find(tool).ok_or_else(|| format!("UNKNOWN_TOOL: {tool}"))?;
    if spec.write {
        if let Caller::Agent(client) = &caller {
            approvals::pace(spec.platform)?;
            let (summary, details) = tools::describe(spec, &a);
            approvals::request(tool, spec.platform, summary, details, client).await?;
        }
    }
    let r = match spec.platform {
        Platform::X => x::call(tool, &a).await,
        Platform::Instagram => instagram::call(tool, &a).await,
        Platform::Tiktok => tiktok::call(tool, &a).await,
    };
    if spec.write && r.is_ok() {
        approvals::record(spec.platform);
    }
    r
}

fn platform(p: &str) -> Result<Platform, String> {
    Platform::parse(p).ok_or_else(|| format!("unknown platform {p}"))
}

#[tauri::command]
pub async fn social_status(app: AppHandle) -> Value {
    approvals::set_app(&app);
    status_all(false).await
}

#[tauri::command]
pub async fn social_login(app: AppHandle, platform: String) -> Result<Value, String> {
    approvals::set_app(&app);
    let p = self::platform(&platform)?;
    crate::commands::global_dl::gd_browser_show(Some(p.login_url().to_string()), None).await
}

#[tauri::command]
pub async fn social_call(
    app: AppHandle,
    tool: String,
    args: Option<Value>,
) -> Result<Value, String> {
    approvals::set_app(&app);
    call(Caller::Screen, &tool, args.unwrap_or_else(|| json!({}))).await
}

#[tauri::command]
pub fn social_approval_list(app: AppHandle) -> Vec<approvals::Request> {
    approvals::set_app(&app);
    approvals::list()
}

#[tauri::command]
pub fn social_approval_answer(id: String, approve: bool) -> Result<(), String> {
    approvals::answer(&id, approve)
}
