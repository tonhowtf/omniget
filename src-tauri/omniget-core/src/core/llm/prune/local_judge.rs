use async_trait::async_trait;

use crate::core::embed::{self, EmbedSource};

use super::judge::{ContextJudge, Verdicts};
use super::policy::{excerpt, Candidate, GoalContext, KeepVerdict};

const PREVIEW_CHARS: usize = 1600;

pub struct LocalJudge {
    threshold: f32,
    source: EmbedSource,
}

impl std::fmt::Debug for LocalJudge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalJudge")
            .field("threshold", &self.threshold)
            .field("source", &self.source)
            .finish()
    }
}

impl LocalJudge {
    pub fn new(threshold: f32) -> Self {
        Self {
            threshold,
            source: EmbedSource::Onnx,
        }
    }

    fn goal_text(goal: &GoalContext) -> String {
        let mut text = goal.goal.clone();
        if !goal.latest_assistant.is_empty() {
            text.push('\n');
            text.push_str(&goal.latest_assistant);
        }
        text
    }
}

#[async_trait]
impl ContextJudge for LocalJudge {
    fn name(&self) -> &'static str {
        "local"
    }

    async fn judge(&self, goal: &GoalContext, candidates: &[Candidate]) -> Verdicts {
        if candidates.is_empty() {
            return Verdicts::new();
        }
        if self.source == EmbedSource::Onnx && !embed::is_ready() {
            return Verdicts::new();
        }
        let goal_text = Self::goal_text(goal);
        if goal_text.trim().is_empty() {
            return Verdicts::new();
        }
        let mut texts: Vec<String> = Vec::with_capacity(candidates.len() + 1);
        texts.push(goal_text);
        for candidate in candidates {
            texts.push(format!(
                "{} {}\n{}",
                candidate.tool,
                candidate.input,
                excerpt(&candidate.text, PREVIEW_CHARS)
            ));
        }
        let refs: Vec<&str> = texts.iter().map(|t| t.as_str()).collect();
        let Ok(vectors) = embed::embed_batch_with(self.source, &refs).await else {
            return Verdicts::new();
        };
        if vectors.len() != candidates.len() + 1 {
            return Verdicts::new();
        }
        let goal_vec = &vectors[0];
        let mut verdicts = Verdicts::new();
        for (candidate, vector) in candidates.iter().zip(vectors[1..].iter()) {
            let similarity = embed::cosine(goal_vec, vector);
            if similarity < self.threshold {
                verdicts.insert(candidate.tool_use_id.clone(), KeepVerdict::DROP_RESULT);
            } else {
                verdicts.insert(candidate.tool_use_id.clone(), KeepVerdict::KEEP);
            }
        }
        verdicts
    }
}
