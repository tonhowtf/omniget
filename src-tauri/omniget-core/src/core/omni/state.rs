use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::intent::Intent;

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
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
