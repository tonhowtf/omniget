use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    pub record_after_ms: u64,
    pub record_ratio: f64,
    pub watched_ratio: f64,
    pub watched_tail_ms: u64,
    pub watched_floor_ratio: f64,
    pub resume_min_duration_ms: u64,
    pub resume_rewind_ms: u64,
    pub lesson_complete_ratio: f64,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            record_after_ms: 15_000,
            record_ratio: 0.05,
            watched_ratio: 0.9,
            watched_tail_ms: 100_000,
            watched_floor_ratio: 0.8,
            resume_min_duration_ms: 5 * 60_000,
            resume_rewind_ms: 5_000,
            lesson_complete_ratio: 0.85,
        }
    }
}

impl Rules {
    pub fn should_record(&self, pos_ms: u64, dur_ms: u64) -> bool {
        let threshold = if dur_ms == 0 {
            self.record_after_ms
        } else {
            self.record_after_ms
                .min((dur_ms as f64 * self.record_ratio) as u64)
        };
        pos_ms >= threshold
    }

    pub fn watched_mark(&self, dur_ms: u64) -> Option<u64> {
        if dur_ms == 0 {
            return None;
        }
        let ratio = (dur_ms as f64 * self.watched_ratio) as u64;
        let floor = (dur_ms as f64 * self.watched_floor_ratio) as u64;
        let tail = dur_ms.saturating_sub(self.watched_tail_ms).max(floor);
        Some(ratio.min(tail))
    }

    pub fn is_watched(&self, pos_ms: u64, dur_ms: u64) -> bool {
        self.watched_mark(dur_ms).is_some_and(|m| pos_ms >= m)
    }

    pub fn resume_point(&self, pos_ms: u64, dur_ms: u64) -> Option<u64> {
        if dur_ms < self.resume_min_duration_ms
            || self.is_watched(pos_ms, dur_ms)
            || !self.should_record(pos_ms, dur_ms)
        {
            return None;
        }
        Some(pos_ms.saturating_sub(self.resume_rewind_ms))
    }

    pub fn lesson_complete(&self, ratio: f64) -> bool {
        ratio >= self.lesson_complete_ratio
    }
}

pub fn should_record(pos_ms: u64, dur_ms: u64) -> bool {
    Rules::default().should_record(pos_ms, dur_ms)
}

pub fn is_watched(pos_ms: u64, dur_ms: u64) -> bool {
    Rules::default().is_watched(pos_ms, dur_ms)
}

pub fn resume_point(pos_ms: u64, dur_ms: u64) -> Option<u64> {
    Rules::default().resume_point(pos_ms, dur_ms)
}

pub fn lesson_complete(ratio: f64) -> bool {
    Rules::default().lesson_complete(ratio)
}

pub const SEEN_MAX_SECONDS: u64 = 200_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeenBits {
    bits: Vec<u8>,
}

impl SeenBits {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_bytes(mut bytes: Vec<u8>) -> Self {
        bytes.truncate((SEEN_MAX_SECONDS as usize).div_ceil(8));
        SeenBits { bits: bytes }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bits
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bits
    }

    pub fn mark(&mut self, from_s: u64, to_s: u64) {
        let to_s = to_s.min(SEEN_MAX_SECONDS);
        if to_s <= from_s {
            return;
        }
        let need = (to_s as usize).div_ceil(8);
        if self.bits.len() < need {
            self.bits.resize(need, 0);
        }
        for s in from_s..to_s {
            self.bits[(s / 8) as usize] |= 1 << (s % 8);
        }
    }

    pub fn is_seen(&self, s: u64) -> bool {
        self.bits
            .get((s / 8) as usize)
            .is_some_and(|b| b & (1 << (s % 8)) != 0)
    }

    pub fn count(&self) -> u64 {
        self.bits.iter().map(|b| b.count_ones() as u64).sum()
    }

    pub fn ratio(&self, dur_s: u64) -> f64 {
        if dur_s == 0 {
            return 0.0;
        }
        let dur = dur_s.min(SEEN_MAX_SECONDS);
        let full = (dur / 8) as usize;
        let mut n: u64 = self
            .bits
            .iter()
            .take(full)
            .map(|b| b.count_ones() as u64)
            .sum();
        for s in (full as u64 * 8)..dur {
            if self.is_seen(s) {
                n += 1;
            }
        }
        (n as f64 / dur as f64).min(1.0)
    }
}
