//! `usage-tray.json`: what the user chose about the menu bar usage icon. Lives
//! next to `limits-strip.json` (`<data>/llm/limits/`), written through a temp
//! file and a rename. Switches only: no secret is ever written here.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const PREFS_FILE: &str = "usage-tray.json";

/// What the ring on the icon measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Shows {
    /// The 5 h window of the busiest shown account.
    #[default]
    Session,
    /// The weekly window of the busiest shown account.
    Weekly,
    /// Whichever window of whichever shown account is fullest.
    Busiest,
}

impl Shows {
    pub fn as_str(self) -> &'static str {
        match self {
            Shows::Session => "session",
            Shows::Weekly => "weekly",
            Shows::Busiest => "busiest",
        }
    }

    pub fn parse(raw: &str) -> Option<Shows> {
        match raw {
            "session" => Some(Shows::Session),
            "weekly" => Some(Shows::Weekly),
            "busiest" => Some(Shows::Busiest),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrayPrefs {
    /// The icon is in the menu bar. On by default on macOS only, where a
    /// status item is the native home of a usage meter.
    pub enabled: bool,
    /// Ring ids (`claude`, `claude:<account>`, `codex:<account>`…) the icon
    /// reads and the panel lists, in this order. Empty = nothing is read:
    /// the panel asks which accounts to show.
    pub accounts: Vec<String>,
    /// Set once the list was seeded from the strip's switches, so emptying
    /// it by hand is respected.
    pub seeded: bool,
    pub shows: Shows,
    /// "37%" next to the ring (macOS status item title).
    pub percent_text: bool,
    /// Loop in his colours instead of the monochrome template glyph.
    pub colored: bool,
    /// The ring turns orange at `warn` and red at `critical` (percent used).
    pub warn: u8,
    pub critical: u8,
}

impl Default for TrayPrefs {
    fn default() -> Self {
        Self {
            enabled: cfg!(target_os = "macos"),
            accounts: Vec::new(),
            seeded: false,
            shows: Shows::Session,
            percent_text: true,
            colored: false,
            warn: 80,
            critical: 95,
        }
    }
}

impl TrayPrefs {
    /// Thresholds kept in 1..=100 with `warn < critical`.
    pub fn sane(mut self) -> Self {
        self.warn = self.warn.clamp(1, 99);
        self.critical = self.critical.clamp(self.warn + 1, 100);
        let mut seen = std::collections::HashSet::new();
        self.accounts
            .retain(|id| !id.is_empty() && seen.insert(id.clone()));
        self
    }

    /// First run: the accounts of the Claude and Codex rings the strip already
    /// reads, so a user who switched those on sees them here too.
    pub fn seed_from(&mut self, strip: &crate::limits_strip::prefs::StripPrefs) -> bool {
        if self.seeded {
            return false;
        }
        self.seeded = true;
        if self.accounts.is_empty() {
            self.accounts = strip
                .providers
                .iter()
                .filter(|p| p.enabled && is_cli_ring(&p.id))
                .map(|p| p.id.clone())
                .collect();
        }
        true
    }

    pub fn toggle_account(&mut self, id: &str) {
        if let Some(i) = self.accounts.iter().position(|a| a == id) {
            self.accounts.remove(i);
        } else {
            self.accounts.push(id.to_string());
        }
    }
}

/// The rings that belong to a CLI with logins: Claude Code and Codex.
pub fn is_cli_ring(id: &str) -> bool {
    let base = id.split(':').next().unwrap_or(id);
    base == "claude" || base == "codex"
}

pub fn from_str(raw: &str) -> TrayPrefs {
    serde_json::from_str::<TrayPrefs>(raw)
        .unwrap_or_default()
        .sane()
}

pub fn load() -> TrayPrefs {
    let Some(dir) = crate::limits_strip::prefs::dir() else {
        return TrayPrefs::default();
    };
    match std::fs::read_to_string(dir.join(PREFS_FILE)) {
        Ok(raw) => from_str(&raw),
        Err(_) => TrayPrefs::default(),
    }
}

pub fn save(prefs: &TrayPrefs) -> Result<(), String> {
    let dir = crate::limits_strip::prefs::dir().ok_or("ERR_USAGE_TRAY_STORE: no data directory")?;
    save_in(&dir, prefs)
}

pub fn save_in(dir: &Path, prefs: &TrayPrefs) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("ERR_USAGE_TRAY_STORE: {e}"))?;
    let body =
        serde_json::to_vec_pretty(prefs).map_err(|e| format!("ERR_USAGE_TRAY_STORE: {e}"))?;
    let tmp = dir.join(format!("{PREFS_FILE}.tmp"));
    std::fs::write(&tmp, &body).map_err(|e| format!("ERR_USAGE_TRAY_STORE: {e}"))?;
    std::fs::rename(&tmp, dir.join(PREFS_FILE)).map_err(|e| format!("ERR_USAGE_TRAY_STORE: {e}"))
}
