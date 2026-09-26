//! LLM execution stack built without SDKs (plan §2.1 lines 8–14, §2.2).
//! Module owners: `types`/`error`/`sse`/`providers` → f2-llm-providers;
//! `coordinator`/`router`/`broker`/`budget`/`agent`/`templates`/`runtime` →
//! f2-llm-coordinator; `wire_probe` → f2-wire-probe; `local_servers`/`roster_store`
//! → f2-llm-commands. Declarations here are the orchestrator's; bodies are stubs.

pub mod acp;
pub mod agent;
pub mod brain;
pub mod broker;
pub mod budget;
pub mod caps;
pub mod cli_runtime;
pub mod cli_usage;
pub mod code_tools;
pub mod compress;
pub mod coordinator;
pub mod error;
pub mod kb;
pub mod local_servers;
pub mod perm;
pub mod providers;
pub mod prune;
pub mod roster_store;
pub mod router;
pub mod runtime;
pub mod snapshot;
pub mod sse;
pub mod tool_table;
pub mod types;
pub mod wire_probe;
