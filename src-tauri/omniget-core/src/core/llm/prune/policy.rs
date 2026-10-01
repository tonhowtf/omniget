//! What may be pruned, what the omission looks like, and the sticky store that
//! makes a decision permanent.
//!
//! Ported from `compozy/yoshi` (`src/rewrite.ts` `findCandidates`/`stub`/
//! `applyDecisions`/`hasExecutionError`/`excerpt`, `src/lifecycle.ts`
//! `StickyContext`) and `tamaratran/fast-jev-compaction` (`src/state.ts`
//! `collectToolCalls`/`isPinned`/`goalFromMessages`, `src/compact.ts`
//! `DEFAULT_OPTIONS`/`decideCall`). Thresholds are the originals'.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::core::llm::coordinator::is_safe_id;
use crate::core::llm::types::{ContentPart, Message, Role};

/// The sticky store could not be written. Never fatal: pruning is an
/// optimisation, and a store that fails to save only means the next turn
/// re-judges.
pub const ERR_PRUNE_STORE: &str = "ERR_PRUNE_STORE";

/// Every omission marker starts with this, so a marker is never judged again
/// and never counted as a candidate (yoshi's `STUB_PREFIX`).
pub const MARKER_PREFIX: &str = "[omitted by context pruning";

/// Sidecar file next to `<id>.jsonl`. The transcript itself is never rewritten:
/// the omission is applied to the in-memory history the model sees, so the UI
/// and a future re-read keep the original bytes.
const STICKY_SUFFIX: &str = ".prune.json";

/// Bumped whenever a change would make old verdicts unsafe to reuse.
pub const STICKY_VERSION: u32 = 1;

/// Which judge answers. `Local` is the offline MiniLM one; `Jev` talks to
/// TypeSafe's System One and is opt-in.
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

    /// Anything unknown falls back to the offline judge; a typo in settings
    /// must never silently send spans to a third party.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "jev" => PruneJudge::Jev,
            _ => PruneJudge::Local,
        }
    }
}

/// The knobs, with the originals' defaults. `enabled` is false: pruning is
/// opt-in, and a Coordinator that was never configured prunes nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PruneConfig {
    pub enabled: bool,
    pub judge: PruneJudge,
    /// Assistant turns that must have passed before a result is a candidate
    /// (`YOSHI_RECENT_TURNS`, default 2).
    pub recent_turns: u32,
    /// Newest messages never touched (fast-jev `preserveRecentMessages`, 6).
    pub preserve_recent_messages: usize,
    /// Per-candidate size gate (`YOSHI_MIN_CHARS`, 1500).
    pub min_chars: usize,
    /// Conversation-level size gate: below this estimate nothing is judged
    /// (yoshi `minTokens`, 50000).
    pub min_tokens: u32,
    /// Minimum keep probability for a call or a result to stay (fast-jev 0.5).
    pub keep_threshold: f32,
    /// Ceiling on candidates per pass (yoshi `maxCandidates`, 128).
    pub max_candidates: usize,
    /// Judge deadline (yoshi legacy judge, 3000 ms).
    pub timeout_ms: u64,
    /// Cosine similarity below which the local judge omits a result.
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

/// One verdict per `tool_use_id`. Both fields default to true so a partial or
/// unknown record read back from disk means "keep" (fail-open).
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
    /// The call still matters, its output does not — fast-jev's `drop_result`.
    pub const DROP_RESULT: Self = Self {
        keep_call: true,
        keep_result: false,
    };
    /// Neither matters — fast-jev's `drop_call`. The blocks stay in place with
    /// their ids so the pair is never orphaned; only the payload goes.
    pub const DROP_CALL: Self = Self {
        keep_call: false,
        keep_result: false,
    };
}

/// One frozen decision. `fingerprint` guards against a result that changed
/// under the same id; `by` records which judge answered.
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

/// The sticky state of one conversation. Every field carries `#[serde(default)]`
/// so a file written by an older build — or no file at all — reads back as
/// "nothing decided yet" instead of failing the conversation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StickyDecisions {
    pub version: u32,
    pub decided: BTreeMap<String, StickyEntry>,
}

impl StickyDecisions {
    /// Once a `tool_use_id` is in here it is never judged again: re-judging
    /// would rewrite a prefix the provider has already cached.
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

    /// How many ids are omitted. The UI may show this count; it must never be
    /// turned into a saving percentage.
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

/// A missing, unreadable or corrupt file is an empty store: a broken sidecar
/// must not take the chat down.
pub fn load_sticky(dir: &Path, conversation_id: &str) -> StickyDecisions {
    let Some(path) = sticky_path(dir, conversation_id) else {
        return StickyDecisions::default();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return StickyDecisions::default();
    };
    let mut store: StickyDecisions = serde_json::from_str(&text).unwrap_or_default();
    if store.version > STICKY_VERSION {
        // A newer build wrote this. Its verdicts may not mean what they mean
        // here, so start over rather than misapply them.
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

// ---------------------------------------------------------------------------
// Candidates
// ---------------------------------------------------------------------------

/// A complete tool_use/tool_result pair that passed every gate.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub tool_use_id: String,
    pub tool: String,
    pub input: Value,
    /// Bytes of the result content, what the marker reports.
    pub chars: usize,
    /// Assistant turns that happened after the result.
    pub age_turns: u32,
    pub fingerprint: String,
    /// The result content, used to build the judge's preview.
    pub text: String,
    pub call_index: usize,
    pub result_index: usize,
}

/// The goal the judge scores against: the recent user messages, plus the last
/// assistant text as the "what is happening now" line. Ported from yoshi's
/// `goalOf` and fast-jev's `goalFromMessages` (last three user prompts).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoalContext {
    pub goal: String,
    pub latest_assistant: String,
}

/// Head, middle and tail: a prefix alone hides the row that matters, and the
/// omitted region is explicitly not evidence (yoshi `excerpt`).
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

/// Protect execution failures even when a tool forgot its error flag. A
/// success summary such as "0 errors" is not a failure (yoshi
/// `hasExecutionError`, hand-rolled here to avoid a regex dependency).
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

/// The text right after an "exit code:"/"exited with status " prefix, if any.
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

/// Claude Code's Bash `description` is a UI label, not part of the execution.
/// It must not make two identical reads look like different evidence
/// (yoshi `invocationInput`).
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

/// `true` when the message is too new (or is the very first one) to be touched
/// — fast-jev's `isPinned`.
pub fn is_pinned(index: usize, total: usize, preserve_recent: usize) -> bool {
    index == 0 || index + preserve_recent >= total
}

/// The last three user texts, as the goal (fast-jev `goalFromMessages`), plus
/// the newest assistant text.
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

/// Every complete pair that may be judged. Everything else — user text,
/// assistant text, errors, incomplete pairs, the pinned head and tail, and
/// anything already carrying a marker — is left out of the result and is
/// therefore untouchable by construction.
pub fn find_candidates(history: &[Message], config: &PruneConfig) -> Vec<Candidate> {
    // Assistant turns after each message, oldest-last (yoshi `assistantsAfter`).
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
            // Errors are never pruned: a failure is the cheapest thing in the
            // transcript and the most expensive to lose.
            if *is_error || content.starts_with(MARKER_PREFIX) || has_execution_error(content) {
                continue;
            }
            let Some((name, input, call_index)) = uses.get(tool_use_id) else {
                // No matching tool_use: pruning it would orphan the pair.
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

// ---------------------------------------------------------------------------
// Applying decisions
// ---------------------------------------------------------------------------

pub fn result_marker(tool_use_id: &str, bytes: usize) -> String {
    format!("{MARKER_PREFIX}: tool_result {tool_use_id}, {bytes} bytes]")
}

/// Keys whose value is worth keeping verbatim when the call itself is omitted:
/// a path or a URL is how the assistant re-runs the tool.
fn is_locator_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "path", "file", "dir", "url", "uri", "src", "dest", "target", "pattern",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

/// The call is omitted but never removed: the block keeps its id and tool name
/// so the pair stays valid for the API, and every locator survives so the tool
/// can be re-run.
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

/// How a pass changed the history.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneStats {
    /// Pairs whose result now carries a marker.
    pub omitted: usize,
    /// Of those, the ones whose call input was reduced too.
    pub calls_reduced: usize,
    pub candidates: usize,
}

/// Rewrites the history in place from the sticky store. Nothing is removed and
/// no `tool_use_id` changes, so a pruned history is still a valid request.
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
                        // A marker longer than what it replaces saves nothing.
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

/// The conversation-level size gate: below it nothing is judged at all
/// (yoshi's `gate`, bytes/4 against `minTokens`).
pub fn below_size_gate(history: &[Message], config: &PruneConfig) -> bool {
    crate::core::llm::coordinator::estimate_tokens(history) < config.min_tokens
}
