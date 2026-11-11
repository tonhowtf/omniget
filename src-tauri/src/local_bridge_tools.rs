use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::commands::devtools::mcp as devtools;
use crate::commands::research::mcp as research;
use crate::commands::transfer::mcp as transfer;
use crate::local_bridge_telegram::authorized;

#[derive(Debug, PartialEq)]
pub enum Family {
    Dev,
    Browser,
    Transfer,
    Research,
}

pub fn family(tool: &str) -> Option<Family> {
    if devtools::TOOLS.contains(&tool) {
        Some(Family::Dev)
    } else if transfer::TOOLS.contains(&tool) {
        Some(Family::Transfer)
    } else if research::TOOLS.contains(&tool) {
        Some(Family::Research)
    } else if tool.starts_with("gd_") {
        Some(Family::Browser)
    } else {
        None
    }
}

fn listing() -> Value {
    let mut tools: Vec<Value> = devtools::defs()
        .into_iter()
        .map(|(name, description, _, _)| json!({ "name": name, "description": description }))
        .collect();
    tools.extend(
        transfer::defs()
            .into_iter()
            .map(|(name, description, _, _)| json!({ "name": name, "description": description })),
    );
    tools.extend(
        research::defs()
            .into_iter()
            .map(|(name, description, _, _)| json!({ "name": name, "description": description })),
    );
    tools.extend(
        omniget_core::core::llm::tool_table::specs()
            .into_iter()
            .filter(|spec| spec.name.starts_with("gd_"))
            .map(|spec| json!({ "name": spec.name, "description": spec.description })),
    );
    json!({ "tools": tools })
}

async fn run(app: &AppHandle, tool: &str, input: Value) -> Result<Value, String> {
    if tool == "list" {
        return Ok(listing());
    }
    if !input.is_object() {
        return Err("INVALID_ARGUMENTS: the input must be a JSON object".into());
    }
    match family(tool) {
        Some(Family::Dev) => devtools::call(tool, &input, true).await,
        Some(Family::Transfer) => transfer::call(tool, &input, true).await,
        Some(Family::Research) => research::call(tool, &input).await,
        Some(Family::Browser) => crate::mcp::call(app, tool, input).await,
        None => Err(format!(
            "UNKNOWN_TOOL: {tool} is not a dev_*, gd_*, transfer_* or research_* tool; run `omniget tool` for the list"
        )),
    }
}

pub fn router<S>(
    app: AppHandle,
    token: std::sync::Arc<String>,
    guard: crate::bridge_tokens::Guard,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route(
        "/v1/tool/{tool}",
        post(
            move |Path(tool): Path<String>, headers: HeaderMap, body: Option<Json<Value>>| async move {
                if !authorized(&headers, &token, &guard) {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({ "error": "bad or missing bearer token" })),
                    )
                        .into_response();
                }
                let input = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
                let response: Response = match run(&app, &tool, input).await {
                    Ok(v) => Json(v).into_response(),
                    Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
                };
                response
            },
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dev_and_browser_tools_are_reachable() {
        assert_eq!(family("dev_ports"), Some(Family::Dev));
        assert_eq!(family("gd_snapshot"), Some(Family::Browser));
        assert_eq!(family("transfer_list"), Some(Family::Transfer));
        assert_eq!(family("research_call"), Some(Family::Research));
        assert_eq!(family("wa_send"), None);
        assert_eq!(family("dev_unknown"), None);
    }

    #[test]
    fn the_listing_names_every_dev_tool_and_the_browser_tools() {
        let listed = listing();
        let names: Vec<&str> = listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for tool in devtools::TOOLS {
            assert!(names.contains(tool), "{tool}");
        }
        for tool in transfer::TOOLS {
            assert!(names.contains(tool), "{tool}");
        }
        for tool in research::TOOLS {
            assert!(names.contains(tool), "{tool}");
        }
        assert!(names.contains(&"gd_act"));
        assert!(names.contains(&"gd_market"));
    }
}
