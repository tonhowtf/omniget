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

pub const EVENT_BUDGET: &str = "llm://budget";

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

fn table_name(model: &str) -> &str {
    match model.trim() {
        "haiku" => "claude-haiku-4-5",
        "sonnet" => "claude-sonnet-4-5",
        "opus" => "claude-opus-4-1",
        other => other,
    }
}

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
