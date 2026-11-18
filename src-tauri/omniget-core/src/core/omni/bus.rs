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
        #[serde(default)]
        preview: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        impact: Option<crate::core::llm::impact::Impact>,
    },
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

    pub fn emit(&self, event: BusEvent) {
        let _ = self.tx.send(event);
    }

    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}
