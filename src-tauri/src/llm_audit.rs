use std::collections::HashMap;
use std::sync::Mutex;

use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use omniget_core::core::llm::audit::{self, Actor, Entity, Query};
use serde_json::{json, Value};

use crate::mcp::policy::Principal;

pub fn boot(app: &tauri::AppHandle) {
    let Some(dir) = omniget_core::core::llm::roster_store::llm_dir() else {
        return;
    };
    let days = crate::storage::config::load_settings(app)
        .llm
        .audit_retention_days;
    if let Err(e) = audit::init(&dir.join("audit.db"), days) {
        tracing::warn!("[audit] cannot open the audit log: {e}");
    }
}

pub fn actor_of(principal: Option<&Principal>, channel: &str) -> Actor {
    Actor::via(
        channel,
        principal.map(|p| p.id.as_str()).unwrap_or("unknown"),
    )
}

pub fn job_created(actor: Actor, job: &crate::jobs::Job) {
    audit::log(
        actor,
        "job.created",
        Entity::job(&job.id),
        json!({
            "kind": job.kind,
            "agent_id": job.agent_id,
            "run_mode": job.run_mode.as_str(),
            "isolation": job.isolation.as_str(),
            "workspace": job.workspace,
            "auto_resume": job.auto_resume,
        }),
    );
}

pub fn job_cancelled(actor: Actor, job: &crate::jobs::Job) {
    audit::log(
        actor,
        "job.cancelled",
        Entity::job(&job.id),
        json!({ "kind": job.kind, "agent_id": job.agent_id, "state_before": job.state }),
    );
}

pub fn job_reviewed(actor: Actor, action: &str, job_id: &str, details: Value) {
    audit::log(actor, action, Entity::job(job_id), details);
}

pub fn arena_action(action: &str, arena_id: &str, details: Value) {
    audit::log(Actor::window(), action, Entity::arena(arena_id), details);
}

pub fn job_ended(job: &crate::jobs::Job) {
    audit::log(
        Actor::system("system"),
        "job.ended",
        Entity::job(&job.id),
        json!({
            "kind": job.kind,
            "agent_id": job.agent_id,
            "state": job.state,
            "failure_reason": job.failure_reason,
            "files": job.diff_files.as_ref().map(Vec::len),
            "cost_usd": job.usage.as_ref().and_then(|u| u["cost_usd"].as_f64()),
        }),
    );
}

pub fn job_auto_resumed(job: &crate::jobs::Job, reason: &str) {
    audit::log(
        Actor::system("system"),
        "job.auto_resumed",
        Entity::job(&job.id),
        json!({
            "kind": job.kind,
            "agent_id": job.agent_id,
            "reason": reason,
            "attempt": job.attempt,
            "native_resume": job.session_id.is_some(),
            "isolation": job.isolation.as_str(),
            "side_effects": job.side_effects,
        }),
    );
}

pub fn loop_auto_resumed(loop_id: &str, reason: &str, restarts: u32, round: Option<&str>) {
    audit::log(
        Actor::system("system"),
        "loop.auto_resumed",
        Entity::loop_(loop_id),
        json!({ "reason": reason, "restarts": restarts, "round_job": round }),
    );
}

pub fn loop_audited(loop_id: &str, a: &crate::jobs::LoopAudit) {
    audit::log(
        Actor::system("auditor"),
        "loop.audited",
        Entity::loop_(loop_id),
        json!({
            "round": a.round, "verdict": a.verdict, "model": a.model, "reason": a.reason,
            "cost_usd": a.cost_usd, "tokens": a.tokens, "error": a.error.is_some(),
        }),
    );
}

pub fn app_started_hidden(how: &str) {
    audit::log(
        Actor::system("system"),
        "app.started_hidden",
        Entity::app(&std::process::id().to_string()),
        json!({ "how": how }),
    );
}

fn self_audited(method: &Method, path: &str) -> bool {
    if method != Method::POST {
        return false;
    }
    let segs: Vec<&str> = path.trim_end_matches('/').split('/').skip(1).collect();
    matches!(
        segs.as_slice(),
        ["v1", "agent", "run" | "loop"]
            | ["v1", "commands"]
            | ["v1", "jobs", _, "cancel" | "integrate" | "discard"]
            | ["v1", "jobs", _, "hunks", "discard"]
            | ["v1", "approvals", _]
            | ["v1", "agents", "cli"]
            | ["v1", "budget", "policies"]
            | ["v1", "budget", "incidents", _]
    )
}

const READ_FOLD_MS: u64 = 60_000;

pub fn on_route(principal: &Principal, method: &Method, path: &str, now_ms: u64) {
    if self_audited(method, path) {
        return;
    }
    if method == Method::GET {
        static SEEN: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
        let key = format!("{} {path}", principal.id);
        let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
        let map = seen.get_or_insert_with(HashMap::new);
        if map
            .get(&key)
            .is_some_and(|at| now_ms.saturating_sub(*at) < READ_FOLD_MS)
        {
            return;
        }
        if map.len() > 4096 {
            map.clear();
        }
        map.insert(key, now_ms);
    }
    audit::log(
        actor_of(Some(principal), "http"),
        "token.use",
        Entity::route(&format!("{method} {path}")),
        json!({ "scopes": principal.scopes }),
    );
}

pub fn query_of(raw: Option<&str>, now_ms: u64) -> Result<Query, String> {
    let mut q = Query::default();
    for pair in raw.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = v.replace('+', " ");
        match k {
            "since" => {
                let span = audit::parse_since(&v)
                    .ok_or_else(|| format!("since must look like 1d, 12h or 30m (got `{v}`)"))?;
                q.since_ms = Some(now_ms.saturating_sub(span) as i64);
            }
            "action" => q.action = Some(v),
            "entity" => q.entity_id = Some(v),
            "limit" => q.limit = v.parse().ok(),
            _ => {}
        }
    }
    Ok(q)
}

pub fn http_list(query: Option<&str>) -> Response {
    match query_of(query, crate::llm_manager::now_ms()) {
        Ok(q) => Json(json!(audit::list(&q))).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

#[tauri::command]
pub async fn llm_audit_list(
    since: Option<String>,
    action: Option<String>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let span = audit::parse_since(since.as_deref().unwrap_or("1d"))
        .ok_or_else(|| "ERR_AUDIT: since must look like 1d, 12h or 30m".to_string())?;
    let q = Query {
        since_ms: Some(crate::llm_manager::now_ms().saturating_sub(span) as i64),
        action: action.filter(|a| !a.trim().is_empty()),
        entity_id: None,
        limit: Some(limit.unwrap_or(500)),
    };
    Ok(json!({
        "rows": audit::list(&q),
        "retention_days": audit::global().map(|a| a.retention_days()),
    }))
}

#[tauri::command]
pub async fn llm_audit_retention_set(app: tauri::AppHandle, days: u32) -> Result<Value, String> {
    let days = days.clamp(1, 3650);
    crate::storage::config::update_settings(&app, |s| s.llm.audit_retention_days = days)
        .map_err(|e| e.to_string())?;
    if let Some(a) = audit::global() {
        a.set_retention_days(days);
    }
    audit::log(
        Actor::window(),
        "audit.retention",
        Entity::setting("llm.audit_retention_days"),
        json!({ "days": days }),
    );
    Ok(json!({ "retention_days": days }))
}
