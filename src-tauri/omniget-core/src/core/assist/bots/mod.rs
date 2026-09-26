//! Bot identity and effective capability (spec 02, "Bot pessoal" and
//! "Capacidade efetiva, não configuração aparente"). Owner: worker W3.
//!
//! A bot is the roster's `AgentDef` (id, name, model, runtime) plus a profile
//! kept here, keyed by the same id: purpose, instructions, the capabilities
//! the user picked in words they understand (personal memory, web search,
//! reading, project code, delegate), the memory policy and the default
//! connection. Changing the model or the connection never changes the id, so
//! memory and skill bindings stay with the bot (B07).
//!
//! Skills go through one pipeline ([`skills`]):
//! catalogue/installed folder → install record with a sha256 of the folder →
//! a versioned binding per bot (read and run-scripts are separate grants) →
//! one broker tool per installed version (`skill__<name>_<hash8>`, source
//! [`skills::SKILL_SOURCE`], re-registered whenever a skill is installed,
//! updated or removed) → a compact index in the turn's system message, only
//! for the skills bound and usable this turn → the tool opens the body or one
//! reference file (no traversal, no symlink) → every read is traced per run
//! with the hash and the file ([`skills::skill_reads`]).
//!
//! [`manifest`] joins all of it into what the UI shows: requested /
//! available / missing, with the reason and the fix. "A box ticked" is never
//! reported as "a tool available".

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use super::db::{AssistDb, Migration};
use super::tools::{AssistToolset, EmptyToolset};
use crate::core::llm::broker::ToolBroker;

pub mod augment;
pub mod manifest;
pub mod profile;
pub mod skills;

#[cfg(test)]
mod tests;

pub use augment::BotsAugment;
pub use manifest::{report, CapabilityReport};
pub use profile::{BotProfile, Capability, MemoryPolicy};
pub use skills::{BindingState, BindingStatus, SkillProjection, SKILL_SOURCE};

pub const ERR_BOT: &str = "ERR_BOT";

pub const MIGRATIONS: &[Migration] = &[Migration {
    module: "bots",
    version: 1,
    sql: r#"
CREATE TABLE bots_profiles(
  bot_id TEXT PRIMARY KEY,
  purpose TEXT NOT NULL DEFAULT '',
  instructions TEXT NOT NULL DEFAULT '',
  capabilities TEXT NOT NULL DEFAULT '[]',
  memory_policy TEXT NOT NULL DEFAULT 'remember',
  default_connection TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE bots_skill_installs(
  skill TEXT PRIMARY KEY,
  hash TEXT NOT NULL,
  version TEXT,
  source TEXT NOT NULL DEFAULT '{}',
  recorded_at INTEGER NOT NULL
);
CREATE TABLE bots_skill_bindings(
  bot_id TEXT NOT NULL,
  skill TEXT NOT NULL,
  hash TEXT NOT NULL,
  allow_read INTEGER NOT NULL DEFAULT 1,
  allow_scripts INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(bot_id, skill)
);
CREATE INDEX bots_skill_bindings_skill ON bots_skill_bindings(skill);
CREATE TABLE bots_skill_reads(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id TEXT,
  bot_id TEXT NOT NULL,
  conversation_id TEXT,
  skill TEXT NOT NULL,
  hash TEXT NOT NULL,
  file TEXT NOT NULL,
  bytes INTEGER NOT NULL DEFAULT 0,
  ok INTEGER NOT NULL,
  error TEXT,
  at INTEGER NOT NULL
);
CREATE INDEX bots_skill_reads_run ON bots_skill_reads(run_id);
CREATE INDEX bots_skill_reads_bot ON bots_skill_reads(bot_id, at);
"#,
}];

/// Bots offer no model-callable tools through the assistant registry; skill
/// tools are their own broker source ([`skills::SKILL_SOURCE`]).
pub fn toolset() -> Arc<dyn AssistToolset> {
    Arc::new(EmptyToolset("bots"))
}

/// Per-turn hook (see `llm::coordinator::TurnAugment`): capability grants,
/// skill grants and the skill index.
pub fn augment() -> Arc<dyn crate::core::llm::coordinator::TurnAugment> {
    Arc::new(BotsAugment::new(BotEnv::global(), skills::projection()))
}

/// Where the bot layer reads and writes. Every field falls back to the
/// process-wide instance, so production code uses [`BotEnv::global`] and a
/// test hands its own database, skills folder and broker.
#[derive(Clone, Default)]
pub struct BotEnv {
    db: Option<Arc<AssistDb>>,
    skills_root: Option<PathBuf>,
    broker: Option<Arc<ToolBroker>>,
}

static BROKER: RwLock<Option<Arc<ToolBroker>>> = RwLock::new(None);

/// The app's broker, installed at boot so the bot layer can tell a granted
/// tool from a registered one.
pub fn set_broker(broker: Arc<ToolBroker>) {
    *BROKER.write().unwrap_or_else(|e| e.into_inner()) = Some(broker);
}

impl BotEnv {
    pub fn global() -> Self {
        Self::default()
    }

    pub fn new(db: Arc<AssistDb>, skills_root: PathBuf, broker: Option<Arc<ToolBroker>>) -> Self {
        Self {
            db: Some(db),
            skills_root: Some(skills_root),
            broker,
        }
    }

    pub fn db(&self) -> Result<Arc<AssistDb>, String> {
        match &self.db {
            Some(db) => Ok(db.clone()),
            None => super::db::global(),
        }
    }

    pub fn skills_root(&self) -> Result<PathBuf, String> {
        match &self.skills_root {
            Some(p) => Ok(p.clone()),
            None => crate::core::skills::skills_dir().map_err(|e| e.to_string()),
        }
    }

    pub fn broker(&self) -> Option<Arc<ToolBroker>> {
        self.broker
            .clone()
            .or_else(|| BROKER.read().unwrap_or_else(|e| e.into_inner()).clone())
    }
}
