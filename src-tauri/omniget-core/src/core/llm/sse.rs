//! Incremental Server-Sent Events parser (bytes in, events out), plus the
//! newline-delimited JSON splitter Ollama's native `/api/chat` needs.
//! Owned by f2-llm-providers.
//!
//! Design constraints (prompt §7): the body is never materialised as one
//! `String`. Callers push the byte chunks `reqwest` hands them; the parser
//! keeps only the tail of the current incomplete line plus the data buffer of
//! the event being assembled. Line terminators follow the WHATWG SSE spec:
//! LF, CR or CRLF, with CRLF allowed to straddle two chunks.

use serde_json::Value;

use super::error::{LlmError, ERR_LLM_PARSE};

/// Hard ceiling for one line and for one event's `data` buffer. A hostile or
/// broken server must not be able to grow our memory without bound: past this
/// the line/event is dropped and the parser reports `ERR_LLM_PARSE`.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;

/// Splits a byte stream into lines across chunk boundaries.
#[derive(Debug, Default)]
pub struct LineSplitter {
    /// Tail of the line that the last chunk left unfinished.
    buf: Vec<u8>,
    /// Last chunk ended on a bare CR: a leading LF in the next chunk is part
    /// of the same CRLF terminator and must not open an empty line.
    pending_lf: bool,
    /// A line blew the ceiling: swallow its bytes until the next terminator.
    dropping: bool,
    /// Sticky until read by `take_overflow`.
    overflowed: bool,
}

impl LineSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` once (and then cleared) if a line was dropped for being longer
    /// than `MAX_LINE_BYTES`.
    pub fn take_overflow(&mut self) -> bool {
        std::mem::take(&mut self.overflowed)
    }

    fn emit_line(&mut self, tail: &[u8], on_line: &mut impl FnMut(&[u8])) {
        if self.dropping {
            // The line that just ended was already over the ceiling.
            self.dropping = false;
            self.buf.clear();
            return;
        }
        if self.buf.len() + tail.len() > MAX_LINE_BYTES {
            self.overflowed = true;
            self.buf.clear();
            return;
        }
        if self.buf.is_empty() {
            on_line(tail);
        } else {
            self.buf.extend_from_slice(tail);
            on_line(&self.buf);
            self.buf.clear();
        }
    }

    pub fn push(&mut self, bytes: &[u8], mut on_line: impl FnMut(&[u8])) {
        let mut i = 0usize;
        if self.pending_lf {
            self.pending_lf = false;
            if bytes.first() == Some(&b'\n') {
                i = 1;
            }
        }
        let mut start = i;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => {
                    self.emit_line(&bytes[start..i], &mut on_line);
                    i += 1;
                    start = i;
                }
                b'\r' => {
                    self.emit_line(&bytes[start..i], &mut on_line);
                    i += 1;
                    if i < bytes.len() {
                        if bytes[i] == b'\n' {
                            i += 1;
                        }
                    } else {
                        self.pending_lf = true;
                    }
                    start = i;
                }
                _ => i += 1,
            }
        }
        if start < bytes.len() {
            let tail = &bytes[start..];
            if self.dropping {
                return;
            }
            if self.buf.len() + tail.len() > MAX_LINE_BYTES {
                // Stop buffering now instead of at the terminator, so a line
                // with no terminator at all cannot grow forever either.
                self.overflowed = true;
                self.dropping = true;
                self.buf.clear();
            } else {
                self.buf.extend_from_slice(tail);
            }
        }
    }

    /// Flushes a trailing line that the stream ended without terminating.
    pub fn finish(&mut self, mut on_line: impl FnMut(&[u8])) {
        if self.dropping {
            self.dropping = false;
            self.buf.clear();
        } else if !self.buf.is_empty() {
            on_line(&self.buf);
            self.buf.clear();
        }
        self.pending_lf = false;
    }
}

/// One dispatched SSE event.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SseEvent {
    /// `event:` field; `None` when the stream only sends `data:`
    /// (OpenAI-compatible providers) — the spec default is `message`.
    pub event: Option<String>,
    /// Concatenated `data:` lines, newline-joined, without the trailing LF.
    pub data: String,
    /// Last `id:` seen, which the spec keeps sticky across events.
    pub id: Option<String>,
    pub retry: Option<u64>,
}

impl SseEvent {
    /// OpenAI-compatible terminator.
    pub fn is_done(&self) -> bool {
        self.data.trim() == "[DONE]"
    }

    pub fn json(&self) -> Option<Value> {
        serde_json::from_str(&self.data).ok()
    }
}

/// Incremental SSE parser. Feed it chunks; it appends dispatched events to the
/// caller's buffer so a hot stream allocates no `Vec` per chunk.
#[derive(Debug, Default)]
pub struct SseParser {
    lines: LineSplitter,
    data: String,
    event: Option<String>,
    last_id: Option<String>,
    retry: Option<u64>,
    saw_data: bool,
    /// The event being assembled blew `MAX_EVENT_BYTES`: drop it whole.
    dropping: bool,
    /// Sticky until read by `take_overflow`.
    overflowed: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<SseEvent>) {
        // Fields are moved out of `self` so the closure can borrow them while
        // `self.lines` is mutably borrowed by `push`.
        let data = &mut self.data;
        let event = &mut self.event;
        let last_id = &mut self.last_id;
        let retry = &mut self.retry;
        let saw_data = &mut self.saw_data;
        let dropping = &mut self.dropping;
        let overflowed = &mut self.overflowed;
        self.lines.push(bytes, |line| {
            if line.is_empty() {
                if *dropping {
                    // The event was over the ceiling: swallow it entirely.
                    *dropping = false;
                    *saw_data = false;
                    let _ = event.take();
                    data.clear();
                    return;
                }
                if *saw_data || event.is_some() {
                    if data.ends_with('\n') {
                        data.pop();
                    }
                    out.push(SseEvent {
                        event: event.take(),
                        data: std::mem::take(data),
                        id: last_id.clone(),
                        retry: *retry,
                    });
                }
                data.clear();
                *saw_data = false;
                return;
            }
            if line[0] == b':' {
                return; // comment / keep-alive
            }
            let (field, value) = match line.iter().position(|b| *b == b':') {
                Some(pos) => {
                    let mut v = &line[pos + 1..];
                    if v.first() == Some(&b' ') {
                        v = &v[1..];
                    }
                    (&line[..pos], v)
                }
                None => (line, &line[line.len()..]),
            };
            if *dropping {
                return;
            }
            match field {
                b"data" => {
                    if data.len() + value.len() + 1 > MAX_EVENT_BYTES {
                        *overflowed = true;
                        *dropping = true;
                        data.clear();
                        return;
                    }
                    data.push_str(&String::from_utf8_lossy(value));
                    data.push('\n');
                    *saw_data = true;
                }
                b"event" => *event = Some(String::from_utf8_lossy(value).into_owned()),
                b"id" => {
                    if !value.contains(&0) {
                        *last_id = Some(String::from_utf8_lossy(value).into_owned());
                    }
                }
                b"retry" => {
                    if let Ok(ms) = String::from_utf8_lossy(value).parse::<u64>() {
                        *retry = Some(ms);
                    }
                }
                _ => {}
            }
        });
    }

    /// Dispatches whatever a truncated stream left behind. Some servers close
    /// right after the last `data:` line without the blank line.
    pub fn finish(&mut self, out: &mut Vec<SseEvent>) {
        let mut tail: Vec<u8> = Vec::new();
        self.lines.finish(|line| tail.extend_from_slice(line));
        if !tail.is_empty() {
            tail.push(b'\n');
            tail.push(b'\n');
            self.push(&tail, out);
        } else if self.saw_data {
            self.push(b"\n", out);
        }
    }

    /// The parse error owed to the caller when a line or an event was dropped
    /// for blowing its ceiling. Reading it clears the flag, so a provider that
    /// checks after every `push` sees each overflow exactly once.
    pub fn take_overflow(&mut self) -> Option<LlmError> {
        let line = self.lines.take_overflow();
        let event = std::mem::take(&mut self.overflowed);
        if line || event {
            return Some(LlmError::new(
                ERR_LLM_PARSE,
                format!(
                    "SSE {} over the {} byte ceiling: dropped",
                    if line { "line" } else { "event" },
                    if line {
                        MAX_LINE_BYTES
                    } else {
                        MAX_EVENT_BYTES
                    }
                ),
            ));
        }
        None
    }

    /// Convenience for tests and one-shot bodies.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        let mut out = Vec::new();
        self.push(bytes, &mut out);
        out
    }
}

/// Newline-delimited JSON, which Ollama's native `/api/chat` speaks instead of
/// SSE (`Content-Type: application/x-ndjson`, one object per line, no `data:`).
#[derive(Debug, Default)]
pub struct NdjsonParser {
    lines: LineSplitter,
}

impl NdjsonParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<Value>) {
        self.lines.push(bytes, |line| {
            if line.iter().all(|b| b.is_ascii_whitespace()) {
                return;
            }
            if let Ok(v) = serde_json::from_slice::<Value>(line) {
                out.push(v);
            }
        });
    }

    pub fn finish(&mut self, out: &mut Vec<Value>) {
        let mut tail: Vec<u8> = Vec::new();
        self.lines.finish(|line| tail.extend_from_slice(line));
        if !tail.is_empty() {
            if let Ok(v) = serde_json::from_slice::<Value>(&tail) {
                out.push(v);
            }
        }
    }
}
