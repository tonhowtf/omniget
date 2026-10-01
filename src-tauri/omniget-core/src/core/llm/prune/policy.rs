//! Ported from `compozy/yoshi` (`src/rewrite.ts` `findCandidates`/`stub`/

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::core::llm::coordinator::is_safe_id;
use crate::core::llm::types::{ContentPart, Message, Role};

pub const ERR_PRUNE_STORE: &str = "ERR_PRUNE_STORE";

pub const MARKER_PREFIX: &str = "[omitted by context pruning";

const STICKY_SUFFIX: &str = ".prune.json";

pub const STICKY_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PruneJudge {
    #[default]
    Local,
    Jev,
}

impl PruneJudge {
    pub fn as_str(self) -> &'static str {
        match self {
            PruneJudge::Local => "local",
            PruneJudge::Jev => "jev",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "jev" => PruneJudge::Jev,
            _ => PruneJudge::Local,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PruneConfig {
    pub enabled: bool,
    pub judge: PruneJudge,
    pub recent_turns: u32,
    pub preserve_recent_messages: usize,
    pub min_chars: usize,
    pub min_tokens: u32,
    pub keep_threshold: f32,
    pub max_candidates: usize,
    pub timeout_ms: u64,
    pub local_threshold: f32,
}

impl Default for PruneConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            judge: PruneJudge::Local,
            recent_turns: 2,
            preserve_recent_messages: 6,
            min_chars: 1500,
            min_tokens: 50_000,
            keep_threshold: 0.5,
            max_candidates: 128,
            timeout_ms: 3000,
            local_threshold: 0.25,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeepVerdict {
    #[serde(default = "yes")]
    pub keep_call: bool,
    #[serde(default = "yes")]
    pub keep_result: bool,
}

fn yes() -> bool {
    true
}

impl Default for KeepVerdict {
    fn default() -> Self {
        Self::KEEP
    }
}

impl KeepVerdict {
    pub const KEEP: Self = Self {
        keep_call: true,
        keep_result: true,
    };
    pub const DROP_RESULT: Self = Self {
        keep_call: true,
        keep_result: false,
    };
    pub const DROP_CALL: Self = Self {
        keep_call: false,
        keep_result: false,
    };
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StickyEntry {
    #[serde(default = "yes")]
    pub keep_call: bool,
    #[serde(default = "yes")]
    pub keep_result: bool,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub by: String,
}

impl StickyEntry {
    pub fn verdict(&self) -> KeepVerdict {
        KeepVerdict {
            keep_call: self.keep_call,
            keep_result: self.keep_result,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StickyDecisions {
    pub version: u32,
    pub decided: BTreeMap<String, StickyEntry>,
}

impl StickyDecisions {
    pub fn contains(&self, tool_use_id: &str) -> bool {
        self.decided.contains_key(tool_use_id)
    }

    pub fn verdict(&self, tool_use_id: &str) -> Option<KeepVerdict> {
        self.decided.get(tool_use_id).map(StickyEntry::verdict)
    }

    pub fn record(&mut self, candidate: &Candidate, verdict: KeepVerdict, by: &str) {
        self.version = STICKY_VERSION;
        self.decided.insert(
            candidate.tool_use_id.clone(),
            StickyEntry {
                keep_call: verdict.keep_call,
                keep_result: verdict.keep_result,
                fingerprint: candidate.fingerprint.clone(),
                at: chrono::Utc::now().to_rfc3339(),
                by: by.to_string(),
            },
        );
    }

    pub fn omitted(&self) -> usize {
        self.decided.values().filter(|e| !e.keep_result).count()
    }
}

pub fn sticky_path(dir: &Path, conversation_id: &str) -> Option<PathBuf> {
    if !is_safe_id(conversation_id) {
        return None;
    }
    Some(dir.join(format!("{conversation_id}{STICKY_SUFFIX}")))
}

pub fn load_sticky(dir: &Path, conversation_id: &str) -> StickyDecisions {
    let Some(path) = sticky_path(dir, conversation_id) else {
        return StickyDecisions::default();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return StickyDecisions::default();
    };
    let mut store: StickyDecisions = serde_json::from_str(&text).unwrap_or_default();
    if store.version > STICKY_VERSION {
        return StickyDecisions::default();
    }
    store.version = STICKY_VERSION;
    store
}

pub fn save_sticky(
    dir: &Path,
    conversation_id: &str,
    store: &StickyDecisions,
) -> Result<(), String> {
    let path = sticky_path(dir, conversation_id)
        .ok_or_else(|| format!("{ERR_PRUNE_STORE}: unsafe conversation id `{conversation_id}`"))?;
    let text = serde_json::to_string(store)
        .map_err(|e| format!("{ERR_PRUNE_STORE}: could not encode the sticky store: {e}"))?;
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("{ERR_PRUNE_STORE}: could not create {}: {e}", dir.display()))?;
    std::fs::write(&path, text)
        .map_err(|e| format!("{ERR_PRUNE_STORE}: could not write {}: {e}", path.display()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub tool_use_id: String,
    pub tool: String,
    pub input: Value,
    pub chars: usize,
    pub age_turns: u32,
    pub fingerprint: String,
    pub text: String,
    pub call_index: usize,
    pub result_index: usize,
}

/// assistant text as the "what is happening now" line. Ported from yoshi's
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoalContext {
    pub goal: String,
    pub latest_assistant: String,
}

pub fn excerpt(text: &str, budget: usize) -> String {
    if text.chars().count() <= budget || budget <= 100 {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let part = (budget - 100) / 3;
    if part == 0 {
        return chars.iter().take(budget).collect();
    }
    let middle = chars.len() / 2 - part / 2;
    let head: String = chars.iter().take(part).collect();
    let mid: String = chars.iter().skip(middle).take(part).collect();
    let tail: String = chars.iter().skip(chars.len() - part).collect();
    format!("{head}\n[content omitted]\n{mid}\n[content omitted]\n{tail}")
}

pub fn has_execution_error(text: &str) -> bool {
    const MARKERS: [&str; 7] = [
        "Error:",
        "TypeError:",
        "SyntaxError:",
        "ReferenceError:",
        "Traceback (most recent call last)",
        "panic:",
        "FAILED",
    ];
    for line in text.lines() {
        let trimmed = line.trim_start();
        if MARKERS.iter().any(|m| trimmed.starts_with(m)) {
            return true;
        }
        if trimmed.starts_with("FAIL") && !trimmed.starts_with("FAILURES") {
            return true;
        }
        if let Some(rest) = exit_status_tail(trimmed) {
            if rest.starts_with(|c: char| ('1'..='9').contains(&c)) {
                return true;
            }
        }
    }
    false
}

fn exit_status_tail(line: &str) -> Option<&str> {
    let lower = line.to_ascii_lowercase();
    for prefix in [
        "exit code",
        "exit status",
        "exited with code",
        "exited with status",
    ] {
        if let Some(at) = lower.find(prefix) {
            let rest = line[at + prefix.len()..].trim_start_matches([':', ' ', '=']);
            return Some(rest);
        }
    }
    None
}

pub fn invocation_input(tool: &str, input: &Value) -> Value {
    if tool != "Bash" {
        return input.clone();
    }
    match input.as_object() {
        Some(map) => {
            let mut copy = map.clone();
            copy.remove("description");
            Value::Object(copy)
        }
        None => input.clone(),
    }
}

pub fn fingerprint(tool: &str, input: &Value, text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(tool.as_bytes());
    hasher.update([0u8]);
    hasher.update(invocation_input(tool, input).to_string().as_bytes());
    hasher.update([0u8]);
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn is_pinned(index: usize, total: usize, preserve_recent: usize) -> bool {
    index == 0 || index + preserve_recent >= total
}

pub fn goal_of(history: &[Message]) -> GoalContext {
    let mut prompts: Vec<String> = Vec::new();
    let mut latest_assistant = String::new();
    for message in history {
        let text = message
            .parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            continue;
        }
        match message.role {
            Role::User => prompts.push(excerpt(&text, 1800)),
            Role::Assistant => latest_assistant = excerpt(&text, 1400),
            _ => {}
        }
    }
    let start = prompts.len().saturating_sub(3);
    GoalContext {
        goal: prompts[start..].join("\n[Next user instruction]\n"),
        latest_assistant,
    }
}

pub fn find_candidates(history: &[Message], config: &PruneConfig) -> Vec<Candidate> {
    let mut assistants_after = vec![0u32; history.len()];
    let mut seen = 0u32;
    for index in (0..history.len()).rev() {
        assistants_after[index] = seen;
        if history[index].role == Role::Assistant {
            seen += 1;
        }
    }

    let mut uses: BTreeMap<String, (String, Value, usize)> = BTreeMap::new();
    for (index, message) in history.iter().enumerate() {
        if message.role != Role::Assistant {
            continue;
        }
        for part in &message.parts {
            if let ContentPart::ToolUse { id, name, input } = part {
                uses.insert(id.clone(), (name.clone(), input.clone(), index));
            }
        }
    }

    let total = history.len();
    let mut candidates = Vec::new();
    for (index, message) in history.iter().enumerate() {
        for part in &message.parts {
            let ContentPart::ToolResult {
                tool_use_id,
                content,
                is_error,
            } = part
            else {
                continue;
            };
            if *is_error || content.starts_with(MARKER_PREFIX) || has_execution_error(content) {
                continue;
            }
            let Some((name, input, call_index)) = uses.get(tool_use_id) else {
                continue;
            };
            if is_pinned(index, total, config.preserve_recent_messages)
                || is_pinned(*call_index, total, config.preserve_recent_messages)
            {
                continue;
            }
            let age = assistants_after[index];
            if age <= config.recent_turns || content.len() < config.min_chars {
                continue;
            }
            candidates.push(Candidate {
                tool_use_id: tool_use_id.clone(),
                tool: name.clone(),
                input: input.clone(),
                chars: content.len(),
                age_turns: age,
                fingerprint: fingerprint(name, input, content),
                text: content.clone(),
                call_index: *call_index,
                result_index: index,
            });
        }
    }
    candidates.truncate(config.max_candidates);
    candidates
}

pub fn result_marker(tool_use_id: &str, bytes: usize) -> String {
    format!("{MARKER_PREFIX}: tool_result {tool_use_id}, {bytes} bytes]")
}

fn is_locator_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "path", "file", "dir", "url", "uri", "src", "dest", "target", "pattern",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

pub fn omitted_call_input(tool: &str, input: &Value, bytes: usize) -> Value {
    let mut kept = Map::new();
    if let Some(map) = input.as_object() {
        for (key, value) in map {
            if !is_locator_key(key) {
                continue;
            }
            match value {
                Value::String(s) => {
                    kept.insert(key.clone(), json!(excerpt(s, 512)));
                }
                Value::Array(_) | Value::Object(_) => {}
                other => {
                    kept.insert(key.clone(), other.clone());
                }
            }
        }
    }
    json!({
        "omitted_by_context_pruning": true,
        "tool": tool,
        "omitted_bytes": bytes,
        "kept": Value::Object(kept),
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneStats {
    pub omitted: usize,
    pub calls_reduced: usize,
    pub candidates: usize,
}

pub fn apply(history: &[Message], sticky: &StickyDecisions) -> (Vec<Message>, PruneStats) {
    let mut stats = PruneStats::default();
    if sticky.decided.is_empty() {
        return (history.to_vec(), stats);
    }
    let out = history
        .iter()
        .map(|message| {
            let mut parts = Vec::with_capacity(message.parts.len());
            let mut changed = false;
            for part in &message.parts {
                match part {
                    ContentPart::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        let verdict = sticky.verdict(tool_use_id).unwrap_or(KeepVerdict::KEEP);
                        let marker = result_marker(tool_use_id, content.len());
                        if verdict.keep_result
                            || *is_error
                            || content.starts_with(MARKER_PREFIX)
                            || marker.len() >= content.len()
                        {
                            parts.push(part.clone());
                            continue;
                        }
                        changed = true;
                        stats.omitted += 1;
                        parts.push(ContentPart::ToolResult {
                            tool_use_id: tool_use_id.clone(),
                            content: marker,
                            is_error: false,
                        });
                    }
                    ContentPart::ToolUse { id, name, input } => {
                        let verdict = sticky.verdict(id).unwrap_or(KeepVerdict::KEEP);
                        if verdict.keep_call {
                            parts.push(part.clone());
                            continue;
                        }
                        changed = true;
                        stats.calls_reduced += 1;
                        parts.push(ContentPart::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: omitted_call_input(name, input, input.to_string().len()),
                        });
                    }
                    other => parts.push(other.clone()),
                }
            }
            if changed {
                Message {
                    role: message.role,
                    parts,
                }
            } else {
                message.clone()
            }
        })
        .collect();
    (out, stats)
}

pub fn below_size_gate(history: &[Message], config: &PruneConfig) -> bool {
    crate::core::llm::coordinator::estimate_tokens(history) < config.min_tokens
}
