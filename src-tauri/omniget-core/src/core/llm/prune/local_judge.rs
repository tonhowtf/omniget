//! The offline judge: MiniLM-L6-v2, the embedder the core already carries
//! (`core::embed`). No model is added and nothing is downloaded — if the model
//! is not on disk, or the runtime is not resolvable, or inference fails, the
//! judge answers nothing and everything is kept.
//!
//! It answers one of the two questions only. Cosine similarity between the
//! recent goal and a tool result says whether the *output* still bears on what
//! is being done; it says nothing about whether making the call mattered. So
//! the local judge never returns `DROP_CALL`: the worst it does is replace an
//! output with a marker, and the call, its id, its name and its paths stay.

use async_trait::async_trait;

use crate::core::embed::{self, EmbedSource};

use super::judge::{ContextJudge, Verdicts};
use super::policy::{excerpt, Candidate, GoalContext, KeepVerdict};

/// Chars of a tool result fed to the embedder. MiniLM truncates at 256
/// WordPiece tokens anyway; a head/middle/tail excerpt of this size keeps the
/// shape of the document instead of only its first lines.
const PREVIEW_CHARS: usize = 1600;

pub struct LocalJudge {
    /// Cosine below which the result is omitted.
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

    /// What the goal vector is built from: the recent user messages plus the
    /// newest assistant text, which together are "what is being done now".
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
        // Never pull the model down from here: pruning is a background nicety,
        // not a reason to start a 90 MB download behind the user's back.
        if self.source == EmbedSource::Onnx && !embed::is_ready() {
            return Verdicts::new();
        }
        let goal_text = Self::goal_text(goal);
        if goal_text.trim().is_empty() {
            // Without a goal there is nothing to be irrelevant to.
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
            // The embedder did not load, or inference failed: keep everything.
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
