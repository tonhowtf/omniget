//! Where the answer of an ask comes from (estudo 98/03 §B.10), best first:
//!
//! 1. an explicit `omniget canvas reply`, or an ask back from the target;
//! 2. `last_assistant_message` of the `Stop` hook (Claude), the `notify`
//!    payload of Codex (`last-assistant-message`), Gemini's `AfterAgent`;
//! 3. the last assistant message of the `transcript_path` JSONL every
//!    Claude hook receives;
//! 4. native node: the final text of the turn;
//! 5. the shadow screen, what appeared since the delivery (last 80 lines),
//!    marked `screen`.
//!
//! Answers over 64 KiB go to a file and the answer carries the path.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_ANSWER_BYTES: usize = 64 * 1024;
pub const SCREEN_LINES: usize = 80;
/// How much of a transcript's tail is read.
const TRANSCRIPT_TAIL: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerSource {
    Reply,
    Hook,
    Transcript,
    Native,
    Screen,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub text: String,
    pub source: AnswerSource,
    pub truncated: bool,
    pub file: Option<String>,
}

impl Answer {
    pub fn new(text: impl Into<String>, source: AnswerSource) -> Self {
        Self {
            text: text.into(),
            source,
            truncated: false,
            file: None,
        }
    }

    /// Applies the 64 KiB cap: the full text goes to `<dir>/<id>-answer.md`,
    /// the answer keeps its head and the path.
    pub fn capped(mut self, dir: &Path, id: &str) -> Self {
        if self.text.len() <= MAX_ANSWER_BYTES {
            return self;
        }
        let path = dir.join(format!("{id}-answer.md"));
        let written = std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(&path, &self.text))
            .is_ok();
        let mut cut = 4 * 1024;
        while !self.text.is_char_boundary(cut) {
            cut -= 1;
        }
        let head = self.text[..cut].to_string();
        self.text = if written {
            format!("{head}\n\n[... the full answer is in {}]", path.display())
        } else {
            format!("{head}\n\n[... truncated]")
        };
        self.truncated = true;
        self.file = written.then(|| path.to_string_lossy().into_owned());
        self
    }
}

fn str_of<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
        .filter(|s| !s.trim().is_empty())
}

/// The answer a hook payload carries: Claude `Stop`
/// (`last_assistant_message`), Codex notify (`last-assistant-message`),
/// Gemini `AfterAgent` (`prompt_response` / `response`).
pub fn from_hook(payload: &Value) -> Option<String> {
    str_of(
        payload,
        &[
            "last_assistant_message",
            "last-assistant-message",
            "lastAssistantMessage",
            "prompt_response",
            "response",
        ],
    )
    .map(|s| s.trim().to_string())
}

/// Text of one message object (`content` as a string or as blocks).
fn message_text(m: &Value) -> Option<String> {
    let content = m.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string()).filter(|s| !s.trim().is_empty());
    }
    let parts: Vec<&str> = content
        .as_array()?
        .iter()
        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
        .collect();
    let t = parts.join("\n");
    (!t.trim().is_empty()).then_some(t)
}

/// Last assistant text of a transcript in JSONL (Claude's format:
/// `{"type":"assistant","message":{"role":"assistant","content":[...]}}`;
/// also plain `{"role":"assistant","content":...}` lines).
pub fn last_assistant_in_jsonl(text: &str) -> Option<String> {
    for line in text.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let msg = v.get("message").unwrap_or(&v);
        let is_assistant = v.get("type").and_then(|t| t.as_str()) == Some("assistant")
            || msg.get("role").and_then(|r| r.as_str()) == Some("assistant");
        if !is_assistant {
            continue;
        }
        if let Some(t) = message_text(msg) {
            return Some(t.trim().to_string());
        }
    }
    None
}

/// Reads the tail of a transcript file and returns its last assistant text.
pub fn from_transcript(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TRANSCRIPT_TAIL);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    // A cut first line is not JSON and is skipped by the parser.
    last_assistant_in_jsonl(&text)
}

/// Background tasks (Claude Code `run_in_background` shells and agents) that
/// a transcript started at or after `since_ms` (epoch ms) and that have not
/// reported back yet. A `Stop` while one runs is not the end of the work:
/// Claude answers "waiting for ..." and starts a new turn when the task's
/// `<task-notification>` arrives.
pub fn pending_background_in_jsonl(text: &str, since_ms: i64) -> Vec<String> {
    let mut open: Vec<String> = Vec::new();
    let mut closed: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|d| d.timestamp_millis());
        let msg = v.get("message").unwrap_or(&v);
        let role = v
            .get("type")
            .and_then(|t| t.as_str())
            .or_else(|| msg.get("role").and_then(|r| r.as_str()))
            .unwrap_or("");
        match role {
            "user" => {
                let started = v
                    .get("toolUseResult")
                    .and_then(|r| r.get("backgroundTaskId"))
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
                    .or_else(|| {
                        let t = message_text_any(msg);
                        let i = t.find("running in background with ID: ")?;
                        let rest = &t[i + "running in background with ID: ".len()..];
                        let id: String = rest
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                            .collect();
                        (!id.is_empty()).then_some(id)
                    });
                if let Some(id) = started {
                    if ts.is_none_or(|t| t >= since_ms) && !open.contains(&id) {
                        open.push(id);
                    }
                }
                let t = message_text_any(msg);
                let mut rest = t.as_str();
                while let Some(i) = rest.find("<task-id>") {
                    let after = &rest[i + "<task-id>".len()..];
                    let Some(j) = after.find("</task-id>") else {
                        break;
                    };
                    closed.insert(after[..j].trim().to_string());
                    rest = &after[j..];
                }
            }
            "assistant" => {
                // KillShell / TaskStop end a task without a notification.
                for b in msg
                    .get("content")
                    .and_then(|c| c.as_array())
                    .into_iter()
                    .flatten()
                {
                    if b.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                        continue;
                    }
                    let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if !matches!(name, "KillShell" | "KillBash" | "TaskStop") {
                        continue;
                    }
                    let input = b.get("input").cloned().unwrap_or(Value::Null);
                    for k in ["shell_id", "bash_id", "task_id", "id"] {
                        if let Some(id) = input.get(k).and_then(|x| x.as_str()) {
                            closed.insert(id.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    open.retain(|id| !closed.contains(id));
    open
}

/// Every text a message carries (string content, text blocks and the
/// string content of tool results).
fn message_text_any(m: &Value) -> String {
    let Some(content) = m.get("content") else {
        return String::new();
    };
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    let mut out = String::new();
    for b in content.as_array().into_iter().flatten() {
        if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
            out.push_str(t);
            out.push('\n');
        }
        match b.get("content") {
            Some(Value::String(s)) => {
                out.push_str(s);
                out.push('\n');
            }
            Some(Value::Array(a)) => {
                for x in a {
                    if let Some(t) = x.get("text").and_then(|t| t.as_str()) {
                        out.push_str(t);
                        out.push('\n');
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// [`pending_background_in_jsonl`] over the tail of a transcript file.
pub fn pending_background(path: &Path, since_ms: i64) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(TRANSCRIPT_TAIL);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    pending_background_in_jsonl(&String::from_utf8_lossy(&buf), since_ms)
}

/// The transcript path a Claude hook carries.
pub fn transcript_path(payload: &Value) -> Option<PathBuf> {
    str_of(payload, &["transcript_path", "transcriptPath"]).map(PathBuf::from)
}

/// What appeared on the screen since `before` (both are screen texts, top
/// to bottom). Handles scrolling: finds the longest suffix of `before` that
/// is a prefix of `after` and returns what follows it. Falls back to the last
/// `max_lines` of `after`.
pub fn screen_diff(before: &str, after: &str, max_lines: usize) -> String {
    let b: Vec<&str> = before.lines().map(str::trim_end).collect();
    let a: Vec<&str> = after.lines().map(str::trim_end).collect();
    let trim = |lines: &[&str]| -> String {
        let lines: Vec<&str> = lines.to_vec();
        let start = lines.len().saturating_sub(max_lines);
        lines[start..].join("\n").trim().to_string()
    };
    // Ignore trailing blank lines of `before` (the empty screen below the prompt).
    let mut blen = b.len();
    while blen > 0 && b[blen - 1].is_empty() {
        blen -= 1;
    }
    let b = &b[..blen];
    for skip in 0..b.len() {
        let tail = &b[skip..];
        // Too short a tail matches by accident.
        if tail.len() < 3.min(b.len()) {
            break;
        }
        if a.len() >= tail.len() && a[..tail.len()] == *tail {
            return trim(&a[tail.len()..]);
        }
    }
    trim(&a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hook_payloads_of_each_agent() {
        assert_eq!(
            from_hook(&json!({"hook_event_name":"Stop","last_assistant_message":" 42 "})),
            Some("42".into())
        );
        assert_eq!(
            from_hook(&json!({"type":"agent-turn-complete","last-assistant-message":"done"})),
            Some("done".into())
        );
        assert_eq!(from_hook(&json!({"hook_event_name":"Stop"})), None);
    }

    #[test]
    fn the_last_assistant_line_of_a_transcript() {
        let t = [
            json!({"type":"user","message":{"role":"user","content":"hi"}}).to_string(),
            json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"first"}]}}).to_string(),
            json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash"}]}}).to_string(),
            json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"final"}]}}).to_string(),
            json!({"type":"system","content":"x"}).to_string(),
        ]
        .join("\n");
        assert_eq!(last_assistant_in_jsonl(&t), Some("final".into()));
    }

    #[test]
    fn background_tasks_hold_the_turn_until_notified() {
        let started = |id: &str, ts: &str| {
            json!({"type":"user","timestamp":ts,"message":{"role":"user","content":[{"type":"tool_result","content":format!("Command running in background with ID: {id}. Output is being written to: /tmp/x")}]},"toolUseResult":{"backgroundTaskId":id}}).to_string()
        };
        let done = |id: &str| {
            json!({"type":"user","message":{"role":"user","content":format!("<task-notification>\n<task-id>{id}</task-id>\n<status>completed</status>\n</task-notification>")}}).to_string()
        };
        let t = [
            started("old1", "2026-01-01T00:00:00Z"),
            started("b1", "2026-09-30T10:00:00Z"),
            started("b2", "2026-09-30T10:00:01Z"),
            done("b1"),
        ]
        .join("\n");
        let since = chrono::DateTime::parse_from_rfc3339("2026-09-30T09:00:00Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            pending_background_in_jsonl(&t, since),
            vec!["b2".to_string()]
        );
        let killed = json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"KillShell","input":{"shell_id":"b2"}}]}}).to_string();
        assert!(pending_background_in_jsonl(&format!("{t}\n{killed}"), since).is_empty());
    }

    #[test]
    fn screen_diff_follows_scrolling() {
        let before = "a\nb\nc\n> ask\n\n";
        let after = "b\nc\n> ask\nanswer 1\nanswer 2";
        assert_eq!(screen_diff(before, after, 80), "answer 1\nanswer 2");
        assert_eq!(screen_diff("zzz\nyyy\nxxx", "1\n2\n3", 2), "2\n3");
    }

    #[test]
    fn big_answers_go_to_a_file() {
        let dir =
            std::env::temp_dir().join(format!("omniget-ans-{}", uuid::Uuid::new_v4().simple()));
        let a = Answer::new("é".repeat(MAX_ANSWER_BYTES), AnswerSource::Hook).capped(&dir, "ask_1");
        assert!(a.truncated);
        let f = a.file.clone().unwrap();
        assert_eq!(
            std::fs::read_to_string(&f).unwrap().len(),
            MAX_ANSWER_BYTES * 2
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
