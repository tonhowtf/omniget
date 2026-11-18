use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::core::llm::cli_usage::{CliKind, CliUsageEntry, FileParser, Parsed};

pub const MAX_CHUNK: u64 = 16 * 1024 * 1024;
const KEPT_KEYS: usize = 512;

#[derive(Debug, Clone, Default)]
pub struct Tail {
    pub path: PathBuf,
    pub offset: u64,
    pub tokens: u64,
    pub usd: f64,
    pub unpriced: bool,
    keys: VecDeque<String>,
}

impl Tail {
    pub fn for_path(self, path: &Path) -> Self {
        if self.path == path {
            self
        } else {
            Tail {
                path: path.to_path_buf(),
                ..Tail::default()
            }
        }
    }

    fn is_new(&mut self, entry: &CliUsageEntry) -> bool {
        let Some(key) = &entry.dedupe_key else {
            return true;
        };
        if self.keys.contains(key) {
            return false;
        }
        if self.keys.len() >= KEPT_KEYS {
            self.keys.pop_front();
        }
        self.keys.push_back(key.clone());
        true
    }

    pub fn take(&mut self, text: &str, session: &str) -> Vec<CliUsageEntry> {
        let mut parser = FileParser::new(CliKind::Claude, "", "", session);
        let mut fresh = Vec::new();
        for line in text.lines() {
            if let Parsed::Usage(entry) = parser.feed(line) {
                if self.is_new(&entry) {
                    self.tokens += entry.input_tokens + entry.output_tokens;
                    fresh.push(*entry);
                }
            }
        }
        fresh
    }

    pub fn add_cost(&mut self, cost: Option<f64>) {
        match cost {
            Some(usd) => self.usd += usd,
            None => self.unpriced = true,
        }
    }
}

pub fn read_from(path: &Path, from: u64) -> std::io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let start = if from > len { 0 } else { from };
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.take(MAX_CHUNK).read_to_end(&mut buf)?;
    let read = buf.len() as u64;
    let whole = buf.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    buf.truncate(whole);
    let next = if whole == 0 && read >= MAX_CHUNK {
        start + read
    } else {
        start + whole as u64
    };
    Ok((String::from_utf8_lossy(&buf).into_owned(), next))
}

pub fn ended_in_refusal(text: &str) -> bool {
    text.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .find(|v| v["type"] == "assistant")
        .is_some_and(|v| v["message"]["stop_reason"] == "refusal")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(id: &str, stop: &str, input: u64, output: u64) -> String {
        serde_json::json!({
            "type": "assistant", "requestId": "r", "sessionId": "s",
            "message": {
                "id": id, "model": "claude-sonnet-4-5", "stop_reason": stop,
                "usage": {"input_tokens": input, "output_tokens": output}
            }
        })
        .to_string()
    }

    #[test]
    fn each_answer_is_counted_once_across_reads() {
        let mut tail = Tail::default();
        let first = format!(
            "{}\n{}\n",
            assistant("m1", "end_turn", 10, 5),
            assistant("m2", "end_turn", 1, 1)
        );
        assert_eq!(tail.take(&first, "s").len(), 2);
        assert_eq!(tail.tokens, 17);
        let cached = serde_json::json!({
            "type": "assistant", "requestId": "r", "sessionId": "s",
            "message": {"id": "c1", "model": "claude-sonnet-4-5", "usage": {
                "input_tokens": 1, "cache_read_input_tokens": 100, "output_tokens": 2
            }}
        })
        .to_string();
        let mut other = Tail::default();
        other.take(&cached, "s");
        assert_eq!(other.tokens, 103);
        let again = format!(
            "{}\n{}\n",
            assistant("m2", "end_turn", 1, 1),
            assistant("m3", "end_turn", 2, 2)
        );
        assert_eq!(tail.take(&again, "s").len(), 1);
        assert_eq!(tail.tokens, 21);
    }

    #[test]
    fn another_transcript_starts_the_count_again() {
        let mut tail = Tail::default().for_path(Path::new("/a.jsonl"));
        tail.offset = 900;
        tail.usd = 2.0;
        let same = tail.clone().for_path(Path::new("/a.jsonl"));
        assert_eq!((same.offset, same.usd), (900, 2.0));
        let fresh = tail.for_path(Path::new("/b.jsonl"));
        assert_eq!((fresh.offset, fresh.usd), (0, 0.0));
    }

    #[test]
    fn a_model_without_a_price_keeps_the_tokens_and_flags_the_total() {
        let mut tail = Tail::default();
        tail.add_cost(Some(0.5));
        tail.add_cost(None);
        assert_eq!(tail.usd, 0.5);
        assert!(tail.unpriced);
    }

    #[test]
    fn only_the_last_answer_decides_a_refusal() {
        let refused = format!(
            "{}\n{}\n",
            assistant("a", "end_turn", 1, 1),
            assistant("b", "refusal", 1, 1)
        );
        assert!(ended_in_refusal(&refused));
        let recovered = format!(
            "{}\n{}\n{{\"type\":\"user\"}}\n",
            assistant("a", "refusal", 1, 1),
            assistant("b", "end_turn", 1, 1)
        );
        assert!(!ended_in_refusal(&recovered));
        assert!(!ended_in_refusal("not json\n"));
    }

    #[test]
    fn a_read_stops_at_the_last_whole_line_and_resumes_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, "one\ntwo\npart").unwrap();
        let (text, next) = read_from(&path, 0).unwrap();
        assert_eq!(text, "one\ntwo\n");
        assert_eq!(next, 8);
        std::fs::write(&path, "one\ntwo\npartial done\n").unwrap();
        let (text, next) = read_from(&path, next).unwrap();
        assert_eq!(text, "partial done\n");
        assert_eq!(next, 21);
        let (text, _) = read_from(&path, 9_999).unwrap();
        assert!(text.starts_with("one"));
    }
}
