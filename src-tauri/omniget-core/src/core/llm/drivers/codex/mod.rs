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

pub fn register() {
    register_driver(registration());
}

#[cfg(test)]
mod tests;
