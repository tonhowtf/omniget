//! Codex driver of the Central: `codex app-server` over JSON-RPC stdio.
//!
//! - `protocol.rs`: types generated from the CLI's own JSON Schema
//!   (`scripts/codex-schema/gen.mjs`; header says which codex version).
//! - `rpc.rs`: JSON-RPC client (ids, pending map, server requests).
//! - `launch.rs`: argv, `CODEX_HOME` from the account, access-mode table.
//! - `translate.rs`: app-server notifications/requests → `RuntimeEvent`.
//! - `supervisor.rs`: the process, its descendants, restart budget.
//! - `driver.rs`: the [`Driver`](super::Driver) implementation.
//!
//! Plug-in: `omniget_core::core::llm::drivers::codex::register()` once, before
//! the first Codex thread runs (the threads host does it next to `native`).

use std::sync::Arc;

use super::{register_driver, DriverRegistration};

pub mod driver;
pub mod launch;
#[rustfmt::skip]
pub mod protocol;
pub mod rpc;
pub mod supervisor;
pub mod translate;

pub use driver::{capabilities, CodexDriver};
pub use protocol::CODEX_PROTOCOL_VERSION;

/// The registration of the `codex` driver kind: every instance spawns the
/// real CLI and reads its account from `<app_data>/llm/accounts.json`.
pub fn registration() -> DriverRegistration {
    DriverRegistration {
        kind: "codex".into(),
        label: "Codex".into(),
        capabilities: capabilities(),
        factory: Arc::new(|instance, sink| {
            let accounts =
                super::super::cli_runtime::accounts::AccountStore::default_store().map(Arc::new);
            let driver = CodexDriver::new(
                instance,
                sink,
                Arc::new(supervisor::ProcessConnector),
                accounts,
            );
            Ok(Arc::new(driver) as Arc<dyn super::Driver>)
        }),
    }
}

/// Registers the `codex` driver kind. Idempotent.
pub fn register() {
    register_driver(registration());
}

#[cfg(test)]
mod tests;
