//! Typed broadcast bus feeding telemetry, the mascot and the world. Owned by
//! f2-llm-coordinator. DRAFT by the orchestrator from
//! docs/agents/f2-llm-coordinator.md §5 (MediaEngine mould: `broadcast` of 256);
//! F5 codes against `BusEvent` today. Additive changes only without notice.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::core::llm::types::Usage;

pub const BUS_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BusEvent {
    TurnStarted {
        agent: String,
        conversation: String,
    },
    TokenDelta {
        agent: String,
        chars: u32,
    },
    ToolAsk {
        agent: String,
        request_id: String,
        tool_call_id: String,
        tool: String,
        /// What the permission prompt shows: the command, the path, the head
        /// of the patch. Empty for tools that have nothing worth showing.
        #[serde(default)]
        preview: String,
    },
    /// A `ToolAsk` ended, whatever ended it: `outcome` is `allow`, `deny`,
    /// `expired` (nobody answered in time) or `cancelled` (its turn ended).
    /// Exactly one per ask, so no window ever keeps a ghost question and an
    /// expiry is never silent.
    ToolAskResolved {
        agent: String,
        request_id: String,
        tool_call_id: String,
        tool: String,
        outcome: String,
        waited_ms: u64,
    },
    ToolCalled {
        agent: String,
        tool: String,
        ok: bool,
        ms: u32,
    },
    TurnEnded {
        agent: String,
        usage: Usage,
    },
    BudgetHit {
        agent: String,
    },
    Rerouted {
        agent: String,
        from: String,
        to: String,
        why: String,
    },
    DownloadQueued {
        id: u64,
    },
    DownloadFinished {
        id: u64,
    },
    DownloadFailed {
        id: u64,
        code: String,
    },
    ToolFinished {
        tool: String,
        ok: bool,
    },
    Idle {
        seconds: u32,
    },
    /// A coding CLI running outside the app (Claude Code, Codex in a terminal)
    /// wrote to its session log just now.
    ExternalAgentActive {
        cli: String,
        project: String,
    },
}

#[derive(Debug, Clone)]
pub struct Bus {
    tx: broadcast::Sender<BusEvent>,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BUS_CAPACITY);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BusEvent> {
        self.tx.subscribe()
    }

    /// Never blocks; without subscribers the event is dropped on purpose.
    pub fn emit(&self, event: BusEvent) {
        let _ = self.tx.send(event);
    }

    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_subscriber_gets_every_event() {
        let bus = Bus::new();
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        assert_eq!(bus.receiver_count(), 2);
        bus.emit(BusEvent::Idle { seconds: 3 });
        assert_eq!(a.recv().await.unwrap(), BusEvent::Idle { seconds: 3 });
        assert_eq!(b.recv().await.unwrap(), BusEvent::Idle { seconds: 3 });
    }

    #[test]
    fn emitting_without_subscribers_never_blocks_or_panics() {
        let bus = Bus::new();
        assert_eq!(bus.receiver_count(), 0);
        for i in 0..1_000 {
            bus.emit(BusEvent::DownloadQueued { id: i });
        }
    }

    #[test]
    fn a_slow_subscriber_lags_instead_of_stalling_the_bus() {
        let bus = Bus::new();
        let mut rx = bus.subscribe();
        for i in 0..(BUS_CAPACITY as u64 + 10) {
            bus.emit(BusEvent::DownloadQueued { id: i });
        }
        // The oldest events are dropped for that receiver, the bus is fine.
        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
        ));
        let mut drained = 0;
        while rx.try_recv().is_ok() {
            drained += 1;
        }
        assert_eq!(drained, BUS_CAPACITY, "the last {BUS_CAPACITY} survive");
        bus.emit(BusEvent::Idle { seconds: 1 });
        assert_eq!(rx.try_recv().unwrap(), BusEvent::Idle { seconds: 1 });
    }

    #[test]
    fn events_serialise_with_a_tagged_type_the_ui_can_switch_on() {
        let ev = BusEvent::ToolCalled {
            agent: "worker".into(),
            tool: "dl_add".into(),
            ok: true,
            ms: 12,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "tool_called");
        assert_eq!(json["ok"], true);
        let back: BusEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn the_reroute_event_carries_both_ends_and_the_reason() {
        let ev = BusEvent::Rerouted {
            agent: "worker".into(),
            from: "cli:max-1:sonnet".into(),
            to: "native:openrouter:sonnet".into(),
            why: "ERR_CLI_RATE".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "rerouted");
        assert_eq!(json["from"], "cli:max-1:sonnet");
        assert_eq!(json["why"], "ERR_CLI_RATE");
    }

    #[test]
    fn a_dropped_subscriber_stops_counting() {
        let bus = Bus::new();
        let rx = bus.subscribe();
        assert_eq!(bus.receiver_count(), 1);
        drop(rx);
        assert_eq!(bus.receiver_count(), 0);
    }
}
