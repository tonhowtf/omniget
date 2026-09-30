//! What the program in a terminal is doing, and how agents are launched so
//! that it can be known.
//!
//! - [`state`]: the state machine, fed by hooks, OSC/title sequences and
//!   heuristics, in that order of authority.
//! - [`launch`]: recipes that start Claude Code, Codex and shells with the
//!   hooks, notification settings and shell integration that make those
//!   signals exist.

pub mod launch;
pub mod state;

pub use state::{AgentKind, AgentSignal, AgentState, AgentTracker, SignalSource};
