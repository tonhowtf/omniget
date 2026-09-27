//! Frontmatter YAML (subconjunto) → `serde_json::Value`, sem biblioteca de
//! terceiro. Port de `scripts/agentkit-catalog/yaml.mjs`: mapas e listas por
//! indentação, escalares simples/aspas, blocos `|`/`>`, fluxo `[..]`/`{..}` e
//! escalares simples continuados em várias linhas. Nunca falha: o pior caso é
//! um mapa `chave: valor` linha a linha.

use regex::Regex;
use serde_json::{Map, Value};
use std::sync::OnceLock;

/// Separa `---\n…\n---` do corpo. `None` quando não há frontmatter.
pub fn split(text: &str) -> Option<(&str, &str)> {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let first_nl = t.find('\n')?;
    if t[..first_nl].trim_end_matches('\r').trim_end() != "---" {
        return None;
    }
    let rest = &t[first_nl + 1..];
    let mut pos = 0usize;
    for line in rest.split_inclusive('\n') {
        let l = line.trim_end_matches('\n').trim_end_matches('\r');
        if l.trim_end() == "---" {
            let yaml = &rest[..pos];
            let body = &rest[pos + line.len()..];
            return Some((yaml, body));
        }
        pos += line.len();
    }
    None
}

/// Frontmatter parseado + corpo. Sem frontmatter: `({}, texto)`.
pub fn parse_document(text: &str) -> (Value, &str, bool) {
    match split(text) {
        Some((y, body)) => (parse_yaml(y), body, true),
        None => (Value::Object(Map::new()), text, false),
    }
}

fn key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"^("(?:[^"\\]|\\.)*"|'(?:[^']|'')*'|[^\s#'"\-?:,\[\]{}][^#]*?|-[^\s][^#]*?)\s*:(?:\s+(.*)|\s*)$"#,
        )
        .expect("key regex")
    })
}

fn indent_of(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b' ').count()
}

fn is_blank(line: &str) -> bool {
    let s = line.trim();
    s.is_empty() || s.starts_with('#')
}

fn is_dash(s: &str) -> bool {
    s == "-" || s.starts_with("- ")
}

fn strip_comment(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut q: Option<char> = None;
    for i in 0..chars.len() {
        let c = chars[i];
        if let Some(qc) = q {
            if c == qc {
                q = None;
            }
        } else if c == '"' || c == '\'' {
            if i == 0 || chars[i - 1].is_whitespace() || "[{,:".contains(chars[i - 1]) {
                q = Some(c);
            }
        } else if c == '#' && (i == 0 || chars[i - 1].is_whitespace()) {
            return chars[..i].iter().collect::<String>().trim_end().to_string();
        }
    }
    s.to_string()
}

fn unescape_double(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some(k @ ('u' | 'x')) => {
                let n = if k == 'u' { 4 } else { 2 };
                let hex: String = (0..n).filter_map(|_| it.next()).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => {
                        out.push(k);
                        out.push_str(&hex);
                    }
                }
            }
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

fn plain_scalar(s: &str) -> Value {
    let v = s.trim();
    match v {
        "" | "~" | "null" | "Null" | "NULL" => Value::Null,
        "true" | "True" | "TRUE" => Value::Bool(true),
        "false" | "False" | "FALSE" => Value::Bool(false),
        _ => {
            let digits = v.strip_prefix('-').unwrap_or(v);
            if !digits.is_empty()
                && digits.len() <= 15
                && digits.bytes().all(|b| b.is_ascii_digit())
                && (digits == "0" || !digits.starts_with('0'))
            {
                if let Ok(n) = v.parse::<i64>() {
                    return Value::from(n);
                }
            }
            Value::String(v.to_string())
        }
    }
}

fn find_close(s: &str, q: char) -> Option<usize> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut i = 1;
    while i < chars.len() {
        let c = chars[i].1;
        if q == '"' && c == '\\' {
            i += 2;
            continue;
        }
        if c == q {
            if q == '\'' && chars.get(i + 1).map(|x| x.1) == Some('\'') {
                i += 2;
                continue;
            }
            return Some(chars[i].0);
        }
        i += 1;
    }
    None
}

fn scalar_from(text: &str) -> Value {
    let s = text.trim();
    if let Some(stripped) = s.strip_prefix('"') {
        return Value::String(match find_close(s, '"') {
            Some(end) => unescape_double(&s[1..end]),
            None => unescape_double(stripped),
        });
    }
    if let Some(stripped) = s.strip_prefix('\'') {
        return Value::String(match find_close(s, '\'') {
            Some(end) => s[1..end].replace("''", "'"),
            None => stripped.replace("''", "'"),
        });
    }
    let c = strip_comment(s);
    if c.starts_with('[') || c.starts_with('{') {
        return parse_flow(&c);
    }
    plain_scalar(&c)
}

/// `[a, "b", {c: d}]` / `{a: 1}`.
pub fn parse_flow(src: &str) -> Value {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    fn ws(chars: &[char], i: &mut usize) {
        while *i < chars.len() && chars[*i].is_whitespace() {
            *i += 1;
        }
    }
    fn value(chars: &[char], i: &mut usize, stop: &str, depth: usize) -> Value {
        ws(chars, i);
        if *i >= chars.len() || depth > 64 {
            return Value::Null;
        }
        let c = chars[*i];
        if c == '[' {
            *i += 1;
            let mut arr = Vec::new();
            ws(chars, i);
            if *i < chars.len() && chars[*i] == ']' {
                *i += 1;
                return Value::Array(arr);
            }
            while *i < chars.len() {
                arr.push(value(chars, i, ",]", depth + 1));
                ws(chars, i);
                if *i < chars.len() && chars[*i] == ',' {
                    *i += 1;
                    ws(chars, i);
                    if *i < chars.len() && chars[*i] == ']' {
                        *i += 1;
                        return Value::Array(arr);
                    }
                    continue;
                }
                if *i < chars.len() && chars[*i] == ']' {
                    *i += 1;
                }
                break;
            }
            return Value::Array(arr);
        }
        if c == '{' {
            *i += 1;
            let mut obj = Map::new();
            ws(chars, i);
            if *i < chars.len() && chars[*i] == '}' {
                *i += 1;
                return Value::Object(obj);
            }
            while *i < chars.len() {
                let k = value(chars, i, ":,}", depth + 1);
                ws(chars, i);
                let mut v = Value::Null;
                if *i < chars.len() && chars[*i] == ':' {
                    *i += 1;
                    v = value(chars, i, ",}", depth + 1);
                }
                let key = match k {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                obj.insert(key, v);
                ws(chars, i);
                if *i < chars.len() && chars[*i] == ',' {
                    *i += 1;
                    continue;
                }
                if *i < chars.len() && chars[*i] == '}' {
                    *i += 1;
                }
                break;
            }
            return Value::Object(obj);
        }
        if c == '"' || c == '\'' {
            *i += 1;
            let mut out = String::new();
            while *i < chars.len() {
                if c == '"' && chars[*i] == '\\' && *i + 1 < chars.len() {
                    out.push(chars[*i]);
                    out.push(chars[*i + 1]);
                    *i += 2;
                    continue;
                }
                if chars[*i] == c {
                    if c == '\'' && *i + 1 < chars.len() && chars[*i + 1] == '\'' {
                        out.push('\'');
                        *i += 2;
                        continue;
                    }
                    *i += 1;
                    break;
                }
                out.push(chars[*i]);
                *i += 1;
            }
            return Value::String(if c == '"' { unescape_double(&out) } else { out });
        }
        let start = *i;
        while *i < chars.len() && !stop.contains(chars[*i]) {
            *i += 1;
        }
        plain_scalar(&chars[start..*i].iter().collect::<String>())
    }
    value(&chars, &mut i, "", 0)
}

fn balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut q: Option<char> = None;
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if let Some(qc) = q {
            if c == '\\' && qc == '"' {
                it.next();
                continue;
            }
            if c == qc {
                q = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => q = Some(c),
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth <= 0
}

fn key_text(k: &str) -> String {
    if k.starts_with('"') && k.len() >= 2 {
        return unescape_double(&k[1..k.len() - 1]);
    }
    if k.starts_with('\'') && k.len() >= 2 {
        return k[1..k.len() - 1].replace("''", "'");
    }
    k.trim().to_string()
}

struct Parser {
    lines: Vec<String>,
    i: usize,
}

impl Parser {
    fn skip(&mut self) {
        while self.i < self.lines.len() && is_blank(&self.lines[self.i]) {
            self.i += 1;
        }
    }

    fn node(&mut self, min_indent: usize, depth: usize) -> Value {
        self.skip();
        if self.i >= self.lines.len() || depth > 64 {
            return Value::Null;
        }
        let line = self.lines[self.i].clone();
        let ind = indent_of(&line);
        if ind < min_indent {
            return Value::Null;
        }
        let s = &line[ind..];
        if is_dash(s) {
            return self.list(ind, depth);
        }
        if key_re().is_match(s) {
            return self.map(ind, depth);
        }
        let mut parts = Vec::new();
        while self.i < self.lines.len()
            && (is_blank(&self.lines[self.i]) || indent_of(&self.lines[self.i]) >= ind)
        {
            let t = self.lines[self.i].trim();
            if !t.is_empty() {
                parts.push(t.to_string());
            }
            self.i += 1;
        }
        scalar_from(&parts.join(" "))
    }

    fn block_scalar(&mut self, header: &str, parent_indent: usize) -> Value {
        let fold = header.starts_with('>');
        let keep = header.contains('+');
        let strip = header.contains('-');
        let mut raw: Vec<String> = Vec::new();
        let mut block_indent: Option<usize> = None;
        while self.i < self.lines.len() {
            let l = &self.lines[self.i];
            if l.trim().is_empty() {
                raw.push(String::new());
                self.i += 1;
                continue;
            }
            let ind = indent_of(l);
            if ind <= parent_indent {
                break;
            }
            let bi = *block_indent.get_or_insert(ind);
            if ind < bi {
                break;
            }
            raw.push(l[bi..].to_string());
            self.i += 1;
        }
        let text = if fold {
            let mut t = String::new();
            for l in &raw {
                if l.is_empty() {
                    t.push('\n');
                    continue;
                }
                if !t.is_empty() && !t.ends_with('\n') {
                    if l.starts_with(char::is_whitespace) {
                        t.push('\n');
                    } else {
                        t.push(' ');
                    }
                }
                t.push_str(l);
            }
            t
        } else {
            raw.join("\n")
        };
        let trimmed = text.trim_end_matches('\n');
        Value::String(if strip {
            trimmed.to_string()
        } else if keep {
            format!("{text}\n")
        } else {
            format!("{trimmed}\n")
        })
    }

    fn inline_value(&mut self, rest: &str, own_indent: usize, depth: usize) -> Value {
        let r = rest.trim();
        if r.is_empty() || r.starts_with('#') {
            self.skip();
            if self.i < self.lines.len() {
                let nl = self.lines[self.i].clone();
                let ni = indent_of(&nl);
                if ni > own_indent {
                    return self.node(ni, depth + 1);
                }
                if ni == own_indent && is_dash(&nl[ni..]) {
                    return self.list(ni, depth + 1);
                }
            }
            return Value::Null;
        }
        let header_ok = {
            let b = r.as_bytes();
            (b[0] == b'|' || b[0] == b'>') && {
                let tail = strip_comment(&r[1..]);
                tail.trim()
                    .chars()
                    .all(|c| c == '+' || c == '-' || c.is_ascii_digit())
                    && tail.trim().len() <= 2
            }
        };
        if header_ok {
            return self.block_scalar(r, own_indent);
        }
        let first = r.chars().next().unwrap_or(' ');
        if (first == '"' || first == '\'') && find_close(r, first).is_none() {
            let mut acc = r.to_string();
            while self.i < self.lines.len() {
                let l = self.lines[self.i].trim().to_string();
                self.i += 1;
                acc.push(' ');
                acc.push_str(&l);
                if find_close(&acc, first).is_some() {
                    break;
                }
            }
            return scalar_from(&acc);
        }
        if first == '[' || first == '{' {
            let mut acc = strip_comment(r);
            let mut guard = 0;
            while !balanced(&acc) && self.i < self.lines.len() && guard < 500 {
                acc.push(' ');
                acc.push_str(&strip_comment(self.lines[self.i].trim()));
                self.i += 1;
                guard += 1;
            }
            return parse_flow(&acc);
        }
        let mut parts = vec![r.to_string()];
        while self.i < self.lines.len() {
            let l = &self.lines[self.i];
            if l.trim().is_empty() {
                let mut j = self.i + 1;
                while j < self.lines.len() && self.lines[j].trim().is_empty() {
                    j += 1;
                }
                if j < self.lines.len()
                    && indent_of(&self.lines[j]) > own_indent
                    && !key_re().is_match(self.lines[j].trim())
                {
                    parts.push("\n".into());
                    self.i = j;
                    continue;
                }
                break;
            }
            if indent_of(l) <= own_indent {
                break;
            }
            parts.push(l.trim().to_string());
            self.i += 1;
        }
        if parts.len() == 1 {
            return scalar_from(r);
        }
        let joined = parts
            .join(" ")
            .replace(" \n ", "\n")
            .replace(" \n", "\n")
            .replace("\n ", "\n");
        scalar_from(&joined)
    }

    fn map(&mut self, ind: usize, depth: usize) -> Value {
        let mut obj = Map::new();
        while self.i < self.lines.len() {
            self.skip();
            if self.i >= self.lines.len() {
                break;
            }
            let line = self.lines[self.i].clone();
            let li = indent_of(&line);
            if li < ind {
                break;
            }
            if li > ind {
                self.i += 1;
                continue;
            }
            let s = &line[li..];
            if is_dash(s) {
                break;
            }
            let Some(caps) = key_re().captures(s) else {
                self.i += 1;
                continue;
            };
            let key = key_text(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
            let rest = caps
                .get(2)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            self.i += 1;
            let v = self.inline_value(&rest, ind, depth);
            obj.insert(key, v);
        }
        Value::Object(obj)
    }

    fn list(&mut self, ind: usize, depth: usize) -> Value {
        let mut arr = Vec::new();
        while self.i < self.lines.len() {
            self.skip();
            if self.i >= self.lines.len() {
                break;
            }
            let line = self.lines[self.i].clone();
            let li = indent_of(&line);
            if li != ind {
                break;
            }
            let s = &line[li..];
            if !is_dash(s) {
                break;
            }
            let rest = if s == "-" { "" } else { &s[2..] };
            let rest_trim = rest.trim_start();
            let col = li + 2 + (rest.len() - rest_trim.len());
            if !rest_trim.is_empty()
                && key_re().is_match(rest_trim)
                && !rest_trim.starts_with('"')
                && !rest_trim.starts_with('\'')
            {
                self.lines[self.i] = format!("{}{}", " ".repeat(col), rest_trim);
                arr.push(self.map(col, depth + 1));
            } else {
                self.i += 1;
                let rt = rest_trim.to_string();
                arr.push(self.inline_value(&rt, li, depth + 1));
            }
        }
        Value::Array(arr)
    }
}

/// YAML → valor. Documento escalar vira `{"_value": …}`; vazio vira `{}`.
pub fn parse_yaml(text: &str) -> Value {
    let lines: Vec<String> = text
        .replace('\t', "  ")
        .split('\n')
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    let mut p = Parser { lines, i: 0 };
    match p.node(0, 0) {
        Value::Null => Value::Object(Map::new()),
        v @ Value::Object(_) => v,
        v @ Value::Array(_) => v,
        other => {
            let mut m = Map::new();
            m.insert("_value".into(), other);
            Value::Object(m)
        }
    }
}
