use tauri::{AppHandle, Emitter};

use omniget_core::core::assist::runs::{self};

pub const EVENT_RUN: &str = "assist://run";
pub const ERR_ASSIST_RUNS: &str = "ERR_ASSIST_RUNS";

pub fn install(app: &AppHandle) {
    runs::enable();
    let emitter = app.clone();
    runs::set_sink(std::sync::Arc::new(move |update| {
        let _ = emitter.emit(EVENT_RUN, update);
    }));
    match runs::active().map(|r| r.reconcile()) {
        Some(Ok(out)) => {
            if !out.unknown.is_empty() || !out.interrupted.is_empty() {
                tracing::info!(
                    "[runs] after restart: {} unknown, {} interrupted (waiting for a person)",
                    out.unknown.len(),
                    out.interrupted.len()
                );
            }
        }
        Some(Err(e)) => tracing::warn!("[runs] reconcile: {e}"),
        None => tracing::warn!("[runs] no assistant database; runs are not recorded"),
    }
}
