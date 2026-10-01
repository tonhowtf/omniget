//! Working / waiting / done per ring.
//!
//! Two sources, both already in the app: the bus, for OmniGet's own agents
//! (they all share the `omniget` ring), and the mtime of the session logs of
//! a coding CLI running in a terminal (`cli_usage::activity::poll`, which
//! never opens a log). A log cannot say "waiting", so an outside CLI is only
//! ever working or done.

use omniget_core::core::llm::cli_usage::activity::Active;
use omniget_core::core::omni::bus::BusEvent;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

use super::providers::OMNIGET_ID;

/// How long "done" stays lit before the dot goes away.
pub const DONE_MS: i64 = 60_000;
/// How far back `cli_usage::activity::poll` looks for a fresh log.
pub const EXTERNAL_WINDOW_SECS: u64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Activity {
    #[default]
    Idle,
    Working,
    Waiting,
    Done,
}

/// The ring a CLI seen by `cli_usage::activity` belongs to.
pub fn ring_of_cli(cli: &str) -> Option<&'static str> {
    match cli {
        "Claude Code" => Some("claude"),
        "Codex" => Some("codex"),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct Tracker {
    /// OmniGet agents with a turn alive, and those stopped at a permission.
    running: HashSet<String>,
    asking: HashSet<String>,
    external: HashSet<&'static str>,
    done_at: HashMap<&'static str, i64>,
}

impl Tracker {
    pub fn note_bus(&mut self, event: &BusEvent, now_ms: i64) {
        match event {
            BusEvent::TurnStarted { agent, .. } => {
                self.running.insert(agent.clone());
                self.asking.remove(agent);
            }
            BusEvent::ToolAsk { agent, .. } => {
                self.running.insert(agent.clone());
                self.asking.insert(agent.clone());
            }
            BusEvent::ToolCalled { agent, .. } | BusEvent::TokenDelta { agent, .. } => {
                self.asking.remove(agent);
            }
            BusEvent::TurnEnded { agent, .. } | BusEvent::BudgetHit { agent } => {
                self.asking.remove(agent);
                if self.running.remove(agent) && self.running.is_empty() {
                    self.done_at.insert(OMNIGET_ID, now_ms);
                }
            }
            _ => {}
        }
    }

    /// The CLIs whose log moved in the last poll. One that was there and is
    /// not any more has finished.
    pub fn note_external(&mut self, active: &[Active], now_ms: i64) {
        let seen: HashSet<&'static str> =
            active.iter().filter_map(|a| ring_of_cli(a.cli)).collect();
        for gone in self.external.difference(&seen) {
            self.done_at.insert(gone, now_ms);
        }
        self.external = seen;
    }

    pub fn of(&self, ring: &str, now_ms: i64) -> Activity {
        let live = if ring == OMNIGET_ID {
            if !self.asking.is_empty() {
                Some(Activity::Waiting)
            } else if !self.running.is_empty() {
                Some(Activity::Working)
            } else {
                None
            }
        } else {
            self.external.contains(ring).then_some(Activity::Working)
        };
        live.unwrap_or_else(|| match self.done_at.get(ring) {
            Some(at) if now_ms - at < DONE_MS => Activity::Done,
            _ => Activity::Idle,
        })
    }
}
