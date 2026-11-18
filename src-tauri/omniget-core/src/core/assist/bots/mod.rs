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
pub use manifest::CapabilityReport;
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

pub fn toolset() -> Arc<dyn AssistToolset> {
    Arc::new(EmptyToolset("bots"))
}

pub fn augment() -> Arc<dyn crate::core::llm::coordinator::TurnAugment> {
    Arc::new(BotsAugment::new(BotEnv::global(), skills::projection()))
}

#[derive(Clone, Default)]
pub struct BotEnv {
    db: Option<Arc<AssistDb>>,
    skills_root: Option<PathBuf>,
    broker: Option<Arc<ToolBroker>>,
}

static BROKER: RwLock<Option<Arc<ToolBroker>>> = RwLock::new(None);

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
