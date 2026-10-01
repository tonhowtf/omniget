use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::intent::{Animation, Intent, Mood};

pub const DEFAULT_ENERGY: u8 = 100;

pub const EXHAUSTED_ENERGY: u8 = 0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OmniState {
    pub current: Intent,
    pub since_ms: u64,
    pub energy: u8,
}

impl Default for OmniState {
    fn default() -> Self {
        OmniState {
            current: Intent::neutral(),
            since_ms: now_ms(),
            energy: DEFAULT_ENERGY,
        }
    }
}

impl OmniState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, intent: &Intent) -> bool {
        if &self.current == intent {
            return false;
        }
        self.current = intent.clone();
        self.since_ms = now_ms();
        true
    }

    pub fn is_exhausted(&self) -> bool {
        self.energy == EXHAUSTED_ENERGY
    }

    pub fn animation(&self) -> Animation {
        self.current.animation
    }

    pub fn mood(&self) -> Mood {
        self.current.mood
    }

    pub fn elapsed_ms(&self) -> u64 {
        now_ms().saturating_sub(self.since_ms)
    }

    pub fn path() -> Option<PathBuf> {
        crate::core::paths::app_data_dir().map(|d| d.join("omni").join("state.json"))
    }

    pub fn load() -> Self {
        Self::load_from(Self::path())
    }

    fn load_from(path: Option<PathBuf>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str::<OmniState>(&raw) {
            Ok(mut state) => {
                state.energy = state.energy.min(100);
                state
            }
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or_else(|| "ERR_OMNI_NO_DATA_DIR".to_string())?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("ERR_OMNI_STATE_DIR: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("ERR_OMNI_STATE_ENCODE: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("ERR_OMNI_STATE_WRITE: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("ERR_OMNI_STATE_RENAME: {e}"))?;
        Ok(())
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
