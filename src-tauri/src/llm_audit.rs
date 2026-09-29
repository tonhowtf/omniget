//! The app's side of the `/llm` audit log (2.8, S14): opening it at boot,
//! who a bridge caller is, the lines the bridge and the window write, and the
//! three doors to read it (`GET /v1/audit`, the window, `omniget audit`).
//! The log itself (file, redaction, retention) is
//! `omniget_core::core::llm::audit`.
//!
//! Token use on the agent routes: a route that writes its own line (start a
//! job, cancel it, answer an ask, create an agent, start a Loop) carries the
//! token as the line's actor; every other agent route gets a `token.use`
//! line from the guard, reads folded to one per token and route per minute
//! (the CLI polls).

use std::collections::HashMap;
use std::sync::Mutex;

use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use omniget_core::core::llm::audit::{self, Actor, Entity, Query};
use serde_json::{json, Value};

use crate::mcp::policy::Principal;

/// Opens `<llm dir>/audit.db` with the retention the settings ask for.
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

/// The caller of a bridge route as an audit actor (`token:<id>`,
/// `job:<id>`, `extension`), with the channel it came through.
pub fn actor_of(principal: Option<&Principal>, channel: &str) -> Actor {
    Actor::via(
        channel,
        principal.map(|p| p.id.as_str()).unwrap_or("unknown"),
    )
}

/// A job someone started: who, and the facts of it (never the prompt).
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

/// A review action on a job's changes (3.5): `job.hunk_discarded`,
/// `job.integrated`, `job.discarded`. Only the facts (hunk id, strategy,
/// outcome), never the patch.
pub fn job_reviewed(actor: Actor, action: &str, job_id: &str, details: Value) {
    audit::log(actor, action, Entity::job(job_id), details);
}

/// A choice in an Arena (3.8): `arena.applied` (this entry merged) and
/// `arena.cleaned` (the others removed). Actions only, no content.
pub fn arena_action(action: &str, arena_id: &str, details: Value) {
    audit::log(Actor::window(), action, Entity::arena(arena_id), details);
}

/// How a job ended, written by the app (a cancel has its own line, with the
/// person who asked for it).
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

/// Routes whose handler writes its own line with the caller as actor.
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
    )
}

/// Reads of one token on one route are written once a minute at most.
const READ_FOLD_MS: u64 = 60_000;

/// Called by the bridge guard once the caller is known.
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

// ── reading ─────────────────────────────────────────────────────────────

/// `since` (`1d`, `12h`, `30m`), `action` (a prefix, `job.`), `entity`,
/// `limit` of a query string.
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

/// `GET /v1/audit?since=1d&action=job.&limit=200` (scope `audit:read`).
pub fn http_list(query: Option<&str>) -> Response {
    match query_of(query, crate::llm_manager::now_ms()) {
        Ok(q) => Json(json!(audit::list(&q))).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

// ── the window ──────────────────────────────────────────────────────────

/// The lines of the last `since` (default a day), newest first, and the
/// retention in days.
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

/// Sets how many days a line is kept (1–3650); older lines go now.
#[tauri::command]
pub async fn llm_audit_retention_set(app: tauri::AppHandle, days: u32) -> Result<Value, String> {
    let days = days.clamp(1, 3650);
    let mut settings = crate::storage::config::load_settings(&app);
    settings.llm.audit_retention_days = days;
    crate::storage::config::save_settings(&app, &settings).map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_routes_that_write_their_own_line_are_not_counted_twice() {
        assert!(self_audited(&Method::POST, "/v1/agent/run"));
        assert!(self_audited(&Method::POST, "/v1/jobs/j1/cancel"));
        assert!(self_audited(&Method::POST, "/v1/approvals/apr_1"));
        assert!(self_audited(&Method::POST, "/v1/agents/cli"));
        // 2.5's command surface and 3.5's review actions write their own.
        assert!(self_audited(&Method::POST, "/v1/commands"));
        assert!(self_audited(&Method::POST, "/v1/jobs/j1/integrate"));
        assert!(self_audited(&Method::POST, "/v1/jobs/j1/discard"));
        assert!(self_audited(&Method::POST, "/v1/jobs/j1/hunks/discard"));
        assert!(!self_audited(&Method::POST, "/v1/jobs/j1/notes"));
        assert!(!self_audited(&Method::POST, "/v1/hooks/t1"));
        assert!(!self_audited(&Method::GET, "/v1/jobs"));
    }

    #[test]
    fn the_query_reads_since_action_and_limit() {
        let q = query_of(Some("since=1d&action=job.&limit=5"), 100_000_000).unwrap();
        assert_eq!(q.since_ms, Some(100_000_000 - 86_400_000));
        assert_eq!(q.action.as_deref(), Some("job."));
        assert_eq!(q.limit, Some(5));
        assert!(query_of(Some("since=soon"), 0).is_err());
        assert_eq!(query_of(None, 5).unwrap().since_ms, None);
    }
}
