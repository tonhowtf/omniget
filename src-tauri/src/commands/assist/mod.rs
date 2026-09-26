//! Tauri commands of the assistant subsystem (`omniget_core::core::assist`).
//! One file per owner; the orchestrator registers the commands in `lib.rs`.

pub mod bots;
pub mod groups;
pub mod learning;
pub mod memory;
pub mod missions;
pub mod packs;
pub mod reading;
pub mod runs;
