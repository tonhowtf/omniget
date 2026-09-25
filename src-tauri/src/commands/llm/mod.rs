//! Tauri commands of the LLM section (Phase 2), plus what they all share: the
//! `llm://*` events, the delta coalescer that keeps a fast model from flooding
//! the webview, and the bus forwarder. Owned by f2-llm-commands.

pub mod accounts;
pub mod chat;
pub mod help;
pub mod help_redaction;
pub mod jobs;
pub mod local;
pub mod mcp;
pub mod models;
pub mod observatory;
pub mod prompts;
pub mod prune;
pub mod roster;
pub mod skills;
pub mod wire_probe;

use std::time::{Duration, Instant};

use omniget_core::core::llm::types::TurnEvent;
use omniget_core::core::omni::bus::BusEvent;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const EVENT_TURN: &str = "llm://turn";
pub const EVENT_TELEMETRY: &str = "llm://telemetry";
/// `BusEvent::ToolAsk` re-emitted for the chat shell (name agreed with
/// f2-llm-shell-ui).
pub const EVENT_TOOL_ASK: &str = "llm://tool-ask";
/// `BusEvent::Rerouted` re-emitted: the Observatory draws it on the timeline
/// and the chat shows "switched to X".
pub const EVENT_REROUTED: &str = "llm://rerouted";

/// Text deltas are merged and emitted at most this often (budget: ≤ 30 emits/s
/// per turn). Everything else goes through the moment it arrives.
pub const DELTA_HZ: u32 = 30;
/// Telemetry ticks only while a turn is alive, at 2 Hz; 0 Hz at rest.
pub const TELEMETRY_HZ: u32 = 2;

#[derive(Debug, Clone, Serialize)]
pub struct TurnEnvelope<'a> {
    pub request_id: &'a str,
    pub event: &'a TurnEvent,
}

/// One emit per event, in the shape `{ request_id, event }`.
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

/// Merges consecutive `TextDelta`s into one event per tick.
///
/// The clock is an argument, never `Instant::now()` inside, so the whole thing
/// is testable without sleeping.
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

    /// Feeds one event in and gets back what should be emitted right now.
    ///
    /// * a `TextDelta` accumulates and only comes out once `interval` elapsed;
    /// * any other event flushes what is pending first, so order is preserved.
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

    /// Whatever is still buffered, at the end of the turn.
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

/// Emits `llm://telemetry` at 2 Hz while any turn is alive, then stops. Only
/// one ticker runs at a time, and nothing ticks at rest.
pub fn spawn_telemetry_ticker(app: AppHandle) {
    let manager = app.state::<crate::AppState>().llm.clone();
    if manager.claim_telemetry_ticker() {
        return; // already ticking
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

/// The forwarder itself, for a caller that already claimed the slot.
fn spawn_bus_forwarder_claimed(
    app: AppHandle,
    manager: std::sync::Arc<crate::llm_manager::LlmManager>,
) {
    let mut rx = manager.bus().subscribe();
    spawn_external_agent_watcher(app.clone(), manager.clone());
    tauri::async_runtime::spawn(async move {
        // The mascot half of the bus: `rules` is pure and already debounces
        // (same intent as the one on screen → None).
        let mut omni = omniget_core::core::omni::state::OmniState::new();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    manager.note_bus(&event);
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

/// Claude Code or Codex working in a terminal animates the Omni too. Only the
/// mtime of their session logs is looked at, every 10 s, and only while the
/// floating pet is switched on.
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
            // Our own turns already drive the mascot.
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

/// Which Tauri event a bus event becomes, and with what payload. Pure, so the
/// wiring can be proven without a running Tauri app.
///
/// `ToolAsk.request_id` is the id the Coordinator minted for the turn — the
/// same one `llm_turn_start` returned — so the front can filter by equality.
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

/// Everything a command needs before it can run a turn: the tool executor gets
/// its `AppHandle` and the bus forwarder starts. Idempotent and cheap.
pub fn ensure_wired(app: &AppHandle) {
    let manager = app.state::<crate::AppState>().llm.clone();
    manager.attach_app(app);
    if !manager.claim_bus_forwarder() {
        // First call of the process: also clear the throwaway conversations a
        // bridge turn may have left behind if the app died mid-turn.
        let swept = manager.sweep_bridge_conversations();
        if swept > 0 {
            tracing::info!("[llm] swept {swept} orphan bridge conversation(s)");
        }
        prune::install(app);
        spawn_bus_forwarder_claimed(app.clone(), manager);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omniget_core::core::llm::types::FinishReason;

    fn delta(text: &str) -> TurnEvent {
        TurnEvent::TextDelta { text: text.into() }
    }

    fn text_of(events: &[TurnEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn coalescer_merges_deltas_inside_one_tick() {
        let mut c = DeltaCoalescer::new(30);
        let t0 = Instant::now();
        // The first delta goes out immediately (first token latency matters).
        assert_eq!(text_of(&c.push(delta("a"), t0)), "a");
        // The next ones inside the same 33 ms window are buffered.
        assert!(c.push(delta("b"), t0 + Duration::from_millis(5)).is_empty());
        assert!(c
            .push(delta("c"), t0 + Duration::from_millis(10))
            .is_empty());
        let out = c.push(delta("d"), t0 + Duration::from_millis(40));
        assert_eq!(text_of(&out), "bcd");
    }

    #[test]
    fn coalescer_never_exceeds_30_emits_per_second() {
        let mut c = DeltaCoalescer::new(DELTA_HZ);
        let t0 = Instant::now();
        let mut emits = 0;
        // 1000 deltas spread over exactly one second.
        for i in 0..1000u64 {
            let now = t0 + Duration::from_micros(i * 1000);
            emits += c.push(delta("x"), now).len();
        }
        if c.flush().is_some() {
            emits += 1;
        }
        assert!(emits <= 31, "coalescer emitted {emits} times in 1 s");
    }

    #[test]
    fn coalescer_keeps_the_whole_text() {
        let mut c = DeltaCoalescer::new(DELTA_HZ);
        let t0 = Instant::now();
        let mut seen = String::new();
        for (i, ch) in "hello world, this is a stream".chars().enumerate() {
            let now = t0 + Duration::from_micros(i as u64 * 500);
            for e in c.push(delta(&ch.to_string()), now) {
                if let TurnEvent::TextDelta { text } = e {
                    seen.push_str(&text);
                }
            }
        }
        if let Some(TurnEvent::TextDelta { text }) = c.flush() {
            seen.push_str(&text);
        }
        assert_eq!(seen, "hello world, this is a stream");
    }

    #[test]
    fn a_non_text_event_flushes_first_and_keeps_order() {
        let mut c = DeltaCoalescer::new(DELTA_HZ);
        let t0 = Instant::now();
        let _ = c.push(delta("a"), t0);
        let _ = c.push(delta("b"), t0 + Duration::from_millis(1));
        let out = c.push(
            TurnEvent::Finished {
                reason: FinishReason::Stop,
            },
            t0 + Duration::from_millis(2),
        );
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], TurnEvent::TextDelta { .. }));
        assert!(matches!(out[1], TurnEvent::Finished { .. }));
        assert_eq!(text_of(&out), "b");
    }

    #[test]
    fn flush_is_empty_when_nothing_is_pending() {
        let mut c = DeltaCoalescer::new(DELTA_HZ);
        assert!(c.flush().is_none());
        let _ = c.push(delta("a"), Instant::now());
        assert!(c.flush().is_none(), "the first delta was already emitted");
    }

    /// A provider that asks for one tool on the first turn and answers with
    /// text on the second, so the tool loop actually closes.
    #[derive(Debug)]
    struct ToolThenText {
        calls: std::sync::atomic::AtomicU32,
        tool: String,
    }

    #[async_trait::async_trait]
    impl omniget_core::core::llm::providers::Provider for ToolThenText {
        async fn turn(
            &self,
            _req: omniget_core::core::llm::types::TurnRequest,
        ) -> Result<
            futures::stream::BoxStream<'static, TurnEvent>,
            omniget_core::core::llm::error::LlmError,
        > {
            use futures::StreamExt;
            use omniget_core::core::llm::types::FinishReason;
            let first = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
            let script = if first {
                vec![
                    TurnEvent::ToolCallStart {
                        id: "call-1".into(),
                        name: self.tool.clone(),
                    },
                    TurnEvent::ToolCallDelta {
                        id: "call-1".into(),
                        input_json_delta: "{}".into(),
                    },
                    TurnEvent::ToolCallEnd {
                        id: "call-1".into(),
                    },
                    TurnEvent::Finished {
                        reason: FinishReason::ToolUse,
                    },
                ]
            } else {
                vec![
                    TurnEvent::TextDelta {
                        text: "done".into(),
                    },
                    TurnEvent::Finished {
                        reason: FinishReason::Stop,
                    },
                ]
            };
            Ok(futures::stream::iter(script).boxed())
        }
    }

    /// The defect the Phase 2 verifier caught: two request ids meant the
    /// permission prompt never matched its turn and the turn hung for the
    /// 120 s ask timeout.
    ///
    /// This drives the real path — `LlmManager::turn_stream` (what
    /// `llm_turn_start` calls) → Coordinator → ToolBroker with `GrantMode::Ask`
    /// → bus — and checks the id the front would filter by. The Tauri `emit` is
    /// the only thing left out, and `tauri_event_for` is exactly what the
    /// forwarder hands it.
    #[tokio::test]
    async fn a_tool_ask_carries_the_request_id_of_its_turn_and_answering_unblocks_it() {
        use futures::StreamExt;
        use omniget_core::core::llm::agent::{GrantMode, ModelPolicy, ToolGrant, ToolSource};
        use omniget_core::core::llm::roster_store;
        use omniget_core::core::llm::types::{ModelRef, ProviderId};

        const PROVIDER: &str = "test-tool-then-text";
        // A tool the broker really knows about, so the grant is not a fiction.
        let tool = crate::llm_manager::mcp_specs()
            .first()
            .map(|s| s.name.clone())
            .expect("the MCP server exposes tools");

        let manager = std::sync::Arc::new(crate::llm_manager::LlmManager::new());
        manager
            .set_root(std::env::temp_dir().join(format!("omniget-ask-{}", uuid::Uuid::new_v4())));
        manager.register_provider(
            PROVIDER,
            std::sync::Arc::new(ToolThenText {
                calls: std::sync::atomic::AtomicU32::new(0),
                tool: tool.clone(),
            }),
        );
        let mut agent = roster_store::default_roster().remove(0);
        agent.model = ModelPolicy::Fixed {
            model: ModelRef {
                provider: ProviderId::new(PROVIDER),
                model: "m".into(),
            },
        };
        agent.tools = vec![ToolGrant {
            source: ToolSource::Internal { name: tool.clone() },
            mode: GrantMode::Ask,
        }];
        manager.roster_update(agent.clone()).unwrap();

        // The bus subscriber stands in for the forwarder `ensure_wired` starts.
        let mut bus_rx = manager.bus().subscribe();

        let (request_id, _cancel, mut stream) = manager
            .turn_stream("c-ask", &agent.id, "baixa isso")
            .await
            .expect("the turn must start");

        // The ask reaches the bus…
        let ask = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match bus_rx.recv().await {
                    Ok(event) => {
                        if let Some(pair) = tauri_event_for(&event) {
                            if pair.0 == EVENT_TOOL_ASK {
                                return pair.1;
                            }
                        }
                    }
                    Err(_) => panic!("the bus closed before the ask"),
                }
            }
        })
        .await
        .expect("no llm://tool-ask in 5 s — the turn is hanging on the ask timeout");

        // …with the SAME request_id the command returned. This is the bug.
        assert_eq!(
            ask["request_id"].as_str(),
            Some(request_id.as_str()),
            "llm://tool-ask carries a different request_id than llm_turn_start returned"
        );
        assert_eq!(ask["tool"].as_str(), Some(tool.as_str()));
        assert_eq!(ask["agent"].as_str(), Some(agent.id.as_str()));

        // Answering releases the turn well inside the 120 s ask timeout.
        let tool_call_id = ask["tool_call_id"].as_str().unwrap().to_string();
        manager
            .answer_tool(&request_id, &tool_call_id, true, false)
            .expect("the broker had the pending ask");

        let finished = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = stream.next().await {
                if matches!(event, TurnEvent::Finished { .. }) {
                    return true;
                }
            }
            false
        })
        .await
        .expect("the turn did not finish after the answer");
        assert!(finished, "the stream ended without Finished");

        // And the first event of the stream is the `Started` with that id.
        manager.finish_turn(&request_id, &agent.id);
    }

    #[test]
    fn tauri_event_for_only_maps_the_two_events_the_ui_listens_to() {
        use omniget_core::core::llm::types::Usage;
        let ask = BusEvent::ToolAsk {
            agent: "omni".into(),
            request_id: "req-1".into(),
            tool_call_id: "call-1".into(),
            tool: "download_url".into(),
            preview: String::new(),
        };
        let (name, payload) = tauri_event_for(&ask).unwrap();
        assert_eq!(name, EVENT_TOOL_ASK);
        assert_eq!(payload["request_id"], "req-1");
        assert_eq!(payload["tool_call_id"], "call-1");

        let rerouted = BusEvent::Rerouted {
            agent: "omni".into(),
            from: "a".into(),
            to: "b".into(),
            why: "ERR_LLM_RATE".into(),
        };
        assert_eq!(tauri_event_for(&rerouted).unwrap().0, EVENT_REROUTED);

        // Everything else stays inside the process.
        assert!(tauri_event_for(&BusEvent::TurnEnded {
            agent: "omni".into(),
            usage: Usage::default(),
        })
        .is_none());
    }

    #[test]
    fn the_event_names_are_the_ones_the_ui_listens_to() {
        // f2-llm-shell-ui and f2-observatory-ui coded against these strings.
        assert_eq!(EVENT_TURN, "llm://turn");
        assert_eq!(EVENT_TELEMETRY, "llm://telemetry");
        assert_eq!(EVENT_TOOL_ASK, "llm://tool-ask");
        assert_eq!(EVENT_REROUTED, "llm://rerouted");
    }
}
