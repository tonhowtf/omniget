//! The judge seam. A judge answers two questions per `tool_use_id` — does the
//! call still matter, does its full output still matter — exactly the two
//! `noul` questions `tamaratran/fast-jev-compaction` asks
//! (`src/compact.ts` `questionsFor`).
//!
//! Fail-open is the contract, not an implementation detail: a judge that
//! errors, times out or answers only half the batch simply returns fewer
//! entries, and everything it did not answer is kept. No judge can ever cause
//! an omission by failing.

use std::collections::HashMap;

use async_trait::async_trait;

use super::policy::{Candidate, GoalContext, KeepVerdict};

/// Verdicts by `tool_use_id`. A missing id means keep.
pub type Verdicts = HashMap<String, KeepVerdict>;

#[async_trait]
pub trait ContextJudge: Send + Sync {
    /// Which judge this is, recorded in the sticky store.
    fn name(&self) -> &'static str;

    /// Judge the candidates against the goal. Implementations must never
    /// return a drop verdict for anything they are unsure about, and must
    /// return an empty map rather than an error when they cannot answer.
    async fn judge(&self, goal: &GoalContext, candidates: &[Candidate]) -> Verdicts;
}

/// The judge used when pruning is enabled but no judge is configured: it keeps
/// everything, so the wiring is exercised and nothing is lost.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeepAllJudge;

#[async_trait]
impl ContextJudge for KeepAllJudge {
    fn name(&self) -> &'static str {
        "keep_all"
    }

    async fn judge(&self, _goal: &GoalContext, _candidates: &[Candidate]) -> Verdicts {
        Verdicts::new()
    }
}

/// Turns a pair of keep probabilities into a verdict, with fast-jev's
/// `decideCall` ladder: the result outranks the call, because a call whose
/// output is still needed obviously still matters.
pub fn decide(keep_call: f32, keep_result: f32, threshold: f32) -> KeepVerdict {
    if keep_result >= threshold {
        KeepVerdict::KEEP
    } else if keep_call >= threshold {
        KeepVerdict::DROP_RESULT
    } else {
        KeepVerdict::DROP_CALL
    }
}
