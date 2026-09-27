//! One cleanup rule, whatever its source (BleachBit CleanerML, Kudu, Mole,
//! PureMac, ours). Every source is converted to this shape; the engine in
//! `clean/mod.rs` only knows this shape.

use serde::{Deserialize, Serialize};

use super::super::{Destination, Os, Risk};

/// What a rule does with each target it finds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Remove the matched files or folders.
    Delete,
    /// Empty the matched folder but keep it (apps expect the root).
    DeleteContents,
    /// Truncate a log file to zero bytes (the app keeps its handle).
    Truncate,
    /// `VACUUM` a SQLite database (browser history, caches), after a WAL
    /// checkpoint; never delete `-wal`/`-shm` next to an open database.
    SqliteVacuum,
    /// A native cleanup command (argv only; program in `plan::NATIVE_PROGRAMS`).
    /// `measure` are folders measured before and after for the "freed" number.
    Command {
        program: String,
        args: Vec<String>,
        #[serde(default)]
        measure: Vec<String>,
    },
}

/// Something that must be true for the rule to apply at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Requirement {
    /// A path (with variables) exists: the app is installed.
    PathExists {
        path: String,
    },
    /// A program is on PATH.
    Program {
        name: String,
    },
    FullDiskAccess,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Stable id: `<source>.<cleaner>.<option>` (`bleachbit.firefox.cache`).
    pub id: String,
    pub os: Vec<Os>,
    /// Screen grouping: `system`, `browser`, `dev`, `apps`, `logs`, `trash`, `games`, `package_cache`...
    pub category: String,
    /// i18n key when we have one, else the English label from the source.
    pub label_key: Option<String>,
    pub label: String,
    #[serde(default)]
    pub description: String,
    /// Globs with variables (`~`, `$HOME`, `$XDG_CACHE_HOME`, `%LOCALAPPDATA%`).
    #[serde(default)]
    pub targets: Vec<String>,
    /// Only match names like this inside a target folder (`*.log`); empty = everything.
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Process names that must not be running (a browser while its cache goes).
    #[serde(default)]
    pub running: Vec<String>,
    /// Skip files touched in the last N days (Kudu: never delete a cache used yesterday).
    #[serde(default)]
    pub min_age_days: u32,
    pub action: Action,
    pub destination: Destination,
    pub risk: Risk,
    #[serde(default)]
    pub requires: Vec<Requirement>,
    /// Upstream project the rule came from, and its license.
    pub source: String,
    pub license: String,
}
