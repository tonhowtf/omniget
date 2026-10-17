use crate::mcp::policy::{self, Principal};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tokio::sync::oneshot;

type Replies = HashMap<String, (String, String, oneshot::Sender<Value>)>;
#[derive(Default)]
struct Registry {
    windows: HashMap<String, String>,
    pending: Replies,
    sessions: HashMap<String, (String, Instant)>,
}
static CONTROL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static OWNER: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

fn local_window(window: &WebviewWindow) -> Result<(), String> {
    if !matches!(
        window.label(),
        "main" | "pet" | "limits-strip" | "usage-panel"
    ) {
        return Err("WINDOW_NOT_FOUND".into());
    }
    let url = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| window.url()))
        .map_err(|_| "FRONTEND_NOT_READY")?
        .map_err(|_| "WINDOW_NOT_FOUND")?;
    let local = url.scheme() == "tauri" && url.host_str() == Some("localhost")
        || url.scheme() == "http" && url.host_str() == Some("tauri.localhost")
        || cfg!(debug_assertions)
            && url.origin().ascii_serialization()
                == std::env::var("OMNIGET_DEV_URL")
                    .unwrap_or_else(|_| "http://localhost:1420".into());
    if !local {
        return Err("PERMISSION_REQUIRED: external WebView".into());
    }
    Ok(())
}

#[tauri::command]
pub fn native_agent_register(window: WebviewWindow, epoch: String) -> Result<(), String> {
    local_window(&window)?;
    uuid::Uuid::parse_str(&epoch).map_err(|_| "INVALID_ARGUMENT: epoch")?;
    let mut state = registry().lock().map_err(|_| "FRONTEND_NOT_READY")?;
    state
        .pending
        .retain(|_, (label, _, _)| label != window.label());
    state.windows.insert(window.label().to_string(), epoch);
    Ok(())
}

#[tauri::command]
pub fn native_agent_reply(
    window: WebviewWindow,
    id: String,
    epoch: String,
    payload: Value,
) -> Result<(), String> {
    local_window(&window)?;
    if payload.to_string().len() > 512 * 1024 {
        return Err("INVALID_ARGUMENT: reply exceeds 512 KiB".into());
    }
    let mut state = registry().lock().map_err(|_| "FRONTEND_NOT_READY")?;
    if state
        .pending
        .get(&id)
        .is_some_and(|(label, expected, _)| label == window.label() && expected == &epoch)
    {
        if let Some((_, _, tx)) = state.pending.remove(&id) {
            let _ = tx.send(payload);
        }
    }
    Ok(())
}

struct Pending(String);
impl Drop for Pending {
    fn drop(&mut self) {
        if let Ok(mut state) = registry().lock() {
            state.pending.remove(&self.0);
        }
    }
}

pub(super) async fn bridge(
    app: &AppHandle,
    window_id: &str,
    operation: &str,
    args: Value,
) -> Result<Value, String> {
    let window = app
        .get_webview_window(window_id)
        .ok_or("WINDOW_NOT_FOUND")?;
    local_window(&window)?;
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    let epoch = {
        let mut state = registry().lock().map_err(|_| "FRONTEND_NOT_READY")?;
        if state.pending.len() >= 8 {
            return Err("SESSION_BUSY".into());
        }
        let epoch = state
            .windows
            .get(window_id)
            .cloned()
            .ok_or("FRONTEND_NOT_READY")?;
        state
            .pending
            .insert(id.clone(), (window_id.into(), epoch.clone(), tx));
        epoch
    };
    let _pending = Pending(id.clone());
    window
        .emit(
            "native-agent-request",
            json!({"id":id,"epoch":epoch,"operation":operation,"arguments":args}),
        )
        .map_err(|_| "FRONTEND_NOT_READY")?;
    tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .map_err(|_| "TIMEOUT: frontend inspection".to_string())?
        .map_err(|_| "FRONTEND_NOT_READY: window reloaded or disconnected".to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InspectArgs {
    #[serde(default = "main_window")]
    window_id: String,
    cursor: Option<usize>,
    limit: Option<usize>,
}
fn main_window() -> String {
    "main".into()
}

pub async fn call(
    app: &AppHandle,
    p: &Principal,
    name: &str,
    mut args: Value,
) -> Result<Value, String> {
    policy::active(p)?;
    if !policy::allowed(p, name) {
        return Err("PERMISSION_REQUIRED".into());
    }
    let _ = app.emit_to(
        "main",
        "native-agent-activity",
        json!({"id":p.id,"name":p.name,"windowId":args["windowId"].as_str().unwrap_or("main")}),
    );
    let mutation = matches!(name, "omniget_app_navigate" | "omniget_app_interact")
        || name == "omniget_app_record" && args["action"] == "start";
    if !mutation {
        return call_inner(app, p, name, args).await;
    }
    let Some(key) = args
        .get("requestId")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        if name == "omniget_app_navigate" {
            return call_inner(app, p, name, args).await;
        }
        return Err("INVALID_ARGUMENT: requestId required".into());
    };
    if let Some(receipt) = policy::reserve(p, name, &key, &args)? {
        return if let Some(error) = receipt["error"].as_str() {
            Err(error.into())
        } else {
            Ok(receipt["result"].clone())
        };
    }
    args.as_object_mut()
        .ok_or("INVALID_ARGUMENT")?
        .remove("requestId");
    let result = call_inner(app, p, name, args).await;
    if result.as_ref().is_err_and(|error| transient(error)) {
        policy::release(p, name, &key)?;
        return result;
    }
    let receipt = match &result {
        Ok(value) => json!({"result":value}),
        Err(error) => json!({"error":error}),
    };
    policy::finish(p, name, &key, &receipt)?;
    result
}

// Only errors raised before the request reached the window are safe to retry
// with the same requestId. A timeout or a reload after dispatch may already
// have acted, so those keep their receipt.
fn transient(error: &str) -> bool {
    if error.starts_with("TIMEOUT") || error.contains("window reloaded or disconnected") {
        return false;
    }
    [
        "SESSION_BUSY",
        "FRONTEND_NOT_READY",
        "STALE_TARGET",
        "WINDOW_NOT_FOUND",
    ]
    .contains(&error.split(':').next().unwrap_or(error))
}

async fn call_inner(
    app: &AppHandle,
    p: &Principal,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    policy::active(p)?;
    if !policy::allowed(p, name) {
        return Err("PERMISSION_REQUIRED".into());
    }
    let session_id = {
        let mut state = registry().lock().map_err(|_| "SESSION_BUSY")?;
        state
            .sessions
            .retain(|_, (_, last)| last.elapsed() < Duration::from_secs(300));
        if state.sessions.len() >= 64 && !state.sessions.contains_key(&p.id) {
            return Err("SESSION_BUSY".into());
        }
        let session = state
            .sessions
            .entry(p.id.clone())
            .or_insert_with(|| (uuid::Uuid::new_v4().to_string(), Instant::now()));
        session.1 = Instant::now();
        session.0.clone()
    };
    let mutation = matches!(name, "omniget_app_navigate" | "omniget_app_interact");
    let _control = if mutation {
        Some(CONTROL.try_lock().map_err(|_| "SESSION_BUSY")?)
    } else {
        None
    };
    if mutation {
        let mut owner = OWNER
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| "SESSION_BUSY")?;
        if owner
            .as_ref()
            .is_some_and(|(id, last)| id != &p.id && last.elapsed() < Duration::from_secs(60))
        {
            return Err("SESSION_BUSY: another connection controls the app".into());
        }
        *owner = Some((p.id.clone(), Instant::now()));
    }
    let mut value = match name {
        "omniget_app_status" => {
            if args.as_object().is_none_or(|o| !o.is_empty()) {
                return Err("INVALID_ARGUMENT: status takes no arguments".into());
            }
            let windows = app.webview_windows();
            let mut state = registry().lock().map_err(|_| "SESSION_BUSY")?;
            state.windows.retain(|label, _| windows.contains_key(label));
            let list:Vec<Value>=windows.keys().map(|label|json!({"windowId":label,"bridgeReady":state.windows.contains_key(label)})).collect();
            json!({"schemaVersion":1,"appVersion":env!("CARGO_PKG_VERSION"),"devIdentity":dev_identity(),"platform":std::env::consts::OS,"windows":list,"capabilities":{"inspect":true,"control":true,"capture":cfg!(target_os="macos"),"record":{"supported":cfg!(target_os="macos"),"targetFps":5,"requires":"FFmpeg","audio":false,"cursor":false}},"warnings":["Only explicitly marked presentation actions are allowed. Synthetic DOM input does not provide trusted OS gestures. Recording availability is reported separately."]})
        }
        "omniget_app_catalog" => {
            let screens: Value =
                serde_json::from_str(include_str!("../../src/lib/native-agent/catalog.json"))
                    .map_err(|_| "FRONTEND_NOT_READY")?;
            json!({"schemaVersion":1,"screens":screens,"systemDialogs":{"supported":false,"reason":"Local user intervention required"}})
        }
        "omniget_app_navigate" | "omniget_app_interact" => {
            let window_id = args["windowId"].as_str().unwrap_or("main");
            let auxiliary = args["screenId"].as_str().and_then(|s| match s {
                "/pet" => Some("pet"),
                "/limits-strip" => Some("limits-strip"),
                "/usage-panel" => Some("usage-panel"),
                _ => None,
            });
            if name.ends_with("navigate") && auxiliary.is_some() {
                let label = auxiliary.unwrap_or("pet");
                match label {
                    "pet" => {
                        crate::commands::pet::pet_open(app.clone()).await?;
                    }
                    "limits-strip" => crate::limits_strip::commands::open_window(
                        app,
                        &crate::limits_strip::commands::load_adopted(),
                    )?,
                    "usage-panel" => crate::usage_tray::show_panel(app, None),
                    _ => return Err("WINDOW_NOT_FOUND".into()),
                }
                for _ in 0..25 {
                    if let Ok(value) = bridge(app, label, "inspect", json!({})).await {
                        policy::active(p)?;
                        return Ok(
                            json!({"schemaVersion":1,"sessionId":session_id,"observedAt":chrono::Utc::now().to_rfc3339(),"windowId":label,"state":value}),
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                return Err("FRONTEND_NOT_READY".into());
            }
            let operation = if name.ends_with("navigate") {
                "navigate"
            } else {
                "interact"
            };
            if operation == "navigate" {
                let window = app
                    .get_webview_window(window_id)
                    .ok_or("WINDOW_NOT_FOUND")?;
                local_window(&window)?;
                window
                    .unminimize()
                    .map_err(|_| "FRONTEND_NOT_READY: restore window")?;
                window
                    .show()
                    .map_err(|_| "FRONTEND_NOT_READY: show window")?;
                #[cfg(target_os = "macos")]
                app.run_on_main_thread(|| {
                    if let Some(mtm) = objc2::MainThreadMarker::new() {
                        #[allow(deprecated)]
                        objc2_app_kit::NSApplication::sharedApplication(mtm)
                            .activateIgnoringOtherApps(true);
                    }
                })
                .map_err(|_| "FRONTEND_NOT_READY: activate application")?;
                window
                    .set_focus()
                    .map_err(|_| "FRONTEND_NOT_READY: focus window")?;
            }
            let result = bridge(app, window_id, operation, args.clone()).await?;
            if let Some(code) = result["error"]["code"].as_str() {
                return Err(code.to_string());
            }
            if operation == "navigate" && result["route"] != args["screenId"] {
                return Err(format!(
                    "TARGET_NOT_ACTIONABLE: surface redirected to {}",
                    result["route"]
                ));
            }
            if operation == "navigate" {
                let count = result["nodes"].as_array().map(Vec::len).unwrap_or(0);
                json!({
                    "route": result["route"],
                    "readiness": result["readiness"],
                    "stateRevision": result["stateRevision"],
                    "sensitive": result["sensitive"],
                    "sensitiveReason": result["sensitiveReason"],
                    "semantics": result["semantics"],
                    "elements": count,
                    "more": "omniget_app_inspect lists the elements; omniget_app_capture takes a picture",
                })
            } else {
                result
            }
        }
        "omniget_app_export" => crate::native_agent_export::export(p, args)?,
        "omniget_app_record" => crate::native_agent_record::call(app, p, args).await?,
        "omniget_app_capture" => crate::native_agent_media::capture(app, p, args).await?,
        "omniget_app_inspect" => {
            let parsed: InspectArgs =
                serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT")?;
            if parsed.cursor.is_some_and(|x| x > 10000)
                || parsed.limit.is_some_and(|x| x == 0 || x > 200)
            {
                return Err("INVALID_ARGUMENT: pagination bounds".into());
            }
            let mut result = bridge(
                app,
                &parsed.window_id,
                "inspect",
                json!({"cursor":parsed.cursor,"limit":parsed.limit}),
            )
            .await?;
            if result.get("error").is_some() {
                return Err("FRONTEND_NOT_READY: inspection failed".into());
            }
            result["windowId"] = json!(parsed.window_id);
            result
        }
        _ => return Err("CAPABILITY_UNAVAILABLE".into()),
    };
    policy::active(p)?;
    value["sessionId"] = json!(session_id);
    value["observedAt"] = json!(chrono::Utc::now().to_rfc3339());
    Ok(value)
}

pub fn output_schema(name: &str) -> Option<Value> {
    name.starts_with("omniget_app_").then(||json!({"type":"object","properties":{"schemaVersion":{"type":"integer"},"sessionId":{"type":"string"},"observedAt":{"type":"string"}},"required":["schemaVersion","sessionId","observedAt"]}))
}

fn dev_identity() -> Value {
    if !cfg!(debug_assertions) || std::env::var_os("OMNIGET_DEV_SESSION").is_none() {
        return Value::Null;
    }
    json!({"sessionId":std::env::var("OMNIGET_DEV_SESSION").ok(),
        "backendBuildId":std::env::var("OMNIGET_BACKEND_BUILD_ID").ok(),
        "workerBuildId":std::env::var("OMNIGET_WORKER_BUILD_ID").ok(),
        "bootId":std::env::var("OMNIGET_BOOT_ID").ok(),"pid":std::process::id()})
}

#[cfg(test)]
mod tests {
    #[test]
    fn transient_errors_are_not_kept_as_receipts() {
        assert!(super::transient("SESSION_BUSY"));
        assert!(super::transient("FRONTEND_NOT_READY: show window"));
        assert!(super::transient("WINDOW_NOT_FOUND"));
        assert!(!super::transient("TIMEOUT: frontend inspection"));
        assert!(!super::transient(
            "FRONTEND_NOT_READY: window reloaded or disconnected"
        ));
        assert!(!super::transient("PERMISSION_REQUIRED"));
        assert!(!super::transient("INVALID_ARGUMENT"));
    }
}
