//! Context pruning: between turns, a judge decides which old tool outputs no
//! longer bear on what is being done, and those outputs are replaced by an
//! explicit marker in the history the model sees.
//!
//! Three rules shape everything here, all taken from the originals:
//!
//! 1. **Never on the hot path.** Judging runs in its own task after a turn
//!    ends. The SSE loop never awaits it, and a turn that starts while a
//!    judgement is still running simply uses the history as it stands.
//! 2. **Sticky.** Once a `tool_use_id` has a verdict it is never judged again,
//!    even if the verdict was "keep". Re-judging would rewrite a prefix the
//!    provider has already cached (`yoshi/src/lifecycle.ts`, `StickyContext`).
//! 3. **Fail-open.** Every error, timeout, half-answer, missing model or
//!    unreadable store means keep. Nothing is ever omitted by accident.
//!
//! Nothing is ever deleted: a pruned pair keeps its `tool_use_id`, its tool
//! name and its locators, so the request stays valid and the assistant can
//! re-run the tool. The omitted history is text only — it is never re-executed.
//!
//! Ported from `compozy/yoshi` and `tamaratran/fast-jev-compaction`; the
//! per-module doc comments name the files and functions.

pub mod jev;
pub mod judge;
pub mod local_judge;
pub mod policy;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use jev::JevJudge;
pub use judge::{decide, ContextJudge, KeepAllJudge, Verdicts};
pub use local_judge::LocalJudge;
pub use policy::{
    apply, below_size_gate, find_candidates, goal_of, load_sticky, result_marker, save_sticky,
    sticky_path, Candidate, GoalContext, KeepVerdict, PruneConfig, PruneJudge, PruneStats,
    StickyDecisions, StickyEntry, ERR_PRUNE_STORE, MARKER_PREFIX, STICKY_VERSION,
};

use crate::core::llm::types::Message;

/// Owns the config, the judge and the sticky store of every conversation.
pub struct ContextPruner {
    config: PruneConfig,
    judge: Option<Arc<dyn ContextJudge>>,
    /// Where the `<id>.prune.json` sidecars live. `None` keeps the sticky
    /// store in memory only, which is what a test or a headless run wants.
    dir: Option<PathBuf>,
    sticky: Mutex<HashMap<String, StickyDecisions>>,
    /// One judgement at a time: a long turn asks after every tool round, and
    /// two passes over the same store would overwrite each other's verdicts.
    judging: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for ContextPruner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextPruner")
            .field("enabled", &self.config.enabled)
            .field("judge", &self.judge.as_ref().map(|j| j.name()))
            .field("dir", &self.dir)
            .finish()
    }
}

impl Default for ContextPruner {
    fn default() -> Self {
        Self::disabled()
    }
}

impl ContextPruner {
    /// What a Coordinator gets until someone configures pruning: no judge, no
    /// store, no work.
    pub fn disabled() -> Self {
        Self {
            config: PruneConfig::default(),
            judge: None,
            dir: None,
            sticky: Mutex::new(HashMap::new()),
            judging: tokio::sync::Mutex::new(()),
        }
    }

    pub fn new(config: PruneConfig, judge: Option<Arc<dyn ContextJudge>>) -> Self {
        Self {
            config,
            judge,
            dir: None,
            sticky: Mutex::new(HashMap::new()),
            judging: tokio::sync::Mutex::new(()),
        }
    }

    pub fn with_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.dir = dir;
        self
    }

    pub fn config(&self) -> &PruneConfig {
        &self.config
    }

    /// Pruning does something only when it is on and a judge exists.
    pub fn is_active(&self) -> bool {
        self.config.enabled && self.judge.is_some()
    }

    fn judge_name(&self) -> &'static str {
        self.judge.as_ref().map(|j| j.name()).unwrap_or("none")
    }

    /// The sticky store of one conversation, read through a small cache. The
    /// disk is touched once per conversation per process.
    fn sticky_of(&self, conversation_id: &str) -> StickyDecisions {
        if let Some(store) = self
            .sticky
            .lock()
            .ok()
            .and_then(|m| m.get(conversation_id).cloned())
        {
            return store;
        }
        let loaded = match &self.dir {
            Some(dir) => load_sticky(dir, conversation_id),
            None => StickyDecisions::default(),
        };
        if let Ok(mut map) = self.sticky.lock() {
            map.insert(conversation_id.to_string(), loaded.clone());
        }
        loaded
    }

    fn store_sticky(&self, conversation_id: &str, store: StickyDecisions) {
        if let Some(dir) = &self.dir {
            // A store that cannot be saved only costs a re-judge next time.
            let _ = save_sticky(dir, conversation_id, &store);
        }
        if let Ok(mut map) = self.sticky.lock() {
            map.insert(conversation_id.to_string(), store);
        }
    }

    /// The request-path half: rewrite the history from decisions that were
    /// already made. No judging, no I/O beyond the first load, no awaiting.
    pub fn apply_sticky(
        &self,
        conversation_id: &str,
        history: &[Message],
    ) -> (Vec<Message>, PruneStats) {
        if !self.config.enabled {
            return (history.to_vec(), PruneStats::default());
        }
        apply(history, &self.sticky_of(conversation_id))
    }

    /// How many results are currently omitted in this conversation. Exposed
    /// for the UI as a count of items — never as a saving ratio.
    pub fn omitted_count(&self, conversation_id: &str) -> usize {
        self.sticky_of(conversation_id).omitted()
    }

    /// Omissions over the conversations this process has touched. A count for
    /// the settings card, never a ratio.
    pub fn omitted_total(&self) -> usize {
        self.sticky
            .lock()
            .map(|m| m.values().map(|s| s.omitted()).sum())
            .unwrap_or(0)
    }

    /// The between-turns half: judge whatever is new and freeze the verdicts.
    /// Returns what the *next* turn will omit. Cheap and a no-op when pruning
    /// is off, below the size gate, or has nothing new to look at.
    pub async fn judge_turn(&self, conversation_id: &str, history: &[Message]) -> PruneStats {
        let mut stats = PruneStats::default();
        let Some(judge) = self.judge.clone() else {
            return stats;
        };
        if !self.config.enabled || below_size_gate(history, &self.config) {
            return stats;
        }
        let _one_at_a_time = self.judging.lock().await;
        let mut sticky = self.sticky_of(conversation_id);
        // Sticky: anything already decided is skipped, keep or not.
        let candidates: Vec<Candidate> = find_candidates(history, &self.config)
            .into_iter()
            .filter(|c| !sticky.contains(&c.tool_use_id))
            .collect();
        stats.candidates = candidates.len();
        if candidates.is_empty() {
            return stats;
        }
        let goal = goal_of(history);
        let verdicts = judge.judge(&goal, &candidates).await;
        let name = self.judge_name();
        let mut changed = false;
        for candidate in &candidates {
            // Unanswered is kept, and the keep is frozen too, so a judge that
            // was down once does not get a second chance at the same prefix.
            let verdict = verdicts
                .get(&candidate.tool_use_id)
                .copied()
                .unwrap_or(KeepVerdict::KEEP);
            if !verdict.keep_result {
                stats.omitted += 1;
            }
            if !verdict.keep_call {
                stats.calls_reduced += 1;
            }
            sticky.record(candidate, verdict, name);
            changed = true;
        }
        if changed {
            self.store_sticky(conversation_id, sticky);
        }
        stats
    }
}

/// Builds the judge a config asks for. `Jev` without a key in the
/// `SecretStore` yields `None`, so selecting it and never pasting a key is the
/// same as leaving pruning off.
pub fn judge_for(config: &PruneConfig) -> Option<Arc<dyn ContextJudge>> {
    if !config.enabled {
        return None;
    }
    match config.judge {
        PruneJudge::Local => Some(Arc::new(LocalJudge::new(config.local_threshold))),
        PruneJudge::Jev => JevJudge::from_secrets(config.keep_threshold, config.timeout_ms)
            .map(|j| Arc::new(j) as Arc<dyn ContextJudge>),
    }
}
