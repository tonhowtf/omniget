use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

pub const CAPACITY: usize = 500;

pub fn redact(line: &str) -> String {
    redact_keyed(&redact_urls_in_text(line))
}

const REDACTED: &str = "[REDACTED]";

const SAFE_QUERY_KEYS: &[&str] = &[
    "v",
    "list",
    "t",
    "index",
    "start",
    "end",
    "time_continue",
    "id",
    "p",
    "page",
    "vid",
    "video_id",
    "playlist",
    "pl",
    "item_id",
    "bvid",
    "aid",
    "cid",
    "ep_id",
    "season_id",
    "story_fbid",
    "fbid",
    "hl",
    "lang",
];

const SAFE_VALUE_MAX: usize = 64;

pub fn redact_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let Ok(parsed) = url::Url::parse(trimmed) else {
        return redact_non_url(trimmed);
    };
    if parsed.cannot_be_a_base() {
        return redact_non_url(trimmed);
    }
    let mut out = String::with_capacity(trimmed.len());
    out.push_str(parsed.scheme());
    out.push_str("://");
    if !parsed.username().is_empty() || parsed.password().is_some() {
        out.push_str(REDACTED);
        out.push('@');
    }
    if let Some(host) = parsed.host_str() {
        out.push_str(host);
    }
    if let Some(port) = parsed.port() {
        out.push(':');
        out.push_str(&port.to_string());
    }
    let path = parsed.path();
    let segments: Vec<String> = path
        .split('/')
        .map(|seg| {
            if segment_looks_like_token(seg) {
                REDACTED.to_string()
            } else {
                seg.to_string()
            }
        })
        .collect();
    out.push_str(&segments.join("/"));
    if let Some(query) = parsed.query() {
        out.push('?');
        out.push_str(&redact_query(query));
    }
    if parsed.fragment().is_some_and(|f| !f.is_empty()) {
        out.push('#');
        out.push_str(REDACTED);
    }
    out
}

fn redact_non_url(raw: &str) -> String {
    let cut = raw.find(['?', '#']).unwrap_or(raw.len());
    let mut out = redact_keyed(&raw[..cut]);
    if cut < raw.len() {
        out.push(raw.as_bytes()[cut] as char);
        out.push_str(REDACTED);
    }
    out
}

fn redact_query(query: &str) -> String {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (raw_key, value) = match pair.split_once('=') {
                Some((k, v)) => (k, Some(v)),
                None => (pair, None),
            };
            let key = urlencoding::decode(raw_key)
                .map(|k| k.into_owned())
                .unwrap_or_else(|_| raw_key.to_string());
            let key_is_plain = !key.is_empty()
                && key.len() <= 40
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '[' | ']'));
            if !key_is_plain {
                return REDACTED.to_string();
            }
            let safe = SAFE_QUERY_KEYS.iter().any(|s| s.eq_ignore_ascii_case(&key));
            match value {
                Some(v) if safe && v.len() <= SAFE_VALUE_MAX && !segment_looks_like_token(v) => {
                    format!("{raw_key}={v}")
                }
                Some(_) => format!("{raw_key}={REDACTED}"),
                None if safe => raw_key.to_string(),
                None => REDACTED.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn segment_looks_like_token(seg: &str) -> bool {
    let decoded = urlencoding::decode(seg)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| seg.to_string());
    if decoded.starts_with("eyJ") && decoded.len() >= 20 {
        return true;
    }
    let mut run = 0usize;
    let mut has_digit = false;
    let mut has_alpha = false;
    for c in decoded.chars() {
        if c.is_ascii_alphanumeric() {
            run += 1;
            has_digit |= c.is_ascii_digit();
            has_alpha |= c.is_ascii_alphabetic();
            if run >= 24 && has_digit && has_alpha {
                return true;
            }
        } else {
            run = 0;
            has_digit = false;
            has_alpha = false;
        }
    }
    false
}

pub fn is_redacted_url(url: &str) -> bool {
    url.contains(REDACTED)
}

pub fn redact_urls(text: &str) -> String {
    redact_urls_in_text(text)
}

fn redact_urls_in_text(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    loop {
        let next = [
            lower[cursor..].find("https://"),
            lower[cursor..].find("http://"),
        ]
        .into_iter()
        .flatten()
        .min();
        let Some(rel) = next else { break };
        let start = cursor + rel;
        let mut end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`'))
            .map(|n| start + n)
            .unwrap_or(text.len());
        while end > start
            && !text[start..end].ends_with(REDACTED)
            && matches!(
                text.as_bytes()[end - 1],
                b'.' | b',' | b';' | b':' | b')' | b']' | b'}'
            )
        {
            end -= 1;
        }
        out.push_str(&text[cursor..start]);
        out.push_str(&redact_url(&text[start..end]));
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

fn redact_keyed(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        match next_secret(rest) {
            Some((start, len, replacement)) => {
                out.push_str(&rest[..start]);
                out.push_str(replacement);
                rest = &rest[start + len..];
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    redact_home_paths(&out)
}

fn next_secret(hay: &str) -> Option<(usize, usize, &'static str)> {
    let lower = hay.to_ascii_lowercase();
    let mut best: Option<(usize, usize, &'static str)> = None;

    const KEYED: &[&str] = &[
        "cookie=",
        "cookies=",
        "token=",
        "access_token=",
        "refresh_token=",
        "api_key=",
        "apikey=",
        "key=",
        "password=",
        "passwd=",
        "secret=",
        "authorization=",
        "sessdata=",
        "bili_jct=",
        "auth=",
        "sig=",
        "signature=",
        "x-amz-credential=",
        "x-amz-security-token=",
        "x-goog-credential=",
        "policy=",
        "po_token=",
    ];
    for k in KEYED {
        if let Some(i) = lower.find(k) {
            let value_start = i + k.len();
            let value_len = hay[value_start..]
                .find(|c: char| c.is_whitespace() || c == '&' || c == ';' || c == '"' || c == '\'')
                .unwrap_or(hay.len() - value_start);
            if value_len > 0 {
                let cand = (value_start, value_len, "[REDACTED]");
                if best.is_none_or(|(b, _, _)| cand.0 < b) {
                    best = Some(cand);
                }
            }
        }
    }

    for key in [
        "cookie:",
        "set-cookie:",
        "authorization:",
        "proxy-authorization:",
    ] {
        if let Some(i) = lower.find(key) {
            let start = i + key.len();
            let len = hay[start..].find(['\r', '\n']).unwrap_or(hay.len() - start);
            if len > 0 && best.is_none_or(|(b, _, _)| start < b) {
                best = Some((start, len, "[REDACTED]"));
            }
        }
    }
    for key in [
        "--password ",
        "--username ",
        "--proxy ",
        "--cookies ",
        "--add-header ",
    ] {
        if let Some(i) = lower.find(key) {
            let start = i + key.len();
            if start < hay.len() && best.is_none_or(|(b, _, _)| start < b) {
                best = Some((start, hay.len() - start, "[REDACTED]"));
            }
        }
    }
    for (i, _) in lower.match_indices("://") {
        let start = i + 3;
        let end = hay[start..]
            .find(|c: char| c == '/' || c.is_whitespace())
            .map(|n| start + n)
            .unwrap_or(hay.len());
        if let Some(at) = hay[start..end].rfind('@') {
            if best.is_none_or(|(b, _, _)| start < b) {
                best = Some((start, at + 1, "[REDACTED]@"));
            }
        }
    }

    if let Some(i) = lower.find("bearer ") {
        let value_start = i + "bearer ".len();
        let value_len = hay[value_start..]
            .find(char::is_whitespace)
            .unwrap_or(hay.len() - value_start);
        if value_len > 0 {
            let cand = (value_start, value_len, "[REDACTED]");
            if best.is_none_or(|(b, _, _)| cand.0 < b) {
                best = Some(cand);
            }
        }
    }

    best
}

fn redact_home_paths(line: &str) -> String {
    let mut out = line.to_string();
    for prefix in ["/Users/", "/home/", "\\Users\\"] {
        while let Some(i) = out.find(prefix) {
            let after = i + prefix.len();
            let sep = if prefix.contains('\\') { '\\' } else { '/' };
            let end = out[after..]
                .find(|c: char| c == sep || c.is_whitespace())
                .map(|e| after + e)
                .unwrap_or(out.len());
            if end == after {
                break;
            }
            out.replace_range(i..end, "~");
        }
    }
    out
}

#[derive(Debug)]
pub struct Recorder {
    events: VecDeque<String>,
    capacity: usize,
}

impl Recorder {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity.min(1024)),
            capacity: capacity.max(1),
        }
    }

    pub fn record(&mut self, event: &str) {
        if self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(redact(event));
    }

    pub fn dump(&self) -> Vec<String> {
        self.events.iter().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

static BUFFER: OnceLock<Mutex<Recorder>> = OnceLock::new();

fn buffer() -> &'static Mutex<Recorder> {
    BUFFER.get_or_init(|| Mutex::new(Recorder::new(CAPACITY)))
}

pub fn record(event: &str) {
    if let Ok(mut buf) = buffer().lock() {
        buf.record(event);
    }
}

pub fn dump() -> Vec<String> {
    buffer().lock().map(|b| b.dump()).unwrap_or_default()
}

pub fn clear() {
    if let Ok(mut buf) = buffer().lock() {
        buf.clear();
    }
}
