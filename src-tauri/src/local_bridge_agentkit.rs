//! Bridge routes of the Catalog/agentkit, for `omniget agentkit …` (omniget-cli).
//! Same bearer as the rest of the bridge. One route, `POST /v1/agentkit/{action}`
//! with a JSON body (the bridge's axum has no query extractor); every action
//! runs on a blocking thread because catalog downloads use `block_on`.
//!
//! Actions: `search`, `show`, `compat`, `targets`, `plan`, `apply`, `uninstall`,
//! `installed`, `doctor`, `import_installed`, `export`, `profiles`,
//! `profile_plan`, `loop_plan`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

use omniget_core::core::agentkit::{self as ak, model::Component, plan::PlanRequest, Scope};
use omniget_core::core::catalog::{self, collections, index, search};

use crate::commands::central::agentkit::{
    compat_report, env, parse_policy, project, resolve_any, to_value,
};
use crate::local_bridge_llm::check_bearer;

fn fail(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

pub fn router<S>(token: Arc<String>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route(
        "/v1/agentkit/{action}",
        post(
            move |Path(action): Path<String>, headers: HeaderMap, body: axum::body::Bytes| {
                let token = token.clone();
                async move {
                    if !check_bearer(&headers, &token) {
                        return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
                    }
                    let body: Value = if body.is_empty() {
                        Value::Null
                    } else {
                        match serde_json::from_slice(&body) {
                            Ok(v) => v,
                            Err(e) => {
                                return fail(StatusCode::BAD_REQUEST, format!("AGENTKIT_JSON: {e}"))
                            }
                        }
                    };
                    let res = tokio::task::spawn_blocking(move || run(&action, &body)).await;
                    match res {
                        Ok(Ok(v)) => Json(v).into_response(),
                        Ok(Err(e)) if e.starts_with("AGENTKIT_ACTION") => {
                            fail(StatusCode::NOT_FOUND, e)
                        }
                        Ok(Err(e)) => fail(StatusCode::BAD_REQUEST, e),
                        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                    }
                }
            },
        ),
    )
}

fn s(b: &Value, k: &str) -> Option<String> {
    b.get(k)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|v| !v.trim().is_empty())
}

fn list(b: &Value, k: &str) -> Vec<String> {
    match b.get(k) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .filter(|v| !v.is_empty())
            .collect(),
        Some(Value::String(v)) => v
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        _ => vec![],
    }
}

fn scope_of(b: &Value) -> Result<Option<Scope>, String> {
    Ok(s(b, "scope").as_deref().map(Scope::parse).transpose()?)
}

/// Installed tools the UI would tick by default.
fn default_targets(env: &ak::Env, project_dir: Option<&std::path::Path>) -> Vec<String> {
    ak::detect::detect_all(env, project_dir)
        .into_iter()
        .filter(|d| d.enabled_by_default)
        .map(|d| d.id)
        .collect()
}

pub(crate) fn run(action: &str, b: &Value) -> Result<Value, String> {
    let env = env()?;
    let proj: Option<PathBuf> = project(s(b, "project"));
    match action {
        "search" => {
            let c = index::catalog().map_err(String::from)?;
            let filters = search::Filters {
                kinds: list(b, "kinds"),
                categories: list(b, "categories"),
                sources: list(b, "sources"),
                ..Default::default()
            };
            let sort: search::Sort = b
                .get("sort")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            let page = search::Page {
                offset: b.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                limit: b
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20)
                    .min(search::MAX_LIMIT as u64) as usize,
            };
            to_value(search::search(
                &c.items,
                s(b, "query").as_deref().unwrap_or(""),
                &filters,
                sort,
                page,
            ))
        }
        "show" => {
            let id = s(b, "id").ok_or("AGENTKIT_ARGS: id is required")?;
            let item = index::item(&id).ok();
            let compat = compat_report(&id, proj.as_deref())?;
            Ok(json!({ "item": item, "compat": compat }))
        }
        "compat" => {
            let id = s(b, "id").ok_or("AGENTKIT_ARGS: id is required")?;
            compat_report(&id, proj.as_deref())
        }
        "targets" => to_value(ak::detect::detect_all(&env, proj.as_deref())),
        "plan" => {
            let ids = list(b, "ids");
            // canonical components (e.g. from `import_installed`) go in as they are
            let mut components: Vec<Component> = match b.get("components") {
                Some(Value::Array(a)) => a
                    .iter()
                    .map(|v| {
                        serde_json::from_value(v.clone()).map_err(|e| format!("AGENTKIT_JSON: {e}"))
                    })
                    .collect::<Result<_, _>>()?,
                _ => vec![],
            };
            for id in &ids {
                components.push(resolve_any(id)?);
            }
            if components.is_empty() {
                return Err("AGENTKIT_ARGS: nothing to install".into());
            }
            let mut targets = list(b, "targets");
            if targets.is_empty() {
                targets = default_targets(&env, proj.as_deref());
            }
            if targets.is_empty() {
                return Err("AGENTKIT_ARGS: no tool detected; pass --target".into());
            }
            let secret_values: BTreeMap<String, String> = b
                .get("secret_values")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            let plan = ak::plan::plan(
                &env,
                PlanRequest {
                    components,
                    targets,
                    scope: scope_of(b)?,
                    project_dir: proj,
                    policy: parse_policy(s(b, "policy").as_deref()),
                    secret_values,
                },
            )?;
            to_value(plan)
        }
        "apply" => {
            let id = s(b, "plan_id").ok_or("AGENTKIT_ARGS: plan_id is required")?;
            let plan = ak::plan::get(&id)
                .ok_or_else(|| format!("AGENTKIT_PLAN: plan `{id}` not found (plan again)"))?;
            let report = ak::writer::apply(&env, &plan)?;
            ak::plan::take(&id);
            to_value(report)
        }
        "uninstall" => {
            let id = s(b, "id").ok_or("AGENTKIT_ARGS: id is required")?;
            let target = s(b, "target");
            let force = b.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
            let recs = ak::writer::installed(&env, proj.as_deref());
            let ids: Vec<String> = if recs.iter().any(|r| r.install_id == id) {
                vec![id.clone()]
            } else {
                recs.iter()
                    .filter(|r| r.component.id == id || r.component.name == id)
                    .filter(|r| target.as_deref().map(|t| t == r.target).unwrap_or(true))
                    .map(|r| r.install_id.clone())
                    .collect()
            };
            if ids.is_empty() {
                return Err(format!("AGENTKIT_NOT_INSTALLED: `{id}` is not installed"));
            }
            let mut reports = Vec::new();
            for i in ids {
                reports.push(to_value(ak::writer::uninstall(
                    &env,
                    &i,
                    proj.as_deref(),
                    force,
                )?)?);
            }
            Ok(Value::Array(reports))
        }
        "installed" => to_value(ak::writer::installed(&env, proj.as_deref())),
        "doctor" => {
            let targets = ak::detect::detect_all(&env, proj.as_deref());
            let drift = ak::writer::drift(&env, proj.as_deref())?;
            let installed = ak::writer::installed(&env, proj.as_deref());
            let catalog = index::meta().ok();
            Ok(json!({
                "targets": targets,
                "installed": installed.len(),
                "drift": drift,
                "catalog": catalog,
            }))
        }
        "import_installed" => {
            let from = s(b, "from").ok_or("AGENTKIT_ARGS: from is required")?;
            let scope = match scope_of(b)? {
                Some(sc) => sc,
                None if proj.is_some() => Scope::Project,
                None => Scope::Global,
            };
            to_value(ak::convert::import_installed(
                &env,
                &from,
                scope,
                scope_project(scope, &proj),
            )?)
        }
        "export" => {
            let recs = ak::writer::installed(&env, proj.as_deref());
            let mut items: Vec<collections::StackItem> = Vec::new();
            for r in &recs {
                if r.component.id.starts_with("installed:") || r.component.id.starts_with("path:") {
                    continue;
                }
                match items.iter_mut().find(|i| i.id == r.component.id) {
                    Some(i) => {
                        if !i.targets.contains(&r.target) {
                            i.targets.push(r.target.clone());
                        }
                    }
                    None => items.push(collections::StackItem {
                        id: r.component.id.clone(),
                        content_hash: index::item(&r.component.id)
                            .ok()
                            .and_then(|it| catalog::model::content_hash(&it)),
                        targets: vec![r.target.clone()],
                        scope: Some(r.scope.as_str().to_string()),
                    }),
                }
            }
            let name = s(b, "name").unwrap_or_else(|| "installed".into());
            let mut targets: Vec<String> = Vec::new();
            for i in &items {
                for t in &i.targets {
                    if !targets.contains(t) {
                        targets.push(t.clone());
                    }
                }
            }
            let scope = items.first().and_then(|i| i.scope.clone());
            let hash = collections::stack_hash(&name, &targets, scope.as_deref(), &items);
            to_value(collections::StackFile {
                format: collections::STACK_FORMAT.into(),
                version: collections::STACK_VERSION,
                name,
                description: Some("Exported by omniget agentkit export".into()),
                targets,
                scope,
                created: chrono::Utc::now().to_rfc3339(),
                generator: Some(format!("omniget {}", env!("CARGO_PKG_VERSION"))),
                items,
                hash,
                signature: None,
            })
        }
        "profiles" => to_value(ak::profiles::list()),
        "profile_plan" => {
            let id = s(b, "id").ok_or("AGENTKIT_ARGS: id is required")?;
            let targets = list(b, "targets");
            let plan = ak::profiles::plan_profile(
                &env,
                ak::profiles::ProfileRequest {
                    id,
                    params: b.get("params").cloned().unwrap_or(Value::Null),
                    targets: (!targets.is_empty()).then_some(targets),
                    scope: scope_of(b)?,
                    project_dir: proj,
                    policy: parse_policy(s(b, "policy").as_deref()),
                },
            )?;
            to_value(plan)
        }
        "loop_plan" => {
            let id = s(b, "id").ok_or("AGENTKIT_ARGS: id is required")?;
            let runner = s(b, "runner").unwrap_or_else(|| "claude".into());
            let c = resolve_any(&id)?;
            to_value(ak::convert::loop_runbook::loop_plan(
                &c,
                &runner,
                s(b, "workspace").as_deref(),
            )?)
        }
        other => Err(format!("AGENTKIT_ACTION: unknown action `{other}`")),
    }
}

fn scope_project(scope: Scope, p: &Option<PathBuf>) -> Option<&std::path::Path> {
    if scope.is_project_bound() {
        p.as_deref()
    } else {
        None
    }
}
