//! Execution side of the catalog (plan F9): catalog loops, workflows as
//! playbooks, the project wizard, global agents (`omniget agent run <agent>
//! --tool <x>`) and OmniGet's own sandbox recipe. The host's Jobs engine
//! (`src-tauri/src/jobs.rs`) runs what this module prepares.

pub mod agent;
pub mod playbook;
pub mod runner;
pub mod sandbox;
pub mod stack;
pub mod wizard;
