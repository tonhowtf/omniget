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

pub fn default_db_path() -> Option<std::path::PathBuf> {
    crate::core::llm::roster_store::llm_dir().map(|d| d.join("threads.db"))
}

#[cfg(test)]
mod tests;
