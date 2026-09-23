//! JSON with comments and trailing commas (VS Code, Cursor, OpenCode, Kilo, Zed).
//! A concrete-syntax parse keeps the byte span of every node; edits are text
//! splices, so nothing outside the touched member moves.

use serde_json::Value;

use super::{edit_err, nest, ordered_entries, parse_err, Editor, Result, Seg, SetOutcome};

#[derive(Debug, Clone)]
struct Node {
    start: usize,
    end: usize,
    kind: Kind,
}

#[derive(Debug, Clone)]
enum Kind {
    Object(Vec<Member>),
    Array(Vec<Node>),
    Scalar,
}

#[derive(Debug, Clone)]
struct Member {
    key: String,
    key_start: usize,
    value: Node,
}

/// A JSONC document open for editing.
#[derive(Debug, Clone)]
pub struct JsoncDoc {
    text: String,
    root: Option<Node>,
}

/// Parses JSON/JSONC text into a value (comments and trailing commas allowed).
pub fn parse_value(text: &str) -> Result<Value> {
    let doc = JsoncDoc::parse(text)?;
    Ok(doc.value())
}

impl JsoncDoc {
    pub fn parse(text: &str) -> Result<JsoncDoc> {
        let mut p = Parser {
            s: text.as_bytes(),
            text,
            pos: 0,
        };
        p.skip_trivia()?;
        if p.pos >= p.s.len() {
            return Ok(JsoncDoc {
                text: text.to_string(),
                root: None,
            });
        }
        let root = p.value(0)?;
        p.skip_trivia()?;
        if p.pos < p.s.len() {
            return Err(p.err("unexpected text after the document"));
        }
        Ok(JsoncDoc {
            text: text.to_string(),
            root: Some(root),
        })
    }

    fn reparse(&mut self, new_text: String) -> Result<()> {
        let doc = JsoncDoc::parse(&new_text)?;
        *self = doc;
        Ok(())
    }

    /// Applies text edits given in document order; edits at the same offset end
    /// up in the order they were listed.
    fn splice(&mut self, edits: Vec<(usize, usize, String)>) -> Result<()> {
        let mut edits: Vec<(usize, (usize, usize, String))> =
            edits.into_iter().enumerate().collect();
        edits.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(b.0.cmp(&a.0)));
        let mut t = self.text.clone();
        for (_, (s, e, r)) in edits {
            t.replace_range(s..e, &r);
        }
        self.reparse(t)
    }

    fn node_at(&self, path: &[Seg]) -> Option<&Node> {
        let mut cur = self.root.as_ref()?;
        for seg in path {
            cur = match (&cur.kind, seg) {
                (Kind::Object(ms), Seg::Key(k)) => &ms.iter().rev().find(|m| &m.key == k)?.value,
                (Kind::Array(items), Seg::Index(i)) => items.get(*i)?,
                _ => return None,
            };
        }
        Some(cur)
    }

    fn node_value(&self, node: &Node) -> Value {
        match &node.kind {
            Kind::Object(ms) => {
                let mut m = serde_json::Map::new();
                for mem in ms {
                    m.insert(mem.key.clone(), self.node_value(&mem.value));
                }
                Value::Object(m)
            }
            Kind::Array(items) => Value::Array(items.iter().map(|n| self.node_value(n)).collect()),
            Kind::Scalar => {
                serde_json::from_str(&self.text[node.start..node.end]).unwrap_or(Value::Null)
            }
        }
    }

    // ---- layout helpers ----

    fn line_start(&self, pos: usize) -> usize {
        self.text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0)
    }

    fn line_indent(&self, pos: usize) -> String {
        let ls = self.line_start(pos);
        self.text[ls..]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    fn indent_unit(&self) -> String {
        for line in self.text.lines() {
            let ws: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            if !ws.is_empty() && line.len() > ws.len() {
                if ws.contains('\t') {
                    return "\t".into();
                }
                return ws;
            }
        }
        "  ".into()
    }

    /// Position after whitespace and comments starting at `pos`.
    fn skip_trivia(&self, pos: usize) -> usize {
        let s = self.text.as_bytes();
        let mut p = pos;
        loop {
            while p < s.len() && (s[p] as char).is_ascii_whitespace() {
                p += 1;
            }
            if p + 1 < s.len() && s[p] == b'/' && s[p + 1] == b'/' {
                while p < s.len() && s[p] != b'\n' {
                    p += 1;
                }
                continue;
            }
            if p + 1 < s.len() && s[p] == b'/' && s[p + 1] == b'*' {
                p += 2;
                while p + 1 < s.len() && !(s[p] == b'*' && s[p + 1] == b'/') {
                    p += 1;
                }
                p = (p + 2).min(s.len());
                continue;
            }
            return p;
        }
    }

    fn comma_after(&self, pos: usize, limit: usize) -> Option<usize> {
        let p = self.skip_trivia(pos);
        (p < limit && self.text.as_bytes()[p] == b',').then_some(p)
    }

    // ---- generic container edits (object members and array items) ----

    /// Entries as (start, value_end) byte ranges.
    fn entries(node: &Node) -> Vec<(usize, usize)> {
        match &node.kind {
            Kind::Object(ms) => ms.iter().map(|m| (m.key_start, m.value.end)).collect(),
            Kind::Array(items) => items.iter().map(|n| (n.start, n.end)).collect(),
            Kind::Scalar => Vec::new(),
        }
    }

    /// Inserts `entry` (already rendered for multi-line layout, and `inline_entry`
    /// for single-line containers) as the last entry of `node`.
    fn insert_entry(&mut self, node: &Node, render: &dyn Fn(&str, bool) -> String) -> Result<()> {
        let entries = Self::entries(node);
        let close = node.end - 1;
        let unit = self.indent_unit();
        if entries.is_empty() {
            let inner = &self.text[node.start + 1..close];
            let parent = self.line_indent(node.start);
            let child = format!("{parent}{unit}");
            let body = render(&child, false);
            if inner.trim().is_empty() {
                return self.splice(vec![(
                    node.start + 1,
                    close,
                    format!("\n{child}{body}\n{parent}"),
                )]);
            }
            let kept = inner.trim_end().to_string();
            return self.splice(vec![(
                node.start + 1,
                close,
                format!("{kept}\n{child}{body}\n{parent}"),
            )]);
        }
        let (last_start, after) = *entries.last().unwrap();
        let inline = !self.text[node.start..node.end].contains('\n');
        let trailing = self.comma_after(after, close);
        if inline {
            let body = render("", true);
            return match trailing {
                Some(c) => self.splice(vec![(c + 1, c + 1, format!(" {body},"))]),
                None => self.splice(vec![(after, after, format!(", {body}"))]),
            };
        }
        let child = self.line_indent(last_start);
        let body = render(&child, false);
        let region_nl = self.text[after..close].find('\n').map(|i| after + i);
        match trailing {
            Some(c) => {
                let ins = region_nl.filter(|p| *p > c).unwrap_or(c + 1);
                self.splice(vec![(ins, ins, format!("\n{child}{body},"))])
            }
            None => match region_nl {
                Some(nl) => self.splice(vec![
                    (after, after, ",".into()),
                    (nl, nl, format!("\n{child}{body}")),
                ]),
                None => self.splice(vec![(after, after, format!(",\n{child}{body}"))]),
            },
        }
    }

    fn remove_entry(&mut self, node: &Node, idx: usize) -> Result<()> {
        let entries = Self::entries(node);
        let close = node.end - 1;
        let n = entries.len();
        let (start, vend) = entries[idx];
        let own_comma = self.comma_after(vend, close);
        let end_with_comma = own_comma.map(|c| c + 1).unwrap_or(vend);
        if n == 1 {
            let rest = format!(
                "{}{}",
                &self.text[node.start + 1..start],
                &self.text[end_with_comma..close]
            );
            if rest.trim().is_empty() {
                return self.splice(vec![(node.start + 1, close, String::new())]);
            }
            return self.splice(vec![(start, end_with_comma, String::new())]);
        }
        if idx == n - 1 {
            let (_, prev_end) = entries[idx - 1];
            let pc = self
                .comma_after(prev_end, start)
                .ok_or_else(|| edit_err("missing comma between entries"))?;
            let nl_before = self.text[..start].rfind('\n').filter(|p| *p > pc);
            return match (nl_before, own_comma) {
                (Some(nl), Some(c)) => self.splice(vec![(nl, c + 1, String::new())]),
                (Some(nl), None) => {
                    self.splice(vec![(pc, pc + 1, String::new()), (nl, vend, String::new())])
                }
                (None, Some(c)) => self.splice(vec![(pc + 1, c + 1, String::new())]),
                (None, None) => self.splice(vec![(pc, vend, String::new())]),
            };
        }
        let (next_start, _) = entries[idx + 1];
        let c = own_comma.ok_or_else(|| edit_err("missing comma between entries"))?;
        let ls = self.line_start(start);
        let own_line = self.text[ls..start].trim().is_empty();
        let nl_after = self.text[c..next_start].find('\n').map(|i| c + i);
        match (own_line, nl_after) {
            (true, Some(nl)) => self.splice(vec![(ls, nl + 1, String::new())]),
            _ => {
                let mut e = c + 1;
                let b = self.text.as_bytes();
                while e < next_start && (b[e] == b' ' || b[e] == b'\t') {
                    e += 1;
                }
                self.splice(vec![(start, e, String::new())])
            }
        }
    }
}

impl Editor for JsoncDoc {
    fn text(&self) -> &str {
        &self.text
    }

    fn get(&self, path: &[Seg]) -> Option<Value> {
        let node = self.node_at(path)?;
        Some(self.node_value(node))
    }

    fn set(&mut self, path: &[Seg], value: &Value) -> Result<SetOutcome> {
        let unit = self.indent_unit();
        let Some(root) = self.root.clone() else {
            let lead_ws: String = self
                .text
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect();
            let _ = lead_ws;
            let body = pretty(&nest(path, value), "", &unit);
            let tail = if self.text.ends_with('\n') || self.text.is_empty() {
                "\n"
            } else {
                ""
            };
            self.reparse(format!("{body}{tail}"))?;
            return Ok(SetOutcome {
                created_at: Some(0),
                prev: None,
            });
        };
        if path.is_empty() {
            let prev = self.node_value(&root);
            let body = pretty(value, &self.line_indent(root.start), &unit);
            self.splice(vec![(root.start, root.end, body)])?;
            return Ok(SetOutcome {
                created_at: None,
                prev: Some(prev),
            });
        }
        let mut cur = root;
        for (i, seg) in path.iter().enumerate() {
            let last = i + 1 == path.len();
            match (&cur.kind, seg) {
                (Kind::Object(ms), Seg::Key(k)) => {
                    if let Some(m) = ms.iter().rev().find(|m| &m.key == k) {
                        if last {
                            let prev = self.node_value(&m.value);
                            let indent = self.line_indent(m.key_start);
                            let body = pretty(value, &indent, &unit);
                            let (s, e) = (m.value.start, m.value.end);
                            self.splice(vec![(s, e, body)])?;
                            return Ok(SetOutcome {
                                created_at: None,
                                prev: Some(prev),
                            });
                        }
                        cur = m.value.clone();
                        continue;
                    }
                    let v = nest(&path[i + 1..], value);
                    let key = serde_json::to_string(k).unwrap_or_default();
                    let unit2 = unit.clone();
                    self.insert_entry(&cur, &|indent, inline| {
                        if inline {
                            format!("{key}: {}", compact(&v))
                        } else {
                            format!("{key}: {}", pretty(&v, indent, &unit2))
                        }
                    })?;
                    return Ok(SetOutcome {
                        created_at: Some(i),
                        prev: None,
                    });
                }
                (Kind::Array(items), Seg::Index(idx)) => {
                    if *idx < items.len() {
                        if last {
                            let node = items[*idx].clone();
                            let prev = self.node_value(&node);
                            let body = pretty(value, &self.line_indent(node.start), &unit);
                            self.splice(vec![(node.start, node.end, body)])?;
                            return Ok(SetOutcome {
                                created_at: None,
                                prev: Some(prev),
                            });
                        }
                        cur = items[*idx].clone();
                        continue;
                    }
                    if *idx == items.len() {
                        let v = nest(&path[i + 1..], value);
                        let unit2 = unit.clone();
                        self.insert_entry(&cur, &|indent, inline| {
                            if inline {
                                compact(&v)
                            } else {
                                pretty(&v, indent, &unit2)
                            }
                        })?;
                        return Ok(SetOutcome {
                            created_at: Some(i),
                            prev: None,
                        });
                    }
                    return Err(edit_err(format!(
                        "index {idx} is past the end of the array"
                    )));
                }
                _ => {
                    return Err(edit_err(format!(
                        "`{}` is not a container that accepts `{seg}`",
                        super::path_display(&path[..i])
                    )))
                }
            }
        }
        unreachable!("loop returns on the last segment")
    }

    fn remove(&mut self, path: &[Seg]) -> Result<Option<Value>> {
        let Some((last, parent_path)) = path.split_last() else {
            return Err(edit_err("cannot remove the document root"));
        };
        let Some(parent) = self.node_at(parent_path).cloned() else {
            return Ok(None);
        };
        match (&parent.kind, last) {
            (Kind::Object(ms), Seg::Key(k)) => {
                let Some(idx) = ms.iter().rposition(|m| &m.key == k) else {
                    return Ok(None);
                };
                let prev = self.node_value(&ms[idx].value);
                self.remove_entry(&parent, idx)?;
                Ok(Some(prev))
            }
            (Kind::Array(items), Seg::Index(i)) => {
                if *i >= items.len() {
                    return Ok(None);
                }
                let prev = self.node_value(&items[*i]);
                self.remove_entry(&parent, *i)?;
                Ok(Some(prev))
            }
            _ => Ok(None),
        }
    }

    fn push(&mut self, path: &[Seg], value: &Value) -> Result<Option<SetOutcome>> {
        match self.node_at(path).cloned() {
            None => Ok(Some(self.set(path, &Value::Array(vec![value.clone()]))?)),
            Some(node) => match &node.kind {
                Kind::Array(_) => {
                    let unit = self.indent_unit();
                    let v = value.clone();
                    self.insert_entry(&node, &|indent, inline| {
                        if inline {
                            compact(&v)
                        } else {
                            pretty(&v, indent, &unit)
                        }
                    })?;
                    Ok(None)
                }
                _ => Err(edit_err(format!(
                    "`{}` is not an array",
                    super::path_display(path)
                ))),
            },
        }
    }

    fn remove_item(&mut self, path: &[Seg], value: &Value) -> Result<bool> {
        let Some(node) = self.node_at(path).cloned() else {
            return Ok(false);
        };
        let Kind::Array(items) = &node.kind else {
            return Ok(false);
        };
        let Some(idx) = items.iter().position(|n| &self.node_value(n) == value) else {
            return Ok(false);
        };
        self.remove_entry(&node, idx)?;
        Ok(true)
    }
}

/// Pretty JSON whose continuation lines start with `indent`.
pub fn pretty(value: &Value, indent: &str, unit: &str) -> String {
    let mut out = String::new();
    write_pretty(value, indent, unit, &mut out);
    out
}

fn write_pretty(value: &Value, indent: &str, unit: &str, out: &mut String) {
    match value {
        Value::Object(m) if !m.is_empty() => {
            out.push_str("{\n");
            let inner = format!("{indent}{unit}");
            let entries = ordered_entries(m);
            for (i, (k, v)) in entries.iter().enumerate() {
                out.push_str(&inner);
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push_str(": ");
                write_pretty(v, &inner, unit, out);
                if i + 1 < entries.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(indent);
            out.push('}');
        }
        Value::Array(a) if !a.is_empty() => {
            let all_scalar = a.iter().all(|v| !v.is_object() && !v.is_array());
            let inline = compact(value);
            if all_scalar && inline.len() + indent.len() <= 100 {
                out.push_str(&inline);
                return;
            }
            out.push_str("[\n");
            let inner = format!("{indent}{unit}");
            for (i, v) in a.iter().enumerate() {
                out.push_str(&inner);
                write_pretty(v, &inner, unit, out);
                if i + 1 < a.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(indent);
            out.push(']');
        }
        Value::Object(_) => out.push_str("{}"),
        Value::Array(_) => out.push_str("[]"),
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

/// One-line JSON with a space after `:` and `,`.
pub fn compact(value: &Value) -> String {
    match value {
        Value::Object(m) => {
            let parts: Vec<String> = ordered_entries(m)
                .into_iter()
                .map(|(k, v)| {
                    format!(
                        "{}: {}",
                        serde_json::to_string(k).unwrap_or_default(),
                        compact(v)
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().map(compact).collect();
            format!("[{}]", parts.join(", "))
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn err(&self, msg: &str) -> super::super::AgentkitError {
        parse_err("json", super::line_of(self.text, self.pos), msg)
    }

    fn skip_trivia(&mut self) -> Result<()> {
        loop {
            while self.pos < self.s.len() && (self.s[self.pos] as char).is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.s.len() >= 3 && self.pos == 0 && self.s.starts_with(&[0xEF, 0xBB, 0xBF]) {
                self.pos = 3;
                continue;
            }
            if self.pos + 1 < self.s.len() && self.s[self.pos] == b'/' {
                match self.s[self.pos + 1] {
                    b'/' => {
                        while self.pos < self.s.len() && self.s[self.pos] != b'\n' {
                            self.pos += 1;
                        }
                        continue;
                    }
                    b'*' => {
                        self.pos += 2;
                        while self.pos + 1 < self.s.len()
                            && !(self.s[self.pos] == b'*' && self.s[self.pos + 1] == b'/')
                        {
                            self.pos += 1;
                        }
                        if self.pos + 1 >= self.s.len() {
                            return Err(self.err("unterminated block comment"));
                        }
                        self.pos += 2;
                        continue;
                    }
                    _ => {}
                }
            }
            return Ok(());
        }
    }

    fn value(&mut self, depth: usize) -> Result<Node> {
        if depth > 256 {
            return Err(self.err("nesting too deep"));
        }
        self.skip_trivia()?;
        let start = self.pos;
        match self.s.get(self.pos) {
            None => Err(self.err("unexpected end of document")),
            Some(b'{') => {
                self.pos += 1;
                let mut members = Vec::new();
                loop {
                    self.skip_trivia()?;
                    match self.s.get(self.pos) {
                        Some(b'}') => {
                            self.pos += 1;
                            break;
                        }
                        Some(b'"') => {
                            let key_start = self.pos;
                            self.string()?;
                            let key: String = serde_json::from_str(&self.text[key_start..self.pos])
                                .map_err(|e| self.err(&format!("bad key: {e}")))?;
                            self.skip_trivia()?;
                            if self.s.get(self.pos) != Some(&b':') {
                                return Err(self.err("expected `:`"));
                            }
                            self.pos += 1;
                            let value = self.value(depth + 1)?;
                            members.push(Member {
                                key,
                                key_start,
                                value,
                            });
                            self.skip_trivia()?;
                            match self.s.get(self.pos) {
                                Some(b',') => self.pos += 1,
                                Some(b'}') => {}
                                _ => return Err(self.err("expected `,` or `}`")),
                            }
                        }
                        _ => return Err(self.err("expected a key or `}`")),
                    }
                }
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: Kind::Object(members),
                })
            }
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_trivia()?;
                    if self.s.get(self.pos) == Some(&b']') {
                        self.pos += 1;
                        break;
                    }
                    items.push(self.value(depth + 1)?);
                    self.skip_trivia()?;
                    match self.s.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {}
                        _ => return Err(self.err("expected `,` or `]`")),
                    }
                }
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: Kind::Array(items),
                })
            }
            Some(b'"') => {
                self.string()?;
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: Kind::Scalar,
                })
            }
            Some(_) => {
                while self.pos < self.s.len() {
                    let c = self.s[self.pos];
                    if c == b','
                        || c == b']'
                        || c == b'}'
                        || c == b'/'
                        || (c as char).is_ascii_whitespace()
                    {
                        break;
                    }
                    self.pos += 1;
                }
                let tok = &self.text[start..self.pos];
                if serde_json::from_str::<Value>(tok).is_err() {
                    return Err(self.err(&format!("bad token `{tok}`")));
                }
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: Kind::Scalar,
                })
            }
        }
    }

    fn string(&mut self) -> Result<()> {
        self.pos += 1;
        while self.pos < self.s.len() {
            match self.s[self.pos] {
                b'\\' => self.pos += 2,
                b'"' => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => self.pos += 1,
            }
        }
        Err(self.err("unterminated string"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::edit::keys;
    use serde_json::json;

    fn round_trip(original: &str, path: &[&str], value: Value) {
        let mut doc = JsoncDoc::parse(original).unwrap();
        let p = keys(path.iter().copied());
        let out = doc.set(&p, &value).unwrap();
        assert_eq!(
            doc.get(&p),
            Some(value.clone()),
            "after set:\n{}",
            doc.text()
        );
        let created = out.created_at.expect("created");
        doc.remove(&p[..=created]).unwrap();
        assert_eq!(doc.text(), original, "round trip failed");
    }

    #[test]
    fn insert_and_remove_keep_bytes() {
        let cases = [
            "{}\n",
            "{\n  \"a\": 1\n}\n",
            "{\n    \"a\": 1, // keep me\n    \"b\": [1, 2]\n}",
            "{\n  // leading comment\n  \"mcp\": {\n    \"x\": {\"type\": \"local\"},\n  },\n}\n",
            "{\"a\": 1, \"b\": 2}",
            "{\n\t\"a\": {\n\t\t\"z\": true\n\t}\n}\n",
        ];
        for c in cases {
            round_trip(
                c,
                &["mcpServers", "srv"],
                json!({"command": "npx", "args": ["-y", "x"]}),
            );
            round_trip(c, &["a2"], json!("v"));
        }
        round_trip(
            "{\n  // leading comment\n  \"mcp\": {\n    \"x\": {\"type\": \"local\"},\n  },\n}\n",
            &["mcp", "y"],
            json!({"type": "remote", "url": "https://x"}),
        );
        round_trip(
            "{\n  \"mcpServers\": {}\n}\n",
            &["mcpServers", "a"],
            json!({"url": "u"}),
        );
    }

    #[test]
    fn push_and_remove_item() {
        let original = "{\n  \"hooks\": {\n    \"PreToolUse\": [\n      {\"matcher\": \"Bash\"} // mine\n    ]\n  }\n}\n";
        let mut doc = JsoncDoc::parse(original).unwrap();
        let p = keys(["hooks", "PreToolUse"]);
        assert!(doc.push(&p, &json!({"matcher": "Edit"})).unwrap().is_none());
        assert_eq!(doc.get(&p).unwrap().as_array().unwrap().len(), 2);
        assert!(doc.text().contains("// mine"));
        assert!(doc.remove_item(&p, &json!({"matcher": "Edit"})).unwrap());
        assert_eq!(doc.text(), original);
        let p2 = keys(["hooks", "Stop"]);
        let out = doc.push(&p2, &json!({"x": 1})).unwrap().unwrap();
        assert_eq!(out.created_at, Some(1));
        doc.remove(&p2).unwrap();
        assert_eq!(doc.text(), original);
    }

    #[test]
    fn replace_and_restore() {
        let original = "{\n  \"a\": {\"b\": 1}, /* c */\n  \"z\": 0\n}";
        let mut doc = JsoncDoc::parse(original).unwrap();
        let out = doc.set(&keys(["a"]), &json!({"b": 2})).unwrap();
        assert_eq!(out.prev, Some(json!({"b": 1})));
        assert!(doc.text().contains("/* c */"));
        doc.set(&keys(["a"]), &json!({"b": 1})).unwrap();
        assert_eq!(doc.value(), JsoncDoc::parse(original).unwrap().value());
    }

    #[test]
    fn remove_middle_member() {
        let mut doc = JsoncDoc::parse("{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3\n}").unwrap();
        doc.remove(&keys(["b"])).unwrap();
        assert_eq!(doc.text(), "{\n  \"a\": 1,\n  \"c\": 3\n}");
    }
}
