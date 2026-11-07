use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::local_bridge_auth::check_bearer;
use omniget_core::core::telegram::{commands, net};

const ALLOWED: &[&str] = &[
    "telegram_check_session",
    "telegram_get_self",
    "telegram_list_chats",
    "telegram_list_chats_page",
    "telegram_full_channel_info",
    "telegram_list_media",
    "telegram_search_media",
    "telegram_search_global_hits",
    "telegram_resolve_links",
    "telegram_download_links",
    "telegram_download_chat",
    "telegram_preview_chat_download",
    "telegram_download_media",
    "telegram_download_batch",
    "telegram_cancel_batch",
    "telegram_export_chat",
    "telegram_watch_list",
    "telegram_watch_add",
    "telegram_watch_update",
    "telegram_watch_remove",
    "telegram_watch_run",
    "telegram_watch_settings",
    "telegram_settings_get",
    "telegram_bandwidth_stats",
    "telegram_sync_state",
    "telegram_stream_url",
];

fn fail(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn authorized(headers: &HeaderMap, token: &str, guard: &crate::bridge_tokens::Guard) -> bool {
    if check_bearer(headers, token) {
        return true;
    }
    let Some(bearer) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
    else {
        return false;
    };
    matches!(
        (guard.lookup)(bearer.trim()),
        crate::bridge_tokens::Lookup::Valid(v) if v.kind == crate::bridge_tokens::Kind::Cli
    )
}

pub fn router<S>(token: std::sync::Arc<String>, guard: crate::bridge_tokens::Guard) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route(
        "/v1/telegram/{command}",
        post(
            move |Path(command): Path<String>, headers: HeaderMap, body: Option<Json<Value>>| async move {
                if !authorized(&headers, &token, &guard) {
                    return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
                }
                let args = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
                if command == "telegram_net_status" {
                    return Json(serde_json::to_value(net::manager::get().status()).unwrap_or_default())
                        .into_response();
                }
                if !ALLOWED.contains(&command.as_str()) {
                    return fail(StatusCode::FORBIDDEN, format!("{command} is not available to the CLI"));
                }
                match commands::dispatch(command, args).await {
                    Ok(v) => Json(v).into_response(),
                    Err(e) => fail(StatusCode::BAD_REQUEST, e),
                }
            },
        ),
    )
}
