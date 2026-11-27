use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

use crate::local_bridge::check_bearer;

type PendingReplies = HashMap<String, (String, oneshot::Sender<Value>)>;
static PENDING: OnceLock<Mutex<PendingReplies>> = OnceLock::new();

fn pending() -> &'static Mutex<PendingReplies> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn enabled() -> bool {
    cfg!(debug_assertions) && std::env::var("OMNIGET_TEST_DRIVER").ok().as_deref() == Some("1")
}

#[tauri::command]
pub fn debug_report(window: tauri::WebviewWindow, id: String, value: Value) {
    if !enabled() {
        return;
    }
    let mut pending = pending().lock().unwrap();
    if pending
        .get(&id)
        .is_some_and(|(label, _)| label == window.label())
    {
        if let Some((_, tx)) = pending.remove(&id) {
            let _ = tx.send(value);
        }
    }
}

pub fn router<S>(app: AppHandle, token: std::sync::Arc<String>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    if !enabled() {
        return Router::new();
    }
    Router::new().route(
        "/v1/debug/eval",
        post(move |headers: HeaderMap, body: String| async move {
            let head = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
            if !check_bearer(&headers, &token) {
                return (StatusCode::UNAUTHORIZED, "bad bearer").into_response();
            }
            let label = head("x-window").unwrap_or_else(|| "main".into());
            let Some(window) = app.get_webview_window(&label) else {
                return (StatusCode::NOT_FOUND, "no such window").into_response();
            };
            let id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = oneshot::channel();
            pending().lock().unwrap().insert(id.clone(), (label, tx));
            let script = format!(
                "(async()=>{{let r;try{{r=await (async()=>{{{body}\n}})();}}catch(e){{r={{error:String(e&&e.stack||e)}};}}\
                 if(r===undefined)r=null;window.__TAURI_INTERNALS__.invoke('debug_report',{{id:'{id}',value:r}});}})()"
            );
            if let Err(e) = window.eval(&script) {
                pending().lock().unwrap().remove(&id);
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
            let secs = head("x-timeout").and_then(|t| t.parse().ok()).unwrap_or(20u64).clamp(1, 60);
            let out: Response = match tokio::time::timeout(Duration::from_secs(secs), rx).await {
                Ok(Ok(v)) => Json(v).into_response(),
                _ => {
                    pending().lock().unwrap().remove(&id);
                    Json(json!({ "error": "timeout" })).into_response()
                }
            };
            out
        }),
    )
}
