//! TOML: a reader for the subset real configs use (tables, arrays of tables,
//! dotted keys, every string form, numbers, booleans, dates kept as strings,
//! arrays, inline tables) and a line-preserving editor for Codex, Grok, Vibe and
//! Kimi configs. It also loads the embedded target manifests.

use serde_json::{Map, Value};

use super::{edit_err, ordered_entries, parse_err, value_at, Editor, Result, Seg, SetOutcome};

/// Parses a TOML document into a JSON value.
pub fn parse(text: &str) -> Result<Value> {
    Ok(scan(text)?.value)
}

/// Where each header and key lives, for the editor.
#[derive(Debug, Clone)]
struct Section {
    /// Value path of the table (`[a.b]` → a.b; `[[x]]` → x[i]); root = empty.
    path: Vec<Seg>,
    /// Byte offset of the header line start (root: 0).
    start: usize,
    /// Byte offset after the header line (root: 0).
    body_start: usize,
    /// Offset where the section ends (next header line start or EOF).
    end: usize,
    is_root: bool,
    #[allow(dead_code)]
    array_item: bool,
    /// Key-value entries in this section.
    keys: Vec<KeyEntry>,
}

#[derive(Debug, Clone)]
struct KeyEntry {
    /// Full value path.
    path: Vec<Seg>,
    line_start: usize,
    value_start: usize,
    value_end: usize,
    /// End of the line including the newline.
    line_end: usize,
}

struct Scan {
    value: Value,
    sections: Vec<Section>,
}

fn scan(text: &str) -> Result<Scan> {
    let mut p = P {
        s: text.as_bytes(),
        text,
        pos: 0,
    };
    let mut root = Value::Object(Map::new());
    let mut sections = vec![Section {
        path: vec![],
        start: 0,
        body_start: 0,
        end: text.len(),
        is_root: true,
        array_item: false,
        keys: vec![],
    }];
    let mut current: Vec<Seg> = vec![];
    if text.starts_with('\u{feff}') {
        p.pos = 3;
    }
    loop {
        p.skip_ws_and_newlines_and_comments();
        if p.pos >= p.s.len() {
            break;
        }
        let line_start = p.line_start();
        if p.s[p.pos] == b'[' {
            let aot = p.s.get(p.pos + 1) == Some(&b'[');
            p.pos += if aot { 2 } else { 1 };
            p.skip_inline_ws();
            let key = p.key_path()?;
            p.skip_inline_ws();
            if aot {
                p.expect(b']')?;
                p.expect(b']')?;
            } else {
                p.expect(b']')?;
            }
            p.skip_inline_ws();
            p.skip_comment();
            p.expect_eol()?;
            let body_start = p.pos;
            if let Some(last) = sections.last_mut() {
                last.end = line_start;
            }
            let path = if aot {
                let arr = ensure_path(&mut root, &key, true)
                    .map_err(|m| parse_err("toml", super::line_of(text, line_start), m))?;
                let arr = arr.as_array_mut().ok_or_else(|| {
                    parse_err(
                        "toml",
                        super::line_of(text, line_start),
                        "not an array of tables",
                    )
                })?;
                arr.push(Value::Object(Map::new()));
                let mut path: Vec<Seg> = resolve(&root, &key);
                let idx = value_at(&root, &path)
                    .and_then(|v| v.as_array())
                    .map(|a| a.len() - 1)
                    .unwrap_or(0);
                path.push(Seg::Index(idx));
                path
            } else {
                ensure_path(&mut root, &key, false)
                    .map_err(|m| parse_err("toml", super::line_of(text, line_start), m))?;
                resolve(&root, &key)
            };
            current = path.clone();
            sections.push(Section {
                path,
                start: line_start,
                body_start,
                end: text.len(),
                is_root: false,
                array_item: aot,
                keys: vec![],
            });
            continue;
        }
        // key = value
        let key = p.key_path()?;
        p.skip_inline_ws();
        p.expect(b'=')?;
        p.skip_inline_ws();
        let value_start = p.pos;
        let value = p.value(0)?;
        let value_end = p.pos;
        p.skip_inline_ws();
        p.skip_comment();
        p.expect_eol()?;
        let line_end = p.pos;
        let (last, parents) = key.split_last().expect("key path is never empty");
        let mut full = current.clone();
        {
            let table = get_mut(&mut root, &current).expect("current table exists");
            let table = ensure_path(table, parents, false)
                .map_err(|m| parse_err("toml", super::line_of(text, line_start), m))?;
            let map = table.as_object_mut().ok_or_else(|| {
                parse_err("toml", super::line_of(text, line_start), "not a table")
            })?;
            if map.contains_key(last) {
                return Err(parse_err(
                    "toml",
                    super::line_of(text, line_start),
                    format!("duplicate key `{last}`"),
                ));
            }
            map.insert(last.clone(), value);
        }
        full.extend(key.iter().map(|k| Seg::Key(k.clone())));
        sections.last_mut().unwrap().keys.push(KeyEntry {
            path: full,
            line_start,
            value_start,
            value_end,
            line_end,
        });
    }
    Ok(Scan {
        value: root,
        sections,
    })
}

fn get_mut<'a>(v: &'a mut Value, path: &[Seg]) -> Option<&'a mut Value> {
    let mut cur = v;
    for seg in path {
        cur = match (seg, cur) {
            (Seg::Key(k), Value::Object(m)) => m.get_mut(k)?,
            (Seg::Index(i), Value::Array(a)) => a.get_mut(*i)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Walks key parts from `v`, creating tables; stepping into the last element of
/// arrays of tables. When `want_array` the final node is created as an array.
fn ensure_path<'a>(
    v: &'a mut Value,
    keys: &[String],
    want_array: bool,
) -> std::result::Result<&'a mut Value, String> {
    let mut cur = v;
    for (i, k) in keys.iter().enumerate() {
        let last = i + 1 == keys.len();
        let map = cur
            .as_object_mut()
            .ok_or_else(|| format!("`{k}` is under a non-table"))?;
        let entry = map.entry(k.clone()).or_insert_with(|| {
            if last && want_array {
                Value::Array(vec![])
            } else {
                Value::Object(Map::new())
            }
        });
        let step_into_last = entry.is_array() && !(last && want_array);
        cur = if step_into_last {
            entry
                .as_array_mut()
                .and_then(|a| a.last_mut())
                .ok_or_else(|| format!("`{k}` is an empty array"))?
        } else {
            entry
        };
    }
    Ok(cur)
}

/// Value path of a header key, going through the last element of arrays of tables.
fn resolve(root: &Value, keys: &[String]) -> Vec<Seg> {
    let mut out = vec![];
    let mut cur = root;
    for (i, k) in keys.iter().enumerate() {
        out.push(Seg::Key(k.clone()));
        let Some(next) = cur.get(k) else { break };
        cur = next;
        if i + 1 < keys.len() {
            if let Value::Array(a) = cur {
                if let Some(last) = a.last() {
                    out.push(Seg::Index(a.len() - 1));
                    cur = last;
                }
            }
        }
    }
    out
}

struct P<'a> {
    s: &'a [u8],
    text: &'a str,
    pos: usize,
}

impl<'a> P<'a> {
    fn err(&self, msg: impl std::fmt::Display) -> super::super::AgentkitError {
        parse_err("toml", super::line_of(self.text, self.pos), msg)
    }

    fn line_start(&self) -> usize {
        self.text[..self.pos]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    fn skip_inline_ws(&mut self) {
        while self.pos < self.s.len() && (self.s[self.pos] == b' ' || self.s[self.pos] == b'\t') {
            self.pos += 1;
        }
    }

    fn skip_comment(&mut self) {
        if self.s.get(self.pos) == Some(&b'#') {
            while self.pos < self.s.len() && self.s[self.pos] != b'\n' {
                self.pos += 1;
            }
        }
    }

    fn skip_ws_and_newlines_and_comments(&mut self) {
        loop {
            while self.pos < self.s.len() && (self.s[self.pos] as char).is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.s.get(self.pos) == Some(&b'#') {
                self.skip_comment();
                continue;
            }
            break;
        }
    }

    fn expect(&mut self, b: u8) -> Result<()> {
        if self.s.get(self.pos) == Some(&b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(format!("expected `{}`", b as char)))
        }
    }

    fn expect_eol(&mut self) -> Result<()> {
        match self.s.get(self.pos) {
            None => Ok(()),
            Some(b'\n') => {
                self.pos += 1;
                Ok(())
            }
            Some(b'\r') if self.s.get(self.pos + 1) == Some(&b'\n') => {
                self.pos += 2;
                Ok(())
            }
            _ => Err(self.err("expected end of line")),
        }
    }

    fn key_path(&mut self) -> Result<Vec<String>> {
        let mut parts = vec![self.simple_key()?];
        loop {
            self.skip_inline_ws();
            if self.s.get(self.pos) == Some(&b'.') {
                self.pos += 1;
                self.skip_inline_ws();
                parts.push(self.simple_key()?);
            } else {
                return Ok(parts);
            }
        }
    }

    fn simple_key(&mut self) -> Result<String> {
        match self.s.get(self.pos) {
            Some(b'"') => self.basic_string(),
            Some(b'\'') => self.literal_string(),
            _ => {
                let start = self.pos;
                while self.pos < self.s.len() {
                    let c = self.s[self.pos];
                    if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                if start == self.pos {
                    return Err(self.err("expected a key"));
                }
                Ok(self.text[start..self.pos].to_string())
            }
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > 128 {
            return Err(self.err("nesting too deep"));
        }
        match self.s.get(self.pos) {
            None => Err(self.err("expected a value")),
            Some(b'"') => {
                if self.s[self.pos..].starts_with(b"\"\"\"") {
                    self.ml_basic_string().map(Value::String)
                } else {
                    self.basic_string().map(Value::String)
                }
            }
            Some(b'\'') => {
                if self.s[self.pos..].starts_with(b"'''") {
                    self.ml_literal_string().map(Value::String)
                } else {
                    self.literal_string().map(Value::String)
                }
            }
            Some(b'[') => {
                self.pos += 1;
                let mut items = vec![];
                loop {
                    self.skip_ws_and_newlines_and_comments();
                    if self.s.get(self.pos) == Some(&b']') {
                        self.pos += 1;
                        break;
                    }
                    items.push(self.value(depth + 1)?);
                    self.skip_ws_and_newlines_and_comments();
                    match self.s.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {}
                        _ => return Err(self.err("expected `,` or `]` in array")),
                    }
                }
                Ok(Value::Array(items))
            }
            Some(b'{') => {
                self.pos += 1;
                let mut table = Value::Object(Map::new());
                loop {
                    self.skip_ws_and_newlines_and_comments();
                    if self.s.get(self.pos) == Some(&b'}') {
                        self.pos += 1;
                        break;
                    }
                    let key = self.key_path()?;
                    self.skip_inline_ws();
                    self.expect(b'=')?;
                    self.skip_inline_ws();
                    let v = self.value(depth + 1)?;
                    let (last, parents) = key.split_last().unwrap();
                    let t = ensure_path(&mut table, parents, false).map_err(|m| self.err(m))?;
                    t.as_object_mut()
                        .ok_or_else(|| self.err("not a table"))?
                        .insert(last.clone(), v);
                    self.skip_ws_and_newlines_and_comments();
                    match self.s.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {}
                        _ => return Err(self.err("expected `,` or `}` in inline table")),
                    }
                }
                Ok(table)
            }
            Some(_) => {
                let start = self.pos;
                while self.pos < self.s.len() {
                    let c = self.s[self.pos];
                    if c == b',' || c == b']' || c == b'}' || c == b'#' || c == b'\n' || c == b'\r'
                    {
                        break;
                    }
                    // a date-time may contain one space between date and time
                    if c == b' ' || c == b'\t' {
                        let rest = &self.text[self.pos + 1..];
                        let looks_time = rest.len() >= 2
                            && rest.as_bytes()[0].is_ascii_digit()
                            && rest.as_bytes()[1].is_ascii_digit()
                            && self.text[start..self.pos].len() == 10
                            && self.text[start..self.pos].contains('-');
                        if !looks_time {
                            break;
                        }
                    }
                    self.pos += 1;
                }
                let tok = self.text[start..self.pos].trim();
                scalar(tok).ok_or_else(|| self.err(format!("bad value `{tok}`")))
            }
        }
    }

    fn basic_string(&mut self) -> Result<String> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(&c) = self.s.get(self.pos) else {
                return Err(self.err("unterminated string"));
            };
            match c {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    self.escape(&mut out)?;
                }
                b'\n' => return Err(self.err("newline in string")),
                _ => {
                    let ch = self.text[self.pos..].chars().next().unwrap();
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> Result<()> {
        let Some(&e) = self.s.get(self.pos) else {
            return Err(self.err("bad escape"));
        };
        self.pos += 1;
        match e {
            b'n' => out.push('\n'),
            b't' => out.push('\t'),
            b'r' => out.push('\r'),
            b'b' => out.push('\u{8}'),
            b'f' => out.push('\u{c}'),
            b'e' => out.push('\u{1b}'),
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'u' | b'U' => {
                let n = if e == b'u' { 4 } else { 8 };
                let hex = self
                    .text
                    .get(self.pos..self.pos + n)
                    .ok_or_else(|| self.err("bad unicode escape"))?;
                let cp =
                    u32::from_str_radix(hex, 16).map_err(|_| self.err("bad unicode escape"))?;
                out.push(char::from_u32(cp).ok_or_else(|| self.err("bad unicode escape"))?);
                self.pos += n;
            }
            _ => return Err(self.err(format!("bad escape `\\{}`", e as char))),
        }
        Ok(())
    }

    fn literal_string(&mut self) -> Result<String> {
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.s.len() && self.s[self.pos] != b'\'' {
            if self.s[self.pos] == b'\n' {
                return Err(self.err("newline in string"));
            }
            self.pos += 1;
        }
        if self.pos >= self.s.len() {
            return Err(self.err("unterminated string"));
        }
        let out = self.text[start..self.pos].to_string();
        self.pos += 1;
        Ok(out)
    }

    fn ml_basic_string(&mut self) -> Result<String> {
        self.pos += 3;
        if self.s.get(self.pos) == Some(&b'\n') {
            self.pos += 1;
        } else if self.s[self.pos..].starts_with(b"\r\n") {
            self.pos += 2;
        }
        let mut out = String::new();
        loop {
            if self.pos >= self.s.len() {
                return Err(self.err("unterminated multi-line string"));
            }
            if self.s[self.pos..].starts_with(b"\"\"\"") {
                // up to two extra quotes belong to the content
                let mut q = 3;
                while self.s.get(self.pos + q) == Some(&b'"') && q < 5 {
                    q += 1;
                }
                for _ in 3..q {
                    out.push('"');
                }
                self.pos += q;
                return Ok(out);
            }
            let c = self.s[self.pos];
            if c == b'\\' {
                // line-ending backslash trims following whitespace
                let mut j = self.pos + 1;
                while j < self.s.len() && (self.s[j] == b' ' || self.s[j] == b'\t') {
                    j += 1;
                }
                if j < self.s.len() && (self.s[j] == b'\n' || self.s[j] == b'\r') {
                    self.pos = j;
                    while self.pos < self.s.len()
                        && (self.s[self.pos] as char).is_ascii_whitespace()
                    {
                        self.pos += 1;
                    }
                    continue;
                }
                self.pos += 1;
                self.escape(&mut out)?;
                continue;
            }
            let ch = self.text[self.pos..].chars().next().unwrap();
            out.push(ch);
            self.pos += ch.len_utf8();
        }
    }

    fn ml_literal_string(&mut self) -> Result<String> {
        self.pos += 3;
        if self.s.get(self.pos) == Some(&b'\n') {
            self.pos += 1;
        } else if self.s[self.pos..].starts_with(b"\r\n") {
            self.pos += 2;
        }
        let rel = self.text[self.pos..]
            .find("'''")
            .ok_or_else(|| self.err("unterminated multi-line string"))?;
        let mut end = self.pos + rel;
        let mut close = 3;
        while self.s.get(end + close) == Some(&b'\'') && close < 5 {
            close += 1;
        }
        end += close - 3;
        let out = self.text[self.pos..end].to_string();
        self.pos = end + 3;
        Ok(out)
    }
}

fn scalar(tok: &str) -> Option<Value> {
    match tok {
        "true" => return Some(Value::Bool(true)),
        "false" => return Some(Value::Bool(false)),
        "inf" | "+inf" | "-inf" | "nan" | "+nan" | "-nan" => {
            return Some(Value::String(tok.to_string()))
        }
        _ => {}
    }
    let clean = tok.replace('_', "");
    if let Some(h) = clean.strip_prefix("0x") {
        return i64::from_str_radix(h, 16).ok().map(Value::from);
    }
    if let Some(o) = clean.strip_prefix("0o") {
        return i64::from_str_radix(o, 8).ok().map(Value::from);
    }
    if let Some(b) = clean.strip_prefix("0b") {
        return i64::from_str_radix(b, 2).ok().map(Value::from);
    }
    if let Ok(i) = clean.parse::<i64>() {
        return Some(Value::from(i));
    }
    if clean.chars().any(|c| c.is_ascii_digit())
        && (clean.contains('.') || clean.contains('e') || clean.contains('E'))
        && !clean.contains(':')
    {
        if let Ok(f) = clean.parse::<f64>() {
            return serde_json::Number::from_f64(f).map(Value::Number);
        }
    }
    // dates, times, date-times: keep as text
    if tok.len() >= 8
        && tok.as_bytes()[0].is_ascii_digit()
        && (tok.contains('-') || tok.contains(':'))
    {
        return Some(Value::String(tok.to_string()));
    }
    None
}

// ---------------------------------------------------------------- writing

fn is_bare_key(k: &str) -> bool {
    !k.is_empty()
        && k.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

pub fn key_text(k: &str) -> String {
    if is_bare_key(k) {
        k.to_string()
    } else {
        basic(k)
    }
}

fn basic(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{:04X}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A value on one line (strings, numbers, arrays, inline tables).
pub fn inline(v: &Value) -> String {
    match v {
        Value::Null => "\"\"".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            if s.contains('\n') {
                // multi-line basic string keeps prompts readable
                let esc = s.replace('\\', "\\\\").replace("\"\"\"", "\\\"\\\"\\\"");
                format!("\"\"\"\n{esc}\"\"\"")
            } else {
                basic(s)
            }
        }
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().map(inline).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(m) => {
            if m.is_empty() {
                return "{}".into();
            }
            let parts: Vec<String> = ordered_entries(m)
                .into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| format!("{} = {}", key_text(k), inline(v)))
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
    }
}

fn header_text(path: &[Seg], aot: bool) -> String {
    let keys: Vec<String> = path
        .iter()
        .filter_map(|s| match s {
            Seg::Key(k) => Some(key_text(k)),
            Seg::Index(_) => None,
        })
        .collect();
    if aot {
        format!("[[{}]]", keys.join("."))
    } else {
        format!("[{}]", keys.join("."))
    }
}

/// Renders a table body: scalars/arrays/objects as `k = inline` lines.
fn body_text(map: &Map<String, Value>) -> String {
    let mut out = String::new();
    for (k, v) in ordered_entries(map) {
        if v.is_null() {
            continue;
        }
        out.push_str(&format!("{} = {}\n", key_text(k), inline(v)));
    }
    out
}

/// A TOML document open for editing.
#[derive(Debug, Clone)]
pub struct TomlDoc {
    text: String,
    value: Value,
    sections: Vec<Section>,
}

impl TomlDoc {
    pub fn parse(text: &str) -> Result<TomlDoc> {
        let s = scan(text)?;
        Ok(TomlDoc {
            text: text.to_string(),
            value: s.value,
            sections: s.sections,
        })
    }

    fn replace(&mut self, start: usize, end: usize, with: &str) -> Result<()> {
        let mut t = self.text.clone();
        t.replace_range(start..end, with);
        *self = TomlDoc::parse(&t)?;
        Ok(())
    }

    fn section_of(&self, path: &[Seg]) -> Option<&Section> {
        self.sections.iter().find(|s| s.path == path)
    }

    fn key_entry(&self, path: &[Seg]) -> Option<&KeyEntry> {
        self.sections
            .iter()
            .flat_map(|s| s.keys.iter())
            .find(|k| k.path == path)
    }

    /// Byte range of a table and all its sub-tables (they must be contiguous).
    fn table_block(&self, path: &[Seg]) -> Option<(usize, usize)> {
        let idx = self
            .sections
            .iter()
            .position(|s| s.path == path && !s.is_root)?;
        let start = self.sections[idx].start;
        let mut end = self.sections[idx].end;
        for s in &self.sections[idx + 1..] {
            if s.path.len() > path.len() && s.path[..path.len()] == *path {
                end = s.end;
            } else {
                break;
            }
        }
        Some((start, end))
    }

    /// Appends text at the end of the file separated by one blank line.
    fn append_block(&mut self, block: &str) -> Result<()> {
        let mut t = self.text.clone();
        if t.is_empty() {
        } else if t.ends_with('\n') {
            t.push('\n');
        } else {
            t.push_str("\n\n");
        }
        t.push_str(block);
        *self = TomlDoc::parse(&t)?;
        Ok(())
    }

    /// Removes a block, and the blank line `append_block` put before it.
    fn remove_block(&mut self, start: usize, end: usize) -> Result<()> {
        let mut s = start;
        let t = &self.text;
        if s > 0
            && t[..s].ends_with("\n\n")
            && (end == t.len() || t[end..].starts_with('\n') || t[end..].starts_with('['))
        {
            s -= 1;
        }
        if s == 1 && &t[..1] == "\n" {
            s = 0;
        }
        self.replace(s, end, "")
    }

    /// Writes `[path]` (or `[[path]]`) with an object value at the end of the file.
    fn append_table(&mut self, path: &[Seg], obj: &Map<String, Value>, aot: bool) -> Result<()> {
        let mut block = format!("{}\n", header_text(path, aot));
        block.push_str(&body_text(obj));
        self.append_block(&block)
    }

    /// Inserts `key = value` as the last key line of the section at `table`.
    fn insert_key(&mut self, table: &[Seg], key: &str, v: &Value) -> Result<()> {
        let line = format!("{} = {}\n", key_text(key), inline(v));
        match self.section_of(table).cloned() {
            Some(sec) => {
                let at = sec
                    .keys
                    .last()
                    .map(|k| k.line_end)
                    .unwrap_or(sec.body_start);
                let mut t = self.text.clone();
                let mut line = line;
                if at > 0 && !t[..at].ends_with('\n') {
                    line.insert(0, '\n');
                }
                t.insert_str(at, &line);
                *self = TomlDoc::parse(&t)?;
                Ok(())
            }
            None => {
                let mut m = Map::new();
                m.insert(key.to_string(), v.clone());
                self.append_table(table, &m, false)
            }
        }
    }
}

impl Editor for TomlDoc {
    fn text(&self) -> &str {
        &self.text
    }

    fn get(&self, path: &[Seg]) -> Option<Value> {
        value_at(&self.value, path).cloned()
    }

    fn set(&mut self, path: &[Seg], value: &Value) -> Result<SetOutcome> {
        if path.is_empty() {
            return Err(edit_err("cannot replace a whole TOML document"));
        }
        // full path exists?
        if let Some(prev) = value_at(&self.value, path).cloned() {
            if let Some(k) = self.key_entry(path).cloned() {
                self.replace(k.value_start, k.value_end, &inline(value))?;
                return Ok(SetOutcome {
                    created_at: None,
                    prev: Some(prev),
                });
            }
            if let Some((s, e)) = self.table_block(path) {
                let Value::Object(obj) = value else {
                    return Err(edit_err(format!(
                        "`{}` is a table",
                        super::path_display(path)
                    )));
                };
                let aot = matches!(path.last(), Some(Seg::Index(_)));
                let mut block = format!("{}\n", header_text(path, aot));
                block.push_str(&body_text(obj));
                let tail = &self.text[e..];
                if !tail.is_empty() && !block.ends_with("\n\n") && tail.starts_with('[') {
                    block.push('\n');
                }
                self.replace(s, e, &block)?;
                return Ok(SetOutcome {
                    created_at: None,
                    prev: Some(prev),
                });
            }
            return Err(edit_err(format!(
                "`{}` is defined inside another value; edit it by hand",
                super::path_display(path)
            )));
        }
        // deepest existing prefix
        let mut depth = 0;
        while depth < path.len() && value_at(&self.value, &path[..=depth]).is_some() {
            depth += 1;
        }
        let parent = &path[..depth];
        if let Some(pv) = value_at(&self.value, parent) {
            if !pv.is_object() {
                return Err(edit_err(format!(
                    "`{}` is not a table",
                    super::path_display(parent)
                )));
            }
        }
        if !matches!(path[depth], Seg::Key(_)) {
            return Err(edit_err("TOML arrays are extended with push"));
        }
        match value {
            Value::Object(obj) => self.append_table(path, obj, false)?,
            leaf => {
                let (last, table) = path.split_last().unwrap();
                let Seg::Key(last) = last else {
                    return Err(edit_err("TOML arrays are extended with push"));
                };
                let table_known = table.is_empty() || self.section_of(table).is_some();
                if !table_known && value_at(&self.value, table).is_some() {
                    return Err(edit_err(format!(
                        "`{}` is an inline table; edit it by hand",
                        super::path_display(table)
                    )));
                }
                self.insert_key(table, last, leaf)?;
            }
        }
        Ok(SetOutcome {
            created_at: Some(depth),
            prev: None,
        })
    }

    fn remove(&mut self, path: &[Seg]) -> Result<Option<Value>> {
        let Some(prev) = value_at(&self.value, path).cloned() else {
            return Ok(None);
        };
        if let Some(k) = self.key_entry(path).cloned() {
            self.replace(k.line_start, k.line_end, "")?;
            return Ok(Some(prev));
        }
        if let Some((s, e)) = self.table_block(path) {
            self.remove_block(s, e)?;
            return Ok(Some(prev));
        }
        // array of tables: remove every element block
        if let Value::Array(items) = &prev {
            for i in (0..items.len()).rev() {
                let mut p = path.to_vec();
                p.push(Seg::Index(i));
                if let Some((s, e)) = self.table_block(&p) {
                    self.remove_block(s, e)?;
                }
            }
            return Ok(Some(prev));
        }
        // implicit table (only sub-tables): remove every sub-table
        let subs: Vec<Vec<Seg>> = self
            .sections
            .iter()
            .filter(|s| s.path.len() == path.len() + 1 && s.path[..path.len()] == *path)
            .map(|s| s.path.clone())
            .collect();
        if subs.is_empty() {
            return Err(edit_err(format!(
                "cannot remove `{}` here",
                super::path_display(path)
            )));
        }
        for sp in subs.into_iter().rev() {
            if let Some((s, e)) = self.table_block(&sp) {
                self.remove_block(s, e)?;
            }
        }
        Ok(Some(prev))
    }

    fn push(&mut self, path: &[Seg], value: &Value) -> Result<Option<SetOutcome>> {
        let existing = value_at(&self.value, path).cloned();
        match (&existing, value) {
            (None, Value::Object(obj)) => {
                // new array of tables
                let mut depth = 0;
                while depth < path.len() && value_at(&self.value, &path[..=depth]).is_some() {
                    depth += 1;
                }
                let mut p = path.to_vec();
                p.push(Seg::Index(0));
                self.append_table(&p, obj, true)?;
                Ok(Some(SetOutcome {
                    created_at: Some(depth),
                    prev: None,
                }))
            }
            (None, _) => Ok(Some(self.set(path, &Value::Array(vec![value.clone()]))?)),
            (Some(Value::Array(items)), Value::Object(obj)) if self.key_entry(path).is_none() => {
                let mut p = path.to_vec();
                p.push(Seg::Index(items.len()));
                self.append_table(&p, obj, true)?;
                Ok(None)
            }
            (Some(Value::Array(items)), _) => {
                let k = self
                    .key_entry(path)
                    .cloned()
                    .ok_or_else(|| edit_err("array without a key line"))?;
                let mut items = items.clone();
                items.push(value.clone());
                let original = &self.text[k.value_start..k.value_end];
                let rendered = render_array_like(original, &items);
                self.replace(k.value_start, k.value_end, &rendered)?;
                Ok(None)
            }
            _ => Err(edit_err(format!(
                "`{}` is not an array",
                super::path_display(path)
            ))),
        }
    }

    fn remove_item(&mut self, path: &[Seg], value: &Value) -> Result<bool> {
        let Some(Value::Array(items)) = value_at(&self.value, path).cloned() else {
            return Ok(false);
        };
        let Some(idx) = items.iter().position(|v| v == value) else {
            return Ok(false);
        };
        if let Some(k) = self.key_entry(path).cloned() {
            let mut items = items.clone();
            items.remove(idx);
            if items.is_empty() {
                self.replace(k.value_start, k.value_end, "[]")?;
            } else {
                let original = self.text[k.value_start..k.value_end].to_string();
                self.replace(
                    k.value_start,
                    k.value_end,
                    &render_array_like(&original, &items),
                )?;
            }
            return Ok(true);
        }
        let mut p = path.to_vec();
        p.push(Seg::Index(idx));
        if let Some((s, e)) = self.table_block(&p) {
            self.remove_block(s, e)?;
            return Ok(true);
        }
        Ok(false)
    }
}

/// Re-renders an array keeping the multi-line layout when the original had one.
fn render_array_like(original: &str, items: &[Value]) -> String {
    if original.contains('\n') {
        let indent = original
            .lines()
            .nth(1)
            .map(|l| {
                l.chars()
                    .take_while(|c| c.is_whitespace())
                    .collect::<String>()
            })
            .unwrap_or_else(|| "  ".into());
        let mut out = String::from("[\n");
        for v in items {
            out.push_str(&format!("{indent}{},\n", inline(v)));
        }
        out.push(']');
        out
    } else {
        inline(&Value::Array(items.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::edit::keys;
    use serde_json::json;

    #[test]
    fn parses_the_subset() {
        let v = parse(
            "# c\ntitle = \"x\" # tail\n[a.b]\nn = 1_000\nf = 1.5\nt = true\narr = [\n  \"x\", # c\n  'y',\n]\ninl = { k = \"v\", \"q k\" = [1, 2] }\nml = \"\"\"\nline1\nline2\"\"\"\nlit = '''\nraw\\n'''\nd = 2026-09-22T10:00:00Z\n[[hooks]]\nevent = \"Stop\"\n[[hooks]]\nevent = \"PreToolUse\"\n[hooks.opts]\nx = 1\n",
        )
        .unwrap();
        assert_eq!(v["title"], "x");
        assert_eq!(v["a"]["b"]["n"], 1000);
        assert_eq!(v["a"]["b"]["arr"], json!(["x", "y"]));
        assert_eq!(v["a"]["b"]["inl"]["q k"], json!([1, 2]));
        assert_eq!(v["a"]["b"]["ml"], "line1\nline2");
        assert_eq!(v["a"]["b"]["lit"], "raw\\n");
        assert_eq!(v["a"]["b"]["d"], "2026-09-22T10:00:00Z");
        assert_eq!(v["hooks"][1]["event"], "PreToolUse");
        assert_eq!(v["hooks"][1]["opts"]["x"], 1);
    }

    #[test]
    fn table_round_trip_keeps_comments() {
        let originals = [
            "",
            "# my codex config\nmodel = \"gpt-5\" # pinned\n\n[mcp_servers.old]\ncommand = \"x\"\n",
            "model = \"o3\"",
            "[profiles.fast]\nmodel = \"m\"\n\n# trailing comment\n",
        ];
        for original in originals {
            let mut doc = TomlDoc::parse(original).unwrap();
            let p = keys(["mcp_servers", "ctx7"]);
            let v = json!({"command": "npx", "args": ["-y", "@upstash/context7-mcp"], "env": {"K": "${K}"}});
            let out = doc.set(&p, &v).unwrap();
            assert_eq!(doc.get(&p), Some(v.clone()), "{}", doc.text());
            assert!(doc.text().contains("[mcp_servers.ctx7]"));
            let created = out.created_at.unwrap();
            if created == 0 {
                doc.remove(&p[..1]).unwrap();
            } else {
                doc.remove(&p).unwrap();
            }
            let expect = if original == "model = \"o3\"" {
                "model = \"o3\"\n"
            } else {
                original
            };
            assert_eq!(doc.text(), expect, "round trip of {original:?}");
        }
    }

    #[test]
    fn keys_and_arrays_of_tables() {
        let original = "[tui]\ntheme = \"dark\"\n\n[other]\nx = 1\n";
        let mut doc = TomlDoc::parse(original).unwrap();
        doc.set(&keys(["tui", "status_line"]), &json!(["model"]))
            .unwrap();
        assert!(doc
            .text()
            .contains("theme = \"dark\"\nstatus_line = [\"model\"]\n"));
        doc.remove(&keys(["tui", "status_line"])).unwrap();
        assert_eq!(doc.text(), original);

        let h = json!({"event": "PreToolUse", "command": "x.sh", "timeout": 5});
        doc.push(&keys(["hooks"]), &h).unwrap();
        doc.push(&keys(["hooks"]), &json!({"event": "Stop", "command": "y"}))
            .unwrap();
        assert_eq!(
            doc.get(&keys(["hooks"])).unwrap().as_array().unwrap().len(),
            2
        );
        assert!(doc
            .remove_item(&keys(["hooks"]), &json!({"event": "Stop", "command": "y"}))
            .unwrap());
        assert!(doc.remove_item(&keys(["hooks"]), &h).unwrap());
        assert_eq!(doc.text(), original);
    }
}
