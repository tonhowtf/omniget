pub mod accounts;
pub mod chat;
pub mod help;
pub mod help_redaction;
pub mod jobs;
pub mod keys;
pub mod local;
pub mod mcp;
pub mod models;
pub mod observatory;
pub mod prompts;
pub mod prune;
pub mod roster;
pub mod skills;
pub mod watchdog;
pub mod wire_probe;

use std::time::{Duration, Instant};

use omniget_core::core::llm::types::TurnEvent;
use omniget_core::core::omni::bus::BusEvent;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const EVENT_TURN: &str = "llm://turn";
pub const EVENT_TELEMETRY: &str = "llm://telemetry";
pub const EVENT_TOOL_ASK: &str = "llm://tool-ask";
pub const EVENT_REROUTED: &str = "llm://rerouted";
pub const EVENT_TOOL_RESOLVED: &str = "omni://tool-resolved";

pub const DELTA_HZ: u32 = 30;
pub const TELEMETRY_HZ: u32 = 2;

#[derive(Debug, Clone, Serialize)]
pub struct TurnEnvelope<'a> {
    pub request_id: &'a str,
    pub event: &'a TurnEvent,
}

pub fn emit_turn(app: &AppHandle, request_id: &str, event: &TurnEvent) {
    if let Err(error) = app.emit(EVENT_TURN, TurnEnvelope { request_id, event }) {
        tracing::debug!("[llm] failed to emit {EVENT_TURN}: {error}");
    }
}

pub fn emit_telemetry(app: &AppHandle, snapshot: &crate::llm_manager::TelemetrySnapshot) {
    if let Err(error) = app.emit(EVENT_TELEMETRY, snapshot) {
        tracing::debug!("[llm] failed to emit {EVENT_TELEMETRY}: {error}");
    }
}

#[derive(Debug)]
pub struct DeltaCoalescer {
    interval: Duration,
    pending: String,
    last_flush: Option<Instant>,
}

impl DeltaCoalescer {
    pub fn new(hz: u32) -> Self {
        let hz = hz.max(1);
        Self {
            interval: Duration::from_nanos(1_000_000_000 / hz as u64),
            pending: String::new(),
            last_flush: None,
        }
    }

    pub fn push(&mut self, event: TurnEvent, now: Instant) -> Vec<TurnEvent> {
        match event {
            TurnEvent::TextDelta { text } => {
                self.pending.push_str(&text);
                let due = match self.last_flush {
                    None => true,
                    Some(last) => now.duration_since(last) >= self.interval,
                };
                if due {
                    self.last_flush = Some(now);
                    self.take().into_iter().collect()
                } else {
                    Vec::new()
                }
            }
            other => {
                self.last_flush = Some(now);
                let mut out = Vec::new();
                if let Some(pending) = self.take() {
                    out.push(pending);
                }
                out.push(other);
                out
            }
        }
    }

    pub fn flush(&mut self) -> Option<TurnEvent> {
        self.take()
    }

    fn take(&mut self) -> Option<TurnEvent> {
        if self.pending.is_empty() {
            return None;
        }
        Some(TurnEvent::TextDelta {
            text: std::mem::take(&mut self.pending),
        })
    }
}

pub fn spawn_telemetry_ticker(app: AppHandle) {
    let manager = app.state::<crate::AppState>().llm.clone();
    if manager.claim_telemetry_ticker() {
        return;
    }
    let period = Duration::from_millis(1000 / TELEMETRY_HZ.max(1) as u64);
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(period).await;
            emit_telemetry(&app, &manager.telemetry());
            if manager.active_turns() == 0 {
                break;
            }
        }
        manager.release_telemetry_ticker();
    });
}

fn spawn_bus_forwarder_claimed(
    app: AppHandle,
    manager: std::sync::Arc<crate::llm_manager::LlmManager>,
) {
    let mut rx = manager.bus().subscribe();
    spawn_external_agent_watcher(app.clone(), manager.clone());
    tauri::async_runtime::spawn(async move {
        let mut omni = omniget_core::core::omni::state::OmniState::new();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    manager.note_bus(&event);
                    crate::jobs::on_bus(&event);
                    if let Some(intent) = omniget_core::core::omni::rules::rules(&event, &omni) {
                        omni.apply(&intent);
                        if let Ok(payload) = serde_json::to_value(&intent)
                            .map_err(|e| e.to_string())
                            .and_then(|v| crate::commands::pet::normalize_intent(&v))
                        {
                            let _ = app.emit(crate::commands::pet::EVENT_INTENT, payload);
                        }
                    }
                    if let Some((name, payload)) = tauri_event_for(&event) {
                        let _ = app.emit(name, payload);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!("[llm] bus forwarder lagged by {n}");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn spawn_external_agent_watcher(
    app: AppHandle,
    manager: std::sync::Arc<crate::llm_manager::LlmManager>,
) {
    use omniget_core::core::llm::cli_usage::activity;
    tauri::async_runtime::spawn(async move {
        let mut was_active = false;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            if !crate::commands::pet::is_enabled(&app) {
                continue;
            }
            if manager.active_turns() > 0 {
                continue;
            }
            let active =
                tokio::task::spawn_blocking(|| activity::poll(std::time::Duration::from_secs(20)))
                    .await
                    .unwrap_or_default();
            match active.first() {
                Some(a) => {
                    was_active = true;
                    manager.bus().emit(BusEvent::ExternalAgentActive {
                        cli: a.cli.to_string(),
                        project: a.project.clone(),
                    });
                }
                None if was_active => {
                    was_active = false;
                    manager.bus().emit(BusEvent::Idle { seconds: 0 });
                }
                None => {}
            }
        }
    });
}

pub fn tauri_event_for(event: &BusEvent) -> Option<(&'static str, serde_json::Value)> {
    match event {
        BusEvent::ToolAsk {
            agent,
            request_id,
            tool_call_id,
            tool,
            preview,
        } => Some((
            EVENT_TOOL_ASK,
            serde_json::json!({
                "agent": agent,
                "request_id": request_id,
                "tool_call_id": tool_call_id,
                "tool": tool,
                "preview": preview,
            }),
        )),
        BusEvent::ToolAskResolved {
            agent,
            request_id,
            tool_call_id,
            tool,
            outcome,
            waited_ms,
        } => Some((
            EVENT_TOOL_RESOLVED,
            serde_json::json!({
                "agent": agent,
                "request_id": request_id,
                "tool_call_id": tool_call_id,
                "tool": tool,
                "outcome": outcome,
                "allow": outcome == "allow",
                "waited_ms": waited_ms,
            }),
        )),
        BusEvent::Rerouted {
            agent,
            from,
            to,
            why,
        } => Some((
            EVENT_REROUTED,
            serde_json::json!({
                "agent": agent,
                "from": from,
                "to": to,
                "why": why,
            }),
        )),
        _ => None,
    }
}

pub fn ensure_wired(app: &AppHandle) {
    let manager = app.state::<crate::AppState>().llm.clone();
    manager.attach_app(app);
    if !manager.claim_bus_forwarder() {
        let swept = manager.sweep_bridge_conversations();
        if swept > 0 {
            tracing::info!("[llm] swept {swept} orphan bridge conversation(s)");
        }
        prune::install(app);
        watchdog::install(app);
        spawn_bus_forwarder_claimed(app.clone(), manager);
    }
}
