//! 3D city: local store, founding by LLM, roles and the `city_*` MCP tools
//! (estudo 99, `GameChangeLLM/sessoes-3d/M3-URBANISTA-LLM.md`).
//!
//! - [`store`]: `<app_data>/city3d/<city_id>/` (charter, one program per
//!   region, revision history, `log.jsonl`) and the solve/mesh cache. One
//!   active city at a time.
//! - [`server`]: the same operations over the instance's REST
//!   (`/api/world/city3d/*`) when the app is connected to a server city.
//! - [`api`]: what the Tauri commands and the MCP tools call; picks local or
//!   server and never leaves a region half-written.
//! - [`founding`]: the charter + first program loop with a real LLM.
//! - [`roles`]: mayor, council and resident prompts, token budgets, the
//!   council round and the construction tick.
//! - [`tools`]: the `city_*` entries of the tool table.
//! - [`docs`]: the program format and the options an LLM may pick from.
//!
//! The LLM never writes geometry: it writes intent, the solver executes and
//! the checker decides.

pub mod api;
pub mod docs;
pub mod founding;
pub mod llm;
pub mod roles;
pub mod server;
pub mod store;
pub mod tools;

/// Event the app emits after a region got a new revision.
pub const EVENT_CHANGED: &str = "city3d:changed";
/// Event the app emits while a founding job runs.
pub const EVENT_FOUNDING: &str = "city3d:founding";
/// Error prefix of every failure of this module.
pub const ERR: &str = "ERR_CITY3D";
