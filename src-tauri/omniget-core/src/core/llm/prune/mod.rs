//! Ported from `compozy/yoshi` and `tamaratran/fast-jev-compaction`; the

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

pub struct ContextPruner {
    config: PruneConfig,
    judge: Option<Arc<dyn ContextJudge>>,
    dir: Option<PathBuf>,
    sticky: Mutex<HashMap<String, StickyDecisions>>,
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

    pub fn is_active(&self) -> bool {
        self.config.enabled && self.judge.is_some()
    }

    fn judge_name(&self) -> &'static str {
        self.judge.as_ref().map(|j| j.name()).unwrap_or("none")
    }

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
            let _ = save_sticky(dir, conversation_id, &store);
        }
        if let Ok(mut map) = self.sticky.lock() {
            map.insert(conversation_id.to_string(), store);
        }
    }

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

    pub fn omitted_count(&self, conversation_id: &str) -> usize {
        self.sticky_of(conversation_id).omitted()
    }

    pub fn omitted_total(&self) -> usize {
        self.sticky
            .lock()
            .map(|m| m.values().map(|s| s.omitted()).sum())
            .unwrap_or(0)
    }

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
