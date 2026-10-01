use std::sync::{Arc, Mutex, OnceLock};

use omniget_core::core::llm::error::ERR_LLM_MODEL;
use omniget_core::core::llm::providers::Provider;
use omniget_core::core::llm::types::{ModelRef, ProviderId};
use omniget_core::core::llm::wire_probe::{self, ProbeCase, ProbeRound};
use serde_json::Value;
use tauri::State;

use crate::llm_manager::LlmManager;
use crate::AppState;

fn last_round() -> &'static Mutex<Option<ProbeRound>> {
    static LAST: OnceLock<Mutex<Option<ProbeRound>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn cases_for(names: Option<Vec<String>>) -> Vec<ProbeCase> {
    let all: Vec<ProbeCase> = wire_probe::default_cases()
        .into_iter()
        .chain(std::iter::once(wire_probe::reasoning_case()))
        .collect();
    match names {
        None => wire_probe::default_cases(),
        Some(wanted) => all
            .into_iter()
            .filter(|c| wanted.iter().any(|w| w == c.param))
            .collect(),
    }
}

fn resolve_provider(llm: &LlmManager, provider: &str) -> Result<Arc<dyn Provider>, String> {
    llm.probe_provider_for(&ProviderId::new(provider))
        .ok_or_else(|| {
            format!(
                "{ERR_LLM_MODEL}: no usable client for `{provider}` (no key, or unknown provider)"
            )
        })
}

async fn run_round(
    llm: &LlmManager,
    provider: String,
    model: String,
    cases: Option<Vec<String>>,
    estimate_only: Option<bool>,
) -> Result<Value, String> {
    let cases = cases_for(cases);
    if cases.is_empty() {
        return Err(format!("{ERR_LLM_MODEL}: no probe case selected"));
    }

    if estimate_only.unwrap_or(false) {
        let est = wire_probe::estimate(&cases, None);
        return serde_json::to_value(est).map_err(|e| e.to_string());
    }

    let client = resolve_provider(llm, &provider)?;
    let target = ModelRef {
        provider: ProviderId::new(provider),
        model,
    };
    let started_at_ms = now_ms();
    let results = wire_probe::run(client, &target, &cases).await;
    let round = ProbeRound::new(&target, started_at_ms, results);
    let value = serde_json::to_value(&round).map_err(|e| e.to_string())?;
    if let Ok(mut slot) = last_round().lock() {
        *slot = Some(round);
    }
    Ok(value)
}

#[tauri::command]
pub async fn llm_wire_probe_run(
    state: State<'_, AppState>,
    provider: String,
    model: String,
    cases: Option<Vec<String>>,
    estimate_only: Option<bool>,
) -> Result<Value, String> {
    run_round(&state.llm, provider, model, cases, estimate_only).await
}

#[tauri::command]
pub async fn llm_wire_probe_last() -> Result<Value, String> {
    let round = last_round()
        .lock()
        .map_err(|_| "ERR_LLM_STATE: wire probe lock poisoned".to_string())?
        .clone();
    match round {
        Some(r) => serde_json::to_value(r).map_err(|e| e.to_string()),
        None => Ok(Value::Null),
    }
}
