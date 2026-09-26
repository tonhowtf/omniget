//! Progress guard: tells legitimate polling from a loop that goes nowhere.
//!
//! Each round of a task leaves an observation: the state it produced (the
//! artifact digests and the criteria statuses), the tool calls it made and,
//! when the task is waiting on something outside (a download, a build, a
//! page that updates), the external status it saw. Hashing the tool calls
//! alone is not enough — polling repeats the same call on purpose — so the
//! guard looks at whether anything changed:
//! - polling is tolerated with exponential backoff while the external status
//!   moves, and up to `max_polls` rounds while it does not;
//! - `max_same` consecutive rounds with the same state and the same calls is
//!   a loop without progress: stop and say so (A09).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProgressPolicy {
    pub max_same: u32,
    pub max_rounds: u32,
    pub poll_base_ms: u64,
    pub poll_max_ms: u64,
    pub max_polls: u32,
}

impl Default for ProgressPolicy {
    fn default() -> Self {
        Self {
            max_same: 3,
            max_rounds: 6,
            poll_base_ms: 2_000,
            poll_max_ms: 60_000,
            max_polls: 12,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoundObservation {
    pub round: u32,
    /// Digest of what the round left behind (artifacts + criteria statuses).
    pub state: String,
    /// Tool calls of the round, in order (`name` or `name:argsdigest`).
    pub calls: Vec<String>,
    /// Set when the round waits on something outside.
    pub polling: bool,
    pub external_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Continue { delay_ms: u64 },
    Stop { code: String, reason: String },
}

pub fn decide(history: &[RoundObservation], p: &ProgressPolicy) -> Decision {
    let Some(last) = history.last() else {
        return Decision::Continue { delay_ms: 0 };
    };
    if last.round >= p.max_rounds {
        return Decision::Stop {
            code: super::ERR_MISSION_CRITERIA.into(),
            reason: format!("{} rounds used (limit {})", last.round, p.max_rounds),
        };
    }
    if last.polling {
        // Consecutive polling rounds whose external status did not move.
        let mut still = 0u32;
        let mut polls = 0u32;
        for w in history.iter().rev() {
            if !w.polling {
                break;
            }
            polls += 1;
            if w.external_status == last.external_status {
                still += 1;
            } else {
                break;
            }
        }
        if still > p.max_polls {
            return Decision::Stop {
                code: super::ERR_MISSION_STAGNANT.into(),
                reason: format!(
                    "polled {still} times and the status stayed {:?}",
                    last.external_status.as_deref().unwrap_or("unknown")
                ),
            };
        }
        let exp = still.saturating_sub(1).min(16);
        let delay = p
            .poll_base_ms
            .saturating_mul(1u64 << exp)
            .min(p.poll_max_ms);
        let _ = polls;
        return Decision::Continue { delay_ms: delay };
    }
    let same = history
        .iter()
        .rev()
        .take_while(|o| !o.polling && o.state == last.state && o.calls == last.calls)
        .count() as u32;
    if same >= p.max_same {
        return Decision::Stop {
            code: super::ERR_MISSION_STAGNANT.into(),
            reason: format!(
                "{same} rounds in a row made the same calls ({}) and changed nothing",
                if last.calls.is_empty() {
                    "none".to_string()
                } else {
                    last.calls.join(", ")
                }
            ),
        };
    }
    Decision::Continue { delay_ms: 0 }
}

/// Stable digest of a round's state from the verdict and the artifact
/// digests (used as [`RoundObservation::state`]).
pub fn state_digest(parts: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0]);
    }
    hex::encode(&h.finalize()[..12])
}
