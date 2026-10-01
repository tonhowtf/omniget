use serde_json::Value;

use super::error::{LlmError, ERR_LLM_PARSE};

pub const MAX_LINE_BYTES: usize = 1024 * 1024;
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Default)]
pub struct LineSplitter {
    buf: Vec<u8>,
    pending_lf: bool,
    dropping: bool,
    overflowed: bool,
}

impl LineSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take_overflow(&mut self) -> bool {
        std::mem::take(&mut self.overflowed)
    }

    fn emit_line(&mut self, tail: &[u8], on_line: &mut impl FnMut(&[u8])) {
        if self.dropping {
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
                self.overflowed = true;
                self.dropping = true;
                self.buf.clear();
            } else {
                self.buf.extend_from_slice(tail);
            }
        }
    }

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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
    pub retry: Option<u64>,
}

impl SseEvent {
    pub fn is_done(&self) -> bool {
        self.data.trim() == "[DONE]"
    }

    pub fn json(&self) -> Option<Value> {
        serde_json::from_str(&self.data).ok()
    }
}

#[derive(Debug, Default)]
pub struct SseParser {
    lines: LineSplitter,
    data: String,
    event: Option<String>,
    last_id: Option<String>,
    retry: Option<u64>,
    saw_data: bool,
    dropping: bool,
    overflowed: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<SseEvent>) {
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
                return;
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

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        let mut out = Vec::new();
        self.push(bytes, &mut out);
        out
    }
}

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
