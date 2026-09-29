//! The app's side of the budget policies (3.3, S18, D5): opening
//! `<llm dir>/budget.db` at boot, the price table and the `llm://budget`
//! event, and the three doors to the policies and incidents — the window
//! (`llm_budget_*`), the bridge (`GET /v1/budget`, `POST /v1/budget/policies`,
//! `POST /v1/budget/incidents/{id}`) and `omniget agent budget`. The engine
//! itself (scopes, ledger, gate, incidents) is
//! `omniget_core::core::llm::budget::policy`.
//!
//! "Raise and resume" settles the incident and restarts every Loop it had
//! paused (`Jobs::budget_resumed`); the cancelled jobs of the scope stay
//! cancelled, as their result is unknown.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use omniget_core::core::llm::audit::Actor;
use omniget_core::core::llm::budget::policy::{
    self, BudgetPolicy, Engine, Metric, Price, Resolution, Scope, Window,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

/// Every change of a policy or an incident, and every warning.
pub const EVENT_BUDGET: &str = "llm://budget";

/// Opens the policies before any turn runs and wires the price table and
/// the event. The quota readings are wired where the CLI runtime is built
/// (`llm_manager`).
pub fn boot(app: &AppHandle) {
    let Some(dir) = omniget_core::core::llm::roster_store::llm_dir() else {
        return;
    };
    if let Err(e) = policy::init(&dir.join(policy::DB_FILE)) {
        tracing::warn!("[budget] cannot open the budget policies: {e}");
        return;
    }
    policy::set_price_source(std::sync::Arc::new(|model: String| {
        Box::pin(async move { price_of(&model).await })
    }));
    let app = app.clone();
    policy::set_listener(std::sync::Arc::new(move |event| {
        let _ = app.emit(EVENT_BUDGET, json!(event));
    }));
}

/// The CLI's model aliases, as the price table names them.
fn table_name(model: &str) -> &str {
    match model.trim() {
        "haiku" => "claude-haiku-4-5",
        "sonnet" => "claude-sonnet-4-5",
        "opus" => "claude-opus-4-1",
        other => other,
    }
}

/// A model's price from the LiteLLM table (read from its cache on disk,
/// fetched once a day). Only asked when a dollar ceiling applies to a turn
/// whose provider reported no price.
async fn price_of(model: &str) -> Option<Price> {
    let p = omniget_core::core::tools::pricing::price_for(table_name(model)).await?;
    Some(Price {
        input_per_m: p.input_per_m?,
        output_per_m: p.output_per_m.unwrap_or(0.0),
    })
}

fn engine() -> Result<std::sync::Arc<Engine>, String> {
    policy::active().ok_or_else(|| "ERR_LLM_BUDGET: the budget policies are not open".to_string())
}

/// What a person sends to save a policy: the D5 fields, as words.
#[derive(Debug, Clone, Deserialize)]
pub struct PolicyBody {
    #[serde(default)]
    pub id: Option<String>,
    pub scope: String,
    pub metric: String,
    pub window: String,
    pub amount: f64,
    #[serde(default)]
    pub warn_pct: Option<f64>,
    #[serde(default)]
    pub hard_stop: Option<bool>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

impl PolicyBody {
    pub fn into_policy(self) -> Result<BudgetPolicy, String> {
        let p = BudgetPolicy {
            id: self.id.unwrap_or_default(),
            scope: Scope::parse(&self.scope)?,
            metric: Metric::parse(&self.metric)?,
            window: Window::parse(&self.window)?,
            amount: self.amount,
            warn_pct: self.warn_pct.filter(|w| *w > 0.0),
            hard_stop: self.hard_stop.unwrap_or(true),
            enabled: self.enabled.unwrap_or(true),
            created_ms: 0,
            updated_ms: 0,
        };
        p.validate()?;
        Ok(p)
    }
}

/// `raise` (with `amount`) or `keep_paused`.
#[derive(Debug, Clone, Deserialize)]
pub struct ResolveBody {
    pub action: String,
    #[serde(default)]
    pub amount: Option<f64>,
}

impl ResolveBody {
    fn resolution(&self) -> Result<Resolution, String> {
        match self.action.as_str() {
            "raise" => Ok(Resolution::Raise(self.amount.ok_or_else(|| {
                "ERR_LLM_BUDGET: raise needs the new amount".to_string()
            })?)),
            "keep_paused" => Ok(Resolution::KeepPaused),
            other => Err(format!(
                "ERR_LLM_BUDGET: unknown action `{other}` (raise, keep_paused)"
            )),
        }
    }
}

/// Settles an incident and, on a raise, resumes what it paused.
pub fn resolve(id: &str, body: &ResolveBody, actor: Actor) -> Result<Value, String> {
    let inc = engine()?.resolve(id, body.resolution()?, actor)?;
    let resumed = if inc.state == "resolved" {
        crate::jobs::try_get()
            .map(|j| j.budget_resumed(id))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(json!({ "incident": inc, "resumed_loops": resumed }))
}

pub fn save(body: PolicyBody, actor: Actor) -> Result<Value, String> {
    let p = engine()?.upsert(body.into_policy()?, actor)?;
    Ok(json!(p))
}

// ── the window ──────────────────────────────────────────────────────────

/// Policies with where each stands, and the incidents (newest first).
#[tauri::command]
pub async fn llm_budget_state() -> Result<Value, String> {
    Ok(engine()?.state_view())
}

#[tauri::command]
pub async fn llm_budget_policy_save(policy: PolicyBody) -> Result<Value, String> {
    save(policy, Actor::window())
}

#[tauri::command]
pub async fn llm_budget_policy_delete(id: String) -> Result<bool, String> {
    engine()?.delete(&id, Actor::window())
}

#[tauri::command]
pub async fn llm_budget_incident_resolve(
    id: String,
    action: String,
    amount: Option<f64>,
) -> Result<Value, String> {
    resolve(&id, &ResolveBody { action, amount }, Actor::window())
}

// ── the bridge (`omniget agent budget`) ────────────────────────────────

fn status_of(e: &str) -> StatusCode {
    if e.contains("no incident") {
        StatusCode::NOT_FOUND
    } else if e.contains("not open") {
        StatusCode::SERVICE_UNAVAILABLE
    } else if e.contains("already") {
        StatusCode::CONFLICT
    } else {
        StatusCode::BAD_REQUEST
    }
}

fn reply(r: Result<Value, String>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => (status_of(&e), Json(json!({ "error": e }))).into_response(),
    }
}

pub fn http_state() -> Response {
    reply(engine().map(|e| e.state_view()))
}

pub fn http_save(body: PolicyBody, actor: Actor) -> Response {
    reply(save(body, actor))
}

pub fn http_resolve(id: &str, body: ResolveBody, actor: Actor) -> Response {
    reply(resolve(id, &body, actor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_body_is_checked_before_it_is_saved() {
        let ok = PolicyBody {
            id: None,
            scope: "loop:l1".into(),
            metric: "usd".into(),
            window: "run".into(),
            amount: 0.05,
            warn_pct: None,
            hard_stop: None,
            enabled: None,
        }
        .into_policy()
        .unwrap();
        assert!(ok.hard_stop && ok.enabled);
        assert_eq!(ok.scope, Scope::Loop("l1".into()));
        for (scope, metric, window, amount) in [
            ("loop:l1", "usd", "run", 0.0),
            ("team:x", "usd", "run", 1.0),
            ("global", "quota_pct", "day", 5.0),
            ("global", "dollars", "day", 5.0),
            ("global", "usd", "month", 5.0),
        ] {
            assert!(
                PolicyBody {
                    id: None,
                    scope: scope.into(),
                    metric: metric.into(),
                    window: window.into(),
                    amount,
                    warn_pct: None,
                    hard_stop: None,
                    enabled: None,
                }
                .into_policy()
                .is_err(),
                "{scope} {metric} {window} {amount}"
            );
        }
    }

    #[test]
    fn resolve_needs_an_amount_to_raise() {
        let raise = ResolveBody {
            action: "raise".into(),
            amount: None,
        };
        assert!(raise.resolution().is_err());
        let keep = ResolveBody {
            action: "keep_paused".into(),
            amount: None,
        };
        assert_eq!(keep.resolution().unwrap(), Resolution::KeepPaused);
        assert!(ResolveBody {
            action: "ignore".into(),
            amount: None
        }
        .resolution()
        .is_err());
    }

    #[test]
    fn the_cli_aliases_have_a_price_name() {
        assert_eq!(table_name("haiku"), "claude-haiku-4-5");
        assert_eq!(table_name("gpt-5"), "gpt-5");
    }
}
