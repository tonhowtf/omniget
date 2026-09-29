//! Watch progress rules (when a play counts, when it is watched, where to
//! resume) and a per-second bitmap of what was actually seen.

use serde::{Deserialize, Serialize};

/// Thresholds for progress decisions. All times in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    /// A play enters history after `min(record_after_ms, record_ratio × dur)`.
    pub record_after_ms: u64,
    pub record_ratio: f64,
    /// Watched at `min(watched_ratio × dur, dur − watched_tail_ms)`.
    pub watched_ratio: f64,
    pub watched_tail_ms: u64,
    /// The tail rule never lowers the watched mark below this share of the
    /// duration (keeps a 2-minute clip from counting as watched at 20 s).
    pub watched_floor_ratio: f64,
    /// No resume point for media shorter than this.
    pub resume_min_duration_ms: u64,
    /// Resume this much before the saved position.
    pub resume_rewind_ms: u64,
    /// A lesson is complete once this share of its seconds was seen.
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
    /// True once the play is long enough to enter history. With an unknown
    /// duration (`0`, live) only the absolute threshold applies.
    pub fn should_record(&self, pos_ms: u64, dur_ms: u64) -> bool {
        let threshold = if dur_ms == 0 {
            self.record_after_ms
        } else {
            self.record_after_ms
                .min((dur_ms as f64 * self.record_ratio) as u64)
        };
        pos_ms >= threshold
    }

    /// The position that marks the media as watched, or `None` without a
    /// duration.
    pub fn watched_mark(&self, dur_ms: u64) -> Option<u64> {
        if dur_ms == 0 {
            return None;
        }
        let ratio = (dur_ms as f64 * self.watched_ratio) as u64;
        let floor = (dur_ms as f64 * self.watched_floor_ratio) as u64;
        let tail = dur_ms.saturating_sub(self.watched_tail_ms).max(floor);
        Some(ratio.min(tail))
    }

    /// True when `pos_ms` is past the watched mark (needs `dur_ms > 0`).
    pub fn is_watched(&self, pos_ms: u64, dur_ms: u64) -> bool {
        self.watched_mark(dur_ms).is_some_and(|m| pos_ms >= m)
    }

    /// Where to resume: `None` for short media, finished media or plays too
    /// short to record; otherwise `pos − rewind` (at least 0).
    pub fn resume_point(&self, pos_ms: u64, dur_ms: u64) -> Option<u64> {
        if dur_ms < self.resume_min_duration_ms
            || self.is_watched(pos_ms, dur_ms)
            || !self.should_record(pos_ms, dur_ms)
        {
            return None;
        }
        Some(pos_ms.saturating_sub(self.resume_rewind_ms))
    }

    /// True when the seen ratio completes a lesson.
    pub fn lesson_complete(&self, ratio: f64) -> bool {
        ratio >= self.lesson_complete_ratio
    }
}

/// [`Rules::should_record`] with the default rules.
pub fn should_record(pos_ms: u64, dur_ms: u64) -> bool {
    Rules::default().should_record(pos_ms, dur_ms)
}

/// [`Rules::is_watched`] with the default rules.
pub fn is_watched(pos_ms: u64, dur_ms: u64) -> bool {
    Rules::default().is_watched(pos_ms, dur_ms)
}

/// [`Rules::resume_point`] with the default rules.
pub fn resume_point(pos_ms: u64, dur_ms: u64) -> Option<u64> {
    Rules::default().resume_point(pos_ms, dur_ms)
}

/// [`Rules::lesson_complete`] with the default rules.
pub fn lesson_complete(ratio: f64) -> bool {
    Rules::default().lesson_complete(ratio)
}

/// Highest second tracked (about 55 h); later seconds are ignored so a bogus
/// duration cannot allocate much.
pub const SEEN_MAX_SECONDS: u64 = 200_000;

/// One bit per second of media, set when that second was played. Serializes
/// as the raw byte vector (little-endian bit order inside each byte).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeenBits {
    bits: Vec<u8>,
}

impl SeenBits {
    pub fn new() -> Self {
        Self::default()
    }

    /// Wraps bytes loaded from storage (extra bytes past the cap are dropped).
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

    /// Marks seconds `[from_s, to_s)`. Reversed or empty ranges do nothing.
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

    /// Marks the whole seconds between two positions in milliseconds
    /// (`10_000..15_000` marks seconds 10 to 14).
    pub fn mark_ms(&mut self, from_ms: u64, to_ms: u64) {
        self.mark(from_ms / 1000, to_ms / 1000);
    }

    pub fn is_seen(&self, s: u64) -> bool {
        self.bits
            .get((s / 8) as usize)
            .is_some_and(|b| b & (1 << (s % 8)) != 0)
    }

    /// Number of seconds marked.
    pub fn count(&self) -> u64 {
        self.bits.iter().map(|b| b.count_ones() as u64).sum()
    }

    /// Share of `[0, dur_s)` seen, in `0..=1` (`0` when `dur_s` is 0).
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

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    #[test]
    fn record_threshold() {
        // Long video: 15 s wins.
        assert!(!should_record(14_999, 60 * MIN));
        assert!(should_record(15_000, 60 * MIN));
        // Short video: 5 % wins (100 s -> 5 s).
        assert!(!should_record(4_999, 100_000));
        assert!(should_record(5_000, 100_000));
        // Unknown duration.
        assert!(!should_record(14_000, 0));
        assert!(should_record(15_000, 0));
    }

    #[test]
    fn watched_threshold() {
        // 60 min: min(54 min, 58:20) = 54 min.
        assert!(!is_watched(54 * MIN - 1, 60 * MIN));
        assert!(is_watched(54 * MIN, 60 * MIN));
        // 10 min: min(9 min, 8:20) = 8:20.
        assert!(!is_watched(499_999, 10 * MIN));
        assert!(is_watched(500_000, 10 * MIN));
        // 120 s: the tail would give 20 s, the 80 % floor keeps it at 96 s.
        assert!(!is_watched(95_999, 120_000));
        assert!(is_watched(96_000, 120_000));
        // 30 s: tail saturates, floor 24 s, 90 % = 27 s -> 24 s.
        assert!(!is_watched(23_999, 30_000));
        assert!(is_watched(24_000, 30_000));
        // No duration never counts.
        assert!(!is_watched(10 * MIN, 0));
    }

    #[test]
    fn resume_rules() {
        assert_eq!(resume_point(60_000, 5 * MIN - 1), None);
        assert_eq!(resume_point(60_000, 5 * MIN), Some(55_000));
        assert_eq!(resume_point(10_000, 60 * MIN), None);
        assert_eq!(resume_point(15_000, 60 * MIN), Some(10_000));
        assert_eq!(resume_point(54 * MIN, 60 * MIN), None);
        assert_eq!(resume_point(54 * MIN - 1, 60 * MIN), Some(54 * MIN - 5_001));
    }

    #[test]
    fn seen_bits() {
        let mut s = SeenBits::new();
        s.mark(0, 10);
        s.mark(5, 20);
        s.mark(30, 25); // reversed: ignored
        assert_eq!(s.count(), 20);
        assert!(s.is_seen(19) && !s.is_seen(20));
        assert!((s.ratio(40) - 0.5).abs() < 1e-9);
        assert_eq!(s.ratio(0), 0.0);
        assert!((s.ratio(10) - 1.0).abs() < 1e-9);
        s.mark_ms(20_000, 34_999);
        assert_eq!(s.count(), 34);
        assert!(lesson_complete(s.ratio(40)));
        assert!(!lesson_complete(0.849));
        assert!(lesson_complete(0.85));

        let bytes = s.clone().into_bytes();
        assert_eq!(SeenBits::from_bytes(bytes.clone()), s);
        let json = serde_json::to_string(&s).unwrap();
        let back: SeenBits = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_bytes(), bytes.as_slice());

        let mut big = SeenBits::new();
        big.mark(0, u64::MAX);
        assert_eq!(big.count(), SEEN_MAX_SECONDS);
        assert!(SeenBits::from_bytes(vec![0xff; 1_000_000]).as_bytes().len() <= 25_000);
    }
}
