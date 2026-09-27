//! washmybelly: cleanup, uninstall, startup, privacy, power and FPS for the
//! machine OmniGet runs on. Plan: `docs/washmybelly-plan.md` (not in git).
//!
//! Every destructive action goes scan → plan → approval → execute → receipt:
//!
//! - a scan returns items and never changes anything;
//! - [`plan`] turns chosen items into a plan with a 10 minute life, keeping
//!   the device, inode, size and mtime of each file it will touch;
//! - [`plan::execute`] checks each item again, skips what changed, and writes
//!   a [`receipt`] that says what left and how to bring it back.
//!
//! OS code sits behind `#[cfg(target_os)]`; every text parser is a pure
//! function compiled on every OS in tests (`#[cfg(any(target_os = "..", test))]`)
//! so each CI checks the parsers of the other two.

pub mod approvals;
pub mod apps;
pub mod caps;
pub mod clean;
pub mod disk;
pub mod elevate;
pub mod fsops;
pub mod game;
pub mod hosts;
pub mod maintenance;
pub mod monitor;
pub mod packages;
pub mod parse;
pub mod perf;
pub mod plan;
pub mod power;
pub mod privacy;
pub mod protect;
pub mod quarantine;
pub mod receipt;
pub mod security;
pub mod startup;
pub mod store;
pub mod tools;
pub mod tweaks;
pub mod ui;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

/// The three systems washmybelly knows. Catalogs carry all three in every
/// binary and are filtered at run time, so an agent can say "Windows only".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Macos,
    Windows,
    Linux,
}

impl Os {
    pub fn as_str(self) -> &'static str {
        match self {
            Os::Macos => "macos",
            Os::Windows => "windows",
            Os::Linux => "linux",
        }
    }

    pub fn parse(s: &str) -> Option<Os> {
        match s {
            "macos" | "mac" | "darwin" => Some(Os::Macos),
            "windows" | "win" => Some(Os::Windows),
            "linux" => Some(Os::Linux),
            _ => None,
        }
    }
}

/// The OS this binary was built for.
pub const fn current_os() -> Os {
    if cfg!(target_os = "macos") {
        Os::Macos
    } else if cfg!(windows) {
        Os::Windows
    } else {
        Os::Linux
    }
}

/// How much an item or a tool can hurt. The UI shows it, the MCP respects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    /// Only reads.
    Read,
    /// Changes something the receipt can undo (quarantine, Trash, snapshot).
    Reversible,
    /// Cannot be undone (a regenerable cache unlinked, a package removed).
    Destructive,
    /// Only from the screen, with its own warning; never in an agent plan.
    Extreme,
}

/// Where a removed file goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Destination {
    /// Regenerable cache: unlinked.
    Unlink,
    /// Medium risk: moved to our own quarantine on the same volume.
    Quarantine,
    /// Something the person recognises (apps, documents): the system Trash.
    Trash,
}

/// One progress tick, emitted to the app as `washmybelly-progress`.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    /// What is running: `junk_scan`, `execute`, `disk_usage`...
    pub task: String,
    pub stage: String,
    pub done: u64,
    pub total: u64,
    pub message: String,
}

type Sink = Arc<dyn Fn(Progress) + Send + Sync>;

static SINK: RwLock<Option<Sink>> = RwLock::new(None);

/// The app installs the sink that turns progress into a Tauri event.
pub fn set_progress_sink(sink: Sink) {
    if let Ok(mut s) = SINK.write() {
        *s = Some(sink);
    }
}

/// Report progress; a no-op without a sink (CLI, tests).
pub fn progress(task: &str, stage: &str, done: u64, total: u64, message: impl Into<String>) {
    let sink = SINK.read().ok().and_then(|s| s.clone());
    if let Some(sink) = sink {
        sink(Progress {
            task: task.to_string(),
            stage: stage.to_string(),
            done,
            total,
            message: message.into(),
        });
    }
}

/// Settings the core needs, pushed by the app (the core cannot read the
/// app's settings file on its own).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreSettings {
    pub enabled: bool,
    pub quarantine_days: u32,
    pub protected_paths_extra: Vec<String>,
    pub perf_tool_path: String,
}

impl Default for CoreSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            quarantine_days: 7,
            protected_paths_extra: Vec::new(),
            perf_tool_path: String::new(),
        }
    }
}

static SETTINGS: RwLock<Option<CoreSettings>> = RwLock::new(None);

pub fn set_settings(s: CoreSettings) {
    crate::core::llm::tool_table::set_feature_enabled("washmybelly", s.enabled);
    if let Ok(mut cur) = SETTINGS.write() {
        *cur = Some(s);
    }
}

pub fn settings() -> CoreSettings {
    SETTINGS
        .read()
        .ok()
        .and_then(|s| s.clone())
        .unwrap_or_default()
}

/// `<app_data>/washmybelly`, created on first use. Tests and the CLI can
/// point it elsewhere with `OMNIGET_DATA_DIR`.
pub fn data_dir() -> Result<PathBuf, String> {
    let base = crate::core::paths::app_data_dir().ok_or("no app data dir")?;
    let dir = base.join("washmybelly");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Link to the source of the embedded third-party data whose license asks
/// for it (privacy.sexy is AGPL-3.0 and the MCP can serve its output).
pub const SOURCE_URL: &str = "https://github.com/tonhowtf/omniget";
pub const PRIVACY_SEXY_SOURCE_URL: &str = "https://github.com/undergroundwires/privacy.sexy";
