//! YAML: a lenient reader for frontmatter and tool configs (block maps and
//! sequences, flow `[..]`/`{..}`, plain/quoted/block scalars, comments) and a
//! line-preserving editor for block mappings (Goose `extensions`, Kiro, Rovo).

use serde_json::{Map, Value};

use super::{edit_err, nest, ordered_entries, value_at, Editor, Result, Seg, SetOutcome};

#[derive(Debug, Clone)]
struct Line {
    /// Column where the content starts.
    indent: usize,
    /// Byte offset of the line start in the text.
    start: usize,
    /// Byte offset after the newline.
    end: usize,
    raw: String,
}

impl Line {
    fn content(&self) -> &str {
        self.raw.get(self.indent..).unwrap_or("")
    }
    fn blank(&self) -> bool {
        let c = self.raw.trim();
        c.is_empty() || c.starts_with('#')
    }
}

fn split_lines(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut start = 0;
    for piece in text.split_inclusive('\n') {
        let end = start + piece.len();
        let raw = piece.trim_end_matches(['\n', '\r']).to_string();
        let indent = raw.chars().take_while(|c| *c == ' ').count();
        out.push(Line {
            indent,
            start,
            end,
            raw,
        });
        start = end;
    }
    out
}

/// Cuts a trailing ` # comment` outside quotes.
fn strip_comment(s: &str) -> &str {
    let b = s.as_bytes();
    let (mut sq, mut dq) = (false, false);
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' if dq => i += 1,
            b'\'' if !dq => sq = !sq,
            b'"' if !sq => dq = !dq,
            b'#' if !sq && !dq && (i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t') => {
                return s[..i].trim_end();
            }
            _ => {}
        }
        i += 1;
    }
    s.trim_end()
}

/// `key: rest` → (key, rest). Handles quoted keys; the colon must be followed by
/// a space or end the line.
fn split_key(content: &str) -> Option<(String, &str)> {
    let c = content;
    if c.starts_with('"') || c.starts_with('\'') {
        let q = c.as_bytes()[0];
        let mut i = 1;
        let b = c.as_bytes();
        while i < b.len() {
            if b[i] == b'\\' && q == b'"' {
                i += 2;
                continue;
            }
            if b[i] == q {
                break;
            }
            i += 1;
        }
        let key_raw = &c[..(i + 1).min(c.len())];
        let rest = c.get(i + 1..)?.trim_start();
        let rest = rest.strip_prefix(':')?;
        if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
            return None;
        }
        let key = match quoted(key_raw) {
            Some(Value::String(s)) => s,
            _ => key_raw
                .trim_matches(|ch| ch == '"' || ch == '\'')
                .to_string(),
        };
        return Some((key, rest.trim_start()));
    }
    if c.starts_with('[') || c.starts_with('{') || c.starts_with("- ") || c == "-" {
        return None;
    }
    let b = c.as_bytes();
    for i in 0..b.len() {
        if b[i] == b':' && (i + 1 == b.len() || b[i + 1] == b' ' || b[i + 1] == b'\t') {
            let key = c[..i].trim();
            if key.is_empty() || key.contains(" #") {
                return None;
            }
            return Some((key.to_string(), c[i + 1..].trim_start()));
        }
        if b[i] == b'#' && i > 0 && b[i - 1] == b' ' {
            return None;
        }
    }
    None
}

fn is_seq_item(content: &str) -> bool {
    content == "-" || content.starts_with("- ") || content.starts_with("-\t")
}

/// Parses YAML text into a value (lenient: unknown constructs become strings).
pub fn parse(text: &str) -> Result<Value> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = split_lines(text);
    // skip a leading document marker
    if let Some(first) = lines.iter().position(|l| !l.blank()) {
        if lines[first].raw.trim() == "---" {
            lines[first].raw = String::new();
            lines[first].indent = 0;
        }
    }
    let mut p = Parser { lines, i: 0 };
    let v = p.node(0);
    Ok(v)
}

struct Parser {
    lines: Vec<Line>,
    i: usize,
}

impl Parser {
    fn skip_blank(&mut self) {
        while self.i < self.lines.len() && self.lines[self.i].blank() {
            self.i += 1;
        }
    }

    fn node(&mut self, min_indent: usize) -> Value {
        self.skip_blank();
        if self.i >= self.lines.len() {
            return Value::Null;
        }
        let l = &self.lines[self.i];
        if l.indent < min_indent {
            return Value::Null;
        }
        let indent = l.indent;
        let content = strip_comment(l.content()).to_string();
        if is_seq_item(&content) {
            return self.seq(indent);
        }
        if split_key(&content).is_some() {
            return self.map(indent);
        }
        // a lone scalar / flow value possibly continued on more lines
        self.i += 1;
        self.inline_value(&content, indent.saturating_sub(1))
    }

    fn map(&mut self, indent: usize) -> Value {
        let mut m = Map::new();
        loop {
            self.skip_blank();
            if self.i >= self.lines.len() {
                break;
            }
            let l = &self.lines[self.i];
            if l.indent != indent {
                break;
            }
            let content = strip_comment(l.content()).to_string();
            let Some((key, rest)) = split_key(&content) else {
                break;
            };
            let rest = rest.to_string();
            self.i += 1;
            let v = self.after_key(&rest, indent);
            if key == "<<" {
                if let Value::Object(inner) = v {
                    for (k, x) in inner {
                        m.entry(k).or_insert(x);
                    }
                    continue;
                }
            }
            m.insert(key, v);
        }
        Value::Object(m)
    }

    fn seq(&mut self, indent: usize) -> Value {
        let mut items = Vec::new();
        loop {
            self.skip_blank();
            if self.i >= self.lines.len() {
                break;
            }
            let l = &self.lines[self.i];
            if l.indent != indent {
                break;
            }
            let content = strip_comment(l.content()).to_string();
            if !is_seq_item(&content) {
                break;
            }
            let after = content[1..].trim_start();
            if after.is_empty() {
                self.i += 1;
                items.push(self.node(indent + 1));
                continue;
            }
            let offset = content.len() - after.len();
            if split_key(after).is_some() || is_seq_item(after) {
                // the item is a block collection starting on the dash line
                let col = indent + offset;
                let line = &mut self.lines[self.i];
                line.indent = col;
                items.push(self.node(col));
                continue;
            }
            let after = after.to_string();
            self.i += 1;
            items.push(self.inline_value(&after, indent));
        }
        Value::Array(items)
    }

    /// Value after `key:` (or after `- `) on a line whose parent indent is `indent`.
    fn after_key(&mut self, rest: &str, indent: usize) -> Value {
        let rest = rest.trim();
        if rest.is_empty() {
            self.skip_blank();
            if self.i < self.lines.len() {
                let l = &self.lines[self.i];
                let c = strip_comment(l.content()).to_string();
                if l.indent > indent || (l.indent == indent && is_seq_item(&c)) {
                    let min = if l.indent == indent {
                        indent
                    } else {
                        indent + 1
                    };
                    return self.node(min);
                }
            }
            return Value::Null;
        }
        self.inline_value(rest, indent)
    }

    fn inline_value(&mut self, rest: &str, indent: usize) -> Value {
        let mut rest = rest.trim().to_string();
        // tags and anchors
        loop {
            if rest.starts_with('!') || rest.starts_with('&') {
                match rest.find(' ') {
                    Some(sp) => rest = rest[sp + 1..].trim_start().to_string(),
                    None => {
                        rest.clear();
                    }
                }
                continue;
            }
            break;
        }
        if rest.is_empty() {
            return self.after_key("", indent);
        }
        if rest.starts_with('|') || rest.starts_with('>') {
            return Value::String(self.block_scalar(&rest, indent));
        }
        if rest.starts_with('[') || rest.starts_with('{') {
            let mut buf = rest.clone();
            while !balanced(&buf) && self.i < self.lines.len() {
                buf.push(' ');
                buf.push_str(strip_comment(self.lines[self.i].raw.trim()));
                self.i += 1;
            }
            let mut fp = Flow {
                s: buf.as_bytes(),
                text: &buf,
                pos: 0,
            };
            return fp.value().unwrap_or(Value::String(buf.clone()));
        }
        if rest.starts_with('"') || rest.starts_with('\'') {
            let q = rest.chars().next().unwrap();
            let mut buf = rest.clone();
            while !closed_quote(&buf, q) && self.i < self.lines.len() {
                let next = self.lines[self.i].raw.trim().to_string();
                buf.push('\n');
                buf.push_str(&next);
                self.i += 1;
            }
            let joined = fold_quoted_lines(&buf);
            let end = quote_end(&joined, q).unwrap_or(joined.len());
            return quoted(&joined[..end]).unwrap_or(Value::String(joined));
        }
        // plain scalar, maybe continued on deeper lines
        let mut text = strip_comment(&rest).to_string();
        while self.i < self.lines.len() {
            let l = &self.lines[self.i];
            if l.raw.trim().is_empty() || l.indent <= indent {
                break;
            }
            let c = strip_comment(l.content()).to_string();
            if split_key(&c).is_some() || is_seq_item(&c) {
                break;
            }
            text.push(' ');
            text.push_str(c.trim());
            self.i += 1;
        }
        plain(&text)
    }

    fn block_scalar(&mut self, header: &str, indent: usize) -> String {
        let folded = header.starts_with('>');
        let keep = header.contains('+');
        let strip = header.contains('-');
        let mut raw: Vec<String> = Vec::new();
        let mut content_indent: Option<usize> = None;
        while self.i < self.lines.len() {
            let l = &self.lines[self.i];
            if l.raw.trim().is_empty() {
                raw.push(String::new());
                self.i += 1;
                continue;
            }
            if l.indent <= indent {
                break;
            }
            let ci = *content_indent.get_or_insert(l.indent);
            if l.indent < ci {
                break;
            }
            raw.push(l.raw[ci..].to_string());
            self.i += 1;
        }
        // trailing blank lines are not part of the scalar body
        let mut trailing = 0;
        while raw.last().map(|s| s.is_empty()).unwrap_or(false) {
            raw.pop();
            trailing += 1;
        }
        let mut out = if folded {
            let mut s = String::new();
            let mut prev_more = false;
            for (idx, line) in raw.iter().enumerate() {
                let more = line.starts_with(' ') || line.starts_with('\t');
                if idx > 0 {
                    if line.is_empty() || more || prev_more {
                        s.push('\n');
                    } else if !raw[idx - 1].is_empty() {
                        s.push(' ');
                    }
                }
                s.push_str(line);
                prev_more = more;
            }
            s
        } else {
            raw.join("\n")
        };
        if !strip {
            out.push('\n');
            if keep {
                for _ in 0..trailing {
                    out.push('\n');
                }
            }
        }
        out
    }
}

fn balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let (mut sq, mut dq) = (false, false);
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' if dq => i += 1,
            b'\'' if !dq => sq = !sq,
            b'"' if !sq => dq = !dq,
            b'[' | b'{' if !sq && !dq => depth += 1,
            b']' | b'}' if !sq && !dq => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    depth <= 0
}

fn quote_end(s: &str, q: char) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 1;
    while i < b.len() {
        if q == '"' && b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == q as u8 {
            if q == '\'' && b.get(i + 1) == Some(&b'\'') {
                i += 2;
                continue;
            }
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

fn closed_quote(s: &str, q: char) -> bool {
    quote_end(s, q).is_some()
}

/// Line folding inside quoted scalars: newline + indentation → one space.
fn fold_quoted_lines(s: &str) -> String {
    if !s.contains('\n') {
        return s.to_string();
    }
    let parts: Vec<&str> = s.split('\n').collect();
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            out.push_str(p.trim_end());
        } else if p.trim().is_empty() {
            out.push('\n');
        } else {
            if !out.ends_with('\n') {
                out.push(' ');
            }
            out.push_str(p.trim());
        }
    }
    out
}

/// A quoted scalar (with its quotes) → string.
fn quoted(s: &str) -> Option<Value> {
    let q = s.chars().next()?;
    if s.len() < 2 || !s.ends_with(q) {
        return None;
    }
    let inner = &s[1..s.len() - 1];
    if q == '\'' {
        return Some(Value::String(inner.replace("''", "'")));
    }
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            'e' => out.push('\u{1b}'),
            ' ' => out.push(' '),
            '/' => out.push('/'),
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            '\n' => {}
            'x' => {
                let h: String = chars.by_ref().take(2).collect();
                out.push(char::from_u32(u32::from_str_radix(&h, 16).ok()?)?);
            }
            'u' => {
                let h: String = chars.by_ref().take(4).collect();
                out.push(char::from_u32(u32::from_str_radix(&h, 16).ok()?)?);
            }
            'U' => {
                let h: String = chars.by_ref().take(8).collect();
                out.push(char::from_u32(u32::from_str_radix(&h, 16).ok()?)?);
            }
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    Some(Value::String(out))
}

/// Resolves a plain scalar to null/bool/number/string.
fn plain(s: &str) -> Value {
    let t = s.trim();
    match t {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        _ => {}
    }
    let digits_ok = t
        .trim_start_matches(['-', '+'])
        .chars()
        .next()
        .map(|c| c.is_ascii_digit() || c == '.')
        .unwrap_or(false);
    if digits_ok {
        if let Ok(i) = t.parse::<i64>() {
            return Value::from(i);
        }
        if t.contains('.') && !t.ends_with('.') && t.matches('.').count() == 1 {
            if let Ok(f) = t.parse::<f64>() {
                if let Some(n) = serde_json::Number::from_f64(f) {
                    return Value::Number(n);
                }
            }
        }
    }
    Value::String(t.to_string())
}

struct Flow<'a> {
    s: &'a [u8],
    text: &'a str,
    pos: usize,
}

impl<'a> Flow<'a> {
    fn ws(&mut self) {
        while self.pos < self.s.len() && (self.s[self.pos] as char).is_whitespace() {
            self.pos += 1;
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.ws();
        match *self.s.get(self.pos)? {
            b'[' => {
                self.pos += 1;
                let mut items = vec![];
                loop {
                    self.ws();
                    if self.s.get(self.pos) == Some(&b']') {
                        self.pos += 1;
                        return Some(Value::Array(items));
                    }
                    items.push(self.value()?);
                    self.ws();
                    match self.s.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {}
                        _ => return None,
                    }
                }
            }
            b'{' => {
                self.pos += 1;
                let mut m = Map::new();
                loop {
                    self.ws();
                    if self.s.get(self.pos) == Some(&b'}') {
                        self.pos += 1;
                        return Some(Value::Object(m));
                    }
                    let k = match self.value()? {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    self.ws();
                    let v = if self.s.get(self.pos) == Some(&b':') {
                        self.pos += 1;
                        self.value()?
                    } else {
                        Value::Null
                    };
                    m.insert(k, v);
                    self.ws();
                    match self.s.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {}
                        _ => return None,
                    }
                }
            }
            b'"' | b'\'' => {
                let q = self.s[self.pos] as char;
                let end = quote_end(&self.text[self.pos..], q)?;
                let v = quoted(&self.text[self.pos..self.pos + end]);
                self.pos += end;
                v
            }
            _ => {
                let start = self.pos;
                while self.pos < self.s.len() {
                    let c = self.s[self.pos];
                    if c == b',' || c == b']' || c == b'}' {
                        break;
                    }
                    if c == b':' && (self.pos + 1 >= self.s.len() || self.s[self.pos + 1] == b' ') {
                        break;
                    }
                    self.pos += 1;
                }
                Some(plain(&self.text[start..self.pos]))
            }
        }
    }
}

// ------------------------------------------------------------ writing

fn needs_quotes(s: &str) -> bool {
    if s.is_empty()
        || s.trim() != s
        || s.contains('\n')
        || s.contains(": ")
        || s.contains(" #")
        || s.ends_with(':')
    {
        return true;
    }
    let first = s.chars().next().unwrap();
    if "-?:,[]{}#&*!|>'\"%@`".contains(first) {
        return true;
    }
    !matches!(plain(s), Value::String(_))
}

/// A scalar as YAML text.
pub fn scalar_text(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            if needs_quotes(s) {
                serde_json::to_string(s).unwrap_or_default()
            } else {
                s.clone()
            }
        }
        Value::Array(a) if a.is_empty() => "[]".into(),
        Value::Object(m) if m.is_empty() => "{}".into(),
        other => super::jsonc::compact(other),
    }
}

fn key_out(k: &str) -> String {
    if needs_quotes(k) {
        serde_json::to_string(k).unwrap_or_default()
    } else {
        k.to_string()
    }
}

/// `key: value` block at `indent` columns, ending with a newline.
pub fn entry_text(key: &str, v: &Value, indent: usize) -> String {
    let pad = " ".repeat(indent);
    match v {
        Value::Object(m) if !m.is_empty() => {
            format!("{pad}{}:\n{}", key_out(key), map_text(m, indent + 2))
        }
        Value::Array(a) if !a.is_empty() => {
            format!("{pad}{}:\n{}", key_out(key), seq_text(a, indent + 2))
        }
        other => format!("{pad}{}: {}\n", key_out(key), scalar_text(other)),
    }
}

fn map_text(m: &Map<String, Value>, indent: usize) -> String {
    ordered_entries(m)
        .into_iter()
        .map(|(k, v)| entry_text(k, v, indent))
        .collect()
}

fn seq_text(a: &[Value], indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for item in a {
        match item {
            Value::Object(m) if !m.is_empty() => {
                let body = map_text(m, indent + 2);
                // put the first entry on the dash line
                let trimmed = body.strip_prefix(&" ".repeat(indent + 2)).unwrap_or(&body);
                out.push_str(&format!("{pad}- {trimmed}"));
            }
            Value::Array(inner) if !inner.is_empty() => {
                out.push_str(&format!("{pad}-\n{}", seq_text(inner, indent + 2)));
            }
            other => out.push_str(&format!("{pad}- {}\n", scalar_text(other))),
        }
    }
    out
}

/// Whole document text for a value (maps and sequences at the top).
pub fn to_text(v: &Value) -> String {
    match v {
        Value::Object(m) => map_text(m, 0),
        Value::Array(a) => seq_text(a, 0),
        other => format!("{}\n", scalar_text(other)),
    }
}

/// A YAML document open for editing (block mappings).
#[derive(Debug, Clone)]
pub struct YamlDoc {
    text: String,
    lines: Vec<Line>,
}

struct Found {
    key_line: usize,
    /// One past the last non-blank line of the entry.
    block_end: usize,
    map_indent: usize,
}

impl YamlDoc {
    pub fn parse(text: &str) -> Result<YamlDoc> {
        let _ = parse(text)?;
        Ok(YamlDoc {
            text: text.to_string(),
            lines: split_lines(text),
        })
    }

    fn reload(&mut self, text: String) {
        self.lines = split_lines(&text);
        self.text = text;
    }

    fn map_indent_in(&self, from: usize, to: usize) -> Option<usize> {
        self.lines[from..to]
            .iter()
            .find(|l| !l.blank() && l.raw.trim() != "---")
            .map(|l| l.indent)
    }

    fn find(&self, from: usize, to: usize, key: &str) -> Option<Found> {
        let ind = self.map_indent_in(from, to)?;
        let mut i = from;
        while i < to {
            let l = &self.lines[i];
            if !l.blank() && l.indent == ind {
                let c = strip_comment(l.content());
                if let Some((k, _)) = split_key(c) {
                    if k == key {
                        let mut j = i + 1;
                        let mut last_non_blank = i;
                        while j < to {
                            let lj = &self.lines[j];
                            if !lj.blank() {
                                let cj = strip_comment(lj.content());
                                if lj.indent < ind || (lj.indent == ind && !is_seq_item(cj)) {
                                    break;
                                }
                                last_non_blank = j;
                            }
                            j += 1;
                        }
                        return Some(Found {
                            key_line: i,
                            block_end: last_non_blank + 1,
                            map_indent: ind,
                        });
                    }
                }
            }
            i += 1;
        }
        None
    }

    fn offset_of_line(&self, idx: usize) -> usize {
        self.lines
            .get(idx)
            .map(|l| l.start)
            .unwrap_or(self.text.len())
    }

    fn splice_lines(&mut self, from: usize, to: usize, with: &str) {
        let s = self.offset_of_line(from);
        let e = if to == 0 {
            0
        } else {
            self.lines
                .get(to - 1)
                .map(|l| l.end)
                .unwrap_or(self.text.len())
        };
        let e = e.max(s);
        let mut t = self.text.clone();
        let mut with = with.to_string();
        if e == t.len() && !t.ends_with('\n') && e > s && !with.is_empty() {
            // replacing the last line which had no newline
        } else if s == t.len() && !t.is_empty() && !t.ends_with('\n') {
            with.insert(0, '\n');
        }
        t.replace_range(s..e, &with);
        self.reload(t);
    }

    /// Locates the entry for `path`, returning the found entry for each depth.
    fn walk(&self, path: &[Seg]) -> std::result::Result<Vec<Found>, (usize, usize, usize, usize)> {
        let mut out = Vec::new();
        let (mut from, mut to) = (0usize, self.lines.len());
        let mut parent_indent: Option<usize> = None;
        for (i, seg) in path.iter().enumerate() {
            let Seg::Key(k) = seg else {
                return Err((i, from, to, parent_indent.map(|p| p + 2).unwrap_or(0)));
            };
            match self.find(from, to, k) {
                Some(f) => {
                    from = f.key_line + 1;
                    to = f.block_end;
                    parent_indent = Some(f.map_indent);
                    out.push(f);
                }
                None => {
                    let ind = self
                        .map_indent_in(from, to)
                        .unwrap_or_else(|| parent_indent.map(|p| p + 2).unwrap_or(0));
                    return Err((i, from, to, ind));
                }
            }
        }
        Ok(out)
    }
}

impl Editor for YamlDoc {
    fn text(&self) -> &str {
        &self.text
    }

    fn get(&self, path: &[Seg]) -> Option<Value> {
        let v = parse(&self.text).ok()?;
        value_at(&v, path).cloned()
    }

    fn set(&mut self, path: &[Seg], value: &Value) -> Result<SetOutcome> {
        if path.is_empty() {
            let prev = self.value();
            self.reload(to_text(value));
            return Ok(SetOutcome {
                created_at: None,
                prev: Some(prev),
            });
        }
        match self.walk(path) {
            Ok(found) => {
                let f = found.last().unwrap();
                let prev = self.get(path);
                let Seg::Key(k) = path.last().unwrap() else {
                    unreachable!()
                };
                let text = entry_text(k, value, f.map_indent);
                self.splice_lines(f.key_line, f.block_end, &text);
                Ok(SetOutcome {
                    created_at: None,
                    prev,
                })
            }
            Err((i, from, to, indent)) => {
                if i > 0 {
                    // the parent must be a block map (or empty)
                    let parent_val = self.get(&path[..i]);
                    match parent_val {
                        Some(Value::Object(ref m)) if m.is_empty() && from == to => {
                            // `key: {}` or `key:` with nothing below: rewrite the parent entry
                            let found = self
                                .walk(&path[..i])
                                .map_err(|_| edit_err("lost the parent entry"))?;
                            let f = found.last().unwrap();
                            let Seg::Key(pk) = &path[i - 1] else {
                                unreachable!()
                            };
                            let v = nest(&path[i..], value);
                            let text = entry_text(pk, &v, f.map_indent);
                            self.splice_lines(f.key_line, f.block_end, &text);
                            return Ok(SetOutcome {
                                created_at: Some(i),
                                prev: None,
                            });
                        }
                        Some(Value::Object(_)) | Some(Value::Null) => {}
                        None if i == 0 => {}
                        _ => {
                            return Err(edit_err(format!(
                                "`{}` is not a map",
                                super::path_display(&path[..i])
                            )))
                        }
                    }
                    if matches!(parent_val, Some(Value::Null)) {
                        let found = self
                            .walk(&path[..i])
                            .map_err(|_| edit_err("lost the parent entry"))?;
                        let f = found.last().unwrap();
                        let Seg::Key(pk) = &path[i - 1] else {
                            unreachable!()
                        };
                        let v = nest(&path[i..], value);
                        let text = entry_text(pk, &v, f.map_indent);
                        self.splice_lines(f.key_line, f.block_end, &text);
                        return Ok(SetOutcome {
                            created_at: Some(i),
                            prev: None,
                        });
                    }
                }
                let Seg::Key(k) = &path[i] else {
                    return Err(edit_err("YAML sequences are extended with push"));
                };
                let v = nest(&path[i + 1..], value);
                let text = entry_text(k, &v, indent);
                // insert after the last non-blank line of the parent block
                let mut at = to;
                while at > from && self.lines[at - 1].blank() {
                    at -= 1;
                }
                if i == 0 {
                    at = self.lines.len();
                    while at > 0 && self.lines[at - 1].raw.trim().is_empty() {
                        at -= 1;
                    }
                }
                self.splice_lines(at, at, &text);
                Ok(SetOutcome {
                    created_at: Some(i),
                    prev: None,
                })
            }
        }
    }

    fn remove(&mut self, path: &[Seg]) -> Result<Option<Value>> {
        let prev = self.get(path);
        if prev.is_none() {
            return Ok(None);
        }
        match self.walk(path) {
            Ok(found) => {
                let f = found.last().unwrap();
                let (a, b) = (f.key_line, f.block_end);
                self.splice_lines(a, b, "");
                Ok(prev)
            }
            Err(_) => Err(edit_err(format!(
                "`{}` is not in a block map",
                super::path_display(path)
            ))),
        }
    }

    fn push(&mut self, path: &[Seg], value: &Value) -> Result<Option<SetOutcome>> {
        match self.get(path) {
            None | Some(Value::Null) if self.walk(path).is_err() => {
                Ok(Some(self.set(path, &Value::Array(vec![value.clone()]))?))
            }
            Some(Value::Array(items)) if !items.is_empty() => {
                let found = self
                    .walk(path)
                    .map_err(|_| edit_err("sequence not found"))?;
                let f = found.last().unwrap();
                let item_indent = self.lines[f.key_line + 1..f.block_end]
                    .iter()
                    .find(|l| !l.blank())
                    .map(|l| l.indent)
                    .unwrap_or(f.map_indent + 2);
                let text = seq_text(std::slice::from_ref(value), item_indent);
                self.splice_lines(f.block_end, f.block_end, &text);
                Ok(None)
            }
            Some(Value::Array(_)) | Some(Value::Null) => {
                let prev = self.get(path);
                self.set(path, &Value::Array(vec![value.clone()]))?;
                Ok(Some(SetOutcome {
                    created_at: None,
                    prev,
                }))
            }
            _ => Err(edit_err(format!(
                "`{}` is not a sequence",
                super::path_display(path)
            ))),
        }
    }

    fn remove_item(&mut self, path: &[Seg], value: &Value) -> Result<bool> {
        let Some(Value::Array(items)) = self.get(path) else {
            return Ok(false);
        };
        let Some(idx) = items.iter().position(|v| v == value) else {
            return Ok(false);
        };
        let found = self
            .walk(path)
            .map_err(|_| edit_err("sequence not found"))?;
        let f = found.last().unwrap();
        let body: Vec<usize> = (f.key_line + 1..f.block_end).collect();
        let item_indent = body
            .iter()
            .map(|i| &self.lines[*i])
            .find(|l| !l.blank())
            .map(|l| l.indent);
        let Some(ind) = item_indent else {
            return Ok(false);
        };
        let starts: Vec<usize> = body
            .iter()
            .copied()
            .filter(|i| {
                let l = &self.lines[*i];
                !l.blank() && l.indent == ind && is_seq_item(strip_comment(l.content()))
            })
            .collect();
        let Some(&s) = starts.get(idx) else {
            return Ok(false);
        };
        let mut e = starts.get(idx + 1).copied().unwrap_or(f.block_end);
        while e > s + 1 && self.lines[e - 1].blank() {
            e -= 1;
        }
        if items.len() == 1 {
            let Seg::Key(k) = path.last().unwrap() else {
                return Ok(false);
            };
            let text = entry_text(k, &Value::Array(vec![]), f.map_indent);
            self.splice_lines(f.key_line, f.block_end, &text);
            return Ok(true);
        }
        self.splice_lines(s, e, "");
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::edit::keys;
    use serde_json::json;

    #[test]
    fn parses_frontmatter_shapes() {
        let v = parse(
            "name: frontend-developer\ndescription: \"Use when\\n\\n<example>x</example>\"\ntools: Read, Write, Edit\nmodel: sonnet # comment\ncomponents: [agent:dev/test-runner, hook:testing/x]\nlist:\n  - a\n  - b: 1\n    c: [x, 'y z']\nfolded: >\n  one\n  two\n\n  three\nlit: |-\n  keep\n    this\nn: 12\nf: 1.5\nb: true\nempty:\nq: 'it''s'\n",
        )
        .unwrap();
        assert_eq!(v["name"], "frontend-developer");
        assert_eq!(v["description"], "Use when\n\n<example>x</example>");
        assert_eq!(v["tools"], "Read, Write, Edit");
        assert_eq!(v["model"], "sonnet");
        assert_eq!(
            v["components"],
            json!(["agent:dev/test-runner", "hook:testing/x"])
        );
        assert_eq!(v["list"][1]["c"], json!(["x", "y z"]));
        assert_eq!(v["folded"], "one two\nthree\n");
        assert_eq!(v["lit"], "keep\n  this");
        assert_eq!(v["n"], 12);
        assert_eq!(v["b"], true);
        assert_eq!(v["empty"], Value::Null);
        assert_eq!(v["q"], "it's");
    }

    #[test]
    fn map_round_trip_keeps_comments() {
        let originals = [
            "",
            "# goose config\nGOOSE_PROVIDER: openai # mine\nextensions:\n  developer:\n    enabled: true\n    type: builtin\n",
            "extensions:\n  developer:\n    enabled: true\n\nother: 1\n",
        ];
        for original in originals {
            let mut doc = YamlDoc::parse(original).unwrap();
            let p = keys(["extensions", "github"]);
            let v = json!({"name": "github", "type": "stdio", "cmd": "npx", "args": ["-y", "gh-mcp"], "envs": {"GITHUB_TOKEN": "${GITHUB_TOKEN}"}, "enabled": true});
            let out = doc.set(&p, &v).unwrap();
            assert_eq!(doc.get(&p), Some(v.clone()), "{}", doc.text());
            let at = out.created_at.unwrap();
            doc.remove(&p[..=at]).unwrap();
            assert_eq!(doc.text(), original);
        }
    }

    #[test]
    fn sequence_push_and_remove() {
        let original = "hooks:\n  - event: Stop\n    command: a\n";
        let mut doc = YamlDoc::parse(original).unwrap();
        let item = json!({"event": "PreToolUse", "command": "b"});
        doc.push(&keys(["hooks"]), &item).unwrap();
        assert_eq!(
            doc.get(&keys(["hooks"])).unwrap().as_array().unwrap().len(),
            2
        );
        assert!(doc.remove_item(&keys(["hooks"]), &item).unwrap());
        assert_eq!(doc.text(), original);
    }
}
