use std::collections::HashMap;

use async_trait::async_trait;

use super::policy::{Candidate, GoalContext, KeepVerdict};

pub type Verdicts = HashMap<String, KeepVerdict>;

#[async_trait]
pub trait ContextJudge: Send + Sync {
    fn name(&self) -> &'static str;

    async fn judge(&self, goal: &GoalContext, candidates: &[Candidate]) -> Verdicts;
}

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

pub fn decide(keep_call: f32, keep_result: f32, threshold: f32) -> KeepVerdict {
    if keep_result >= threshold {
        KeepVerdict::KEEP
    } else if keep_call >= threshold {
        KeepVerdict::DROP_RESULT
    } else {
        KeepVerdict::DROP_CALL
    }
}
