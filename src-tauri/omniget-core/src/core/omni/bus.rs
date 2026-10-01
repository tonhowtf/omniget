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
    /// A budget policy was crossed when a turn settled (3.3): `kind` is
    /// `hard` (the scope is paused: owners cancel its work, `incident`
    /// holds the id) or `warn`. `scope` is `global`, `agent:<id>`,
    /// `job:<id>`, `loop:<id>`, `mission:<id>` or `account:<id>`.
    /// `request` is the turn that crossed it (it is ending by itself).
    BudgetCrossed {
        agent: String,
        #[serde(default)]
        request: String,
        scope: String,
        kind: String,
        #[serde(default)]
        incident: Option<String>,
        observed: f64,
        limit: f64,
        metric: String,
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
