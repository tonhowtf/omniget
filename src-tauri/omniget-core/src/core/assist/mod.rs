//! Personal assistants: bots with an identity of their own, memory that grows
//! with the user, the reading companion, groups, and the durable record of
//! runs (sessions, events, receipts, permission requests).
//!
//! The split (spec 02, "Separe quatro coisas"):
//! - connection profile → `llm::cli_runtime::accounts` / the vault (unchanged);
//! - bot identity → [`bots`] (keyed by the roster's `AgentDef::id`);
//! - execution session → [`runs`];
//! - memory scope → [`ctx`] + [`memory`].
//!
//! SQLite (`<llm>/assist.db`, [`db`]) is the canonical store of every entity
//! here. Indexes (FTS) are derived and rebuildable. The project KB
//! (`llm::kb`) and the World memory (`llm::brain`) stay separate on purpose.
//!
//! Tools the model can call live in [`tools`]: one registry, reached from the
//! broker (native runtime, via `code_tools::current_turn`) and from the
//! scoped MCP projection (CLI/ACP runtimes, via a per-session token in
//! [`projection`]). Either way the scope set is computed here, in the backend,
//! never taken from the model.

pub mod authority;
pub mod bots;
pub mod ctx;
pub mod db;
pub mod external_config;
pub mod external_files;
pub mod groups;
pub mod learning;
pub mod media_provider;
pub(crate) mod media_retrieval;
pub mod media_tools;
pub mod memory;
pub mod missions;
pub mod packs;
pub mod projection;
pub mod reading;
pub mod runs;
pub mod tools;
pub mod web;

/// Milliseconds since the epoch; every table stores time this way.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// A fresh opaque id.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A hook that changes nothing (a module that has no per-turn context yet).
pub struct NoAugment;

impl crate::core::llm::coordinator::TurnAugment for NoAugment {
    fn augment(
        &self,
        _agent: &mut crate::core::llm::agent::AgentDef,
        _conversation_id: &str,
        _user_input: &str,
    ) -> Option<String> {
        None
    }
}

/// Per-turn hooks, in the order the coordinator runs them: bot identity and
/// skills first (they change grants), then the room, memory and reading.
pub fn augments() -> Vec<std::sync::Arc<dyn crate::core::llm::coordinator::TurnAugment>> {
    vec![
        bots::augment(),
        groups::augment(),
        memory::augment(),
        reading::augment(),
        learning::augment(),
    ]
}
