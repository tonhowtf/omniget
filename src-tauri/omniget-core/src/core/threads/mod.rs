//! Central threads: an event-sourced engine ported from T3 Code (plan §3.1).
//!
//! `Command` → [`decider::decide`] (pure) → events → one SQLite transaction
//! (append + SQL projections + command receipt) → broadcast. The database is
//! `<app_data>/llm/threads.db` (WAL). The JSONL of the coordinator stays as
//! the native driver's model context; this database is the source of truth of
//! what the UI shows.
//!
//! - [`model`]: commands, persisted events, the light read model.
//! - [`decider`]: the pure decision function (and the runtime-event fold).
//! - [`store`]: schema, projectors, receipts, cursors, read queries.
//! - [`engine`]: the single-writer actor and the read helpers.
//! - [`migrate`]: C-5, the old conversations imported as threads.
//! - [`git`]: worktree per thread, checkpoint per turn, diff, revert, git
//!   actions (T2/T7).
//! - [`usage`]: turn cost from the price table, rate limits per instance (T8).
//! - [`external`]: sessions of CLIs run outside OmniGet as read-only threads.

pub mod decider;
pub mod engine;
pub mod external;
pub mod git;
pub mod migrate;
pub mod model;
pub mod store;
pub mod usage;

pub use decider::{decide, DecideCtx, DecideError};
pub use engine::{DispatchResult, ThreadsEngine};
pub use model::{Command, CommandEnvelope, DomainEvent, ReadModel, StoredEvent};
pub use store::{EventsPage, Snapshot, TurnsPage};

/// `<app_data>/llm/threads.db`.
pub fn default_db_path() -> Option<std::path::PathBuf> {
    crate::core::llm::roster_store::llm_dir().map(|d| d.join("threads.db"))
}

#[cfg(test)]
mod git_tests;
#[cfg(test)]
mod tests;
