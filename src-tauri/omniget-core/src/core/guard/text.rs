use std::collections::BTreeMap;

pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self { starts }
    }

    pub fn pos(&self, text: &str, offset: usize) -> (u32, u32) {
        let offset = offset.min(text.len());
        let line = match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.starts[line];
        let col = text
            .get(start..offset)
            .map(|s| s.chars().count())
            .unwrap_or(0);
        (line as u32 + 1, col as u32 + 1)
    }

    pub fn line_at<'a>(&self, text: &'a str, offset: usize) -> &'a str {
        let offset = offset.min(text.len());
        let line = match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.starts[line];
        let end = self
            .starts
            .get(line + 1)
            .map(|e| e.saturating_sub(1))
            .unwrap_or(text.len());
        text.get(start..end.max(start))
            .unwrap_or("")
            .trim_end_matches('\r')
    }
}

pub fn clip(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut out: String = t.chars().take(max).collect();
    out.push('…');
    out
}

#[derive(Debug, Clone, PartialEq)]
pub enum FmValue {
    Str(String),
    List(Vec<String>),
    Other(String),
}

impl FmValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            FmValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn items(&self) -> Vec<String> {
        match self {
            FmValue::List(v) => v.clone(),
            FmValue::Str(s) => split_list(s),
            FmValue::Other(_) => Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            FmValue::Str(s) => s.trim().is_empty(),
            FmValue::List(v) => v.is_empty(),
            FmValue::Other(s) => s.trim().is_empty(),
        }
    }
}

pub fn split_list(s: &str) -> Vec<String> {
    let s = s.trim();
    let s = s
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .unwrap_or(s);
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c)
            }
            ')' => {
                depth -= 1;
                cur.push(c)
            }
            ',' if depth <= 0 => {
                push_item(&mut out, &cur);
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    push_item(&mut out, &cur);
    out
}

fn push_item(out: &mut Vec<String>, raw: &str) {
    let t = unquote(raw.trim());
    if !t.is_empty() {
        out.push(t);
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

#[derive(Debug, Clone, Default)]
pub struct Frontmatter {
    pub present: bool,
    pub map: BTreeMap<String, FmValue>,
    pub body_offset: usize,
    pub error: Option<(u32, String)>,
}

impl Frontmatter {
    pub fn get(&self, key: &str) -> Option<&FmValue> {
        self.map.get(key)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.map.get(key).and_then(|v| v.as_str())
    }
}

pub fn parse_frontmatter(text: &str) -> Frontmatter {
    let bom = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let t = &text[bom..];
    let first_end = t.find('\n').unwrap_or(t.len());
    if t[..first_end].trim_end_matches('\r').trim_end() != "---" {
        return Frontmatter {
            body_offset: 0,
            ..Default::default()
        };
    }
    let mut fm = Frontmatter {
        present: true,
        ..Default::default()
    };
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut pos = first_end + 1;
    let mut closed_at = None;
    while pos <= t.len() {
        let end = t[pos..].find('\n').map(|e| pos + e).unwrap_or(t.len());
        let line = t[pos..end].trim_end_matches('\r');
        if line.trim_end() == "---" || line.trim_end() == "..." {
            closed_at = Some(end);
            break;
        }
        lines.push((pos, line));
        if end >= t.len() {
            break;
        }
        pos = end + 1;
    }
    let Some(close) = closed_at else {
        fm.error = Some((1, "frontmatter opened with --- but never closed".into()));
        fm.body_offset = 0;
        return fm;
    };
    fm.body_offset = bom + (close + 1).min(t.len());

    let mut i = 0;
    while i < lines.len() {
        let (_, line) = lines[i];
        let lineno = i as u32 + 2;
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            i += 1;
            continue;
        }
        if line.starts_with('\t') {
            fm.error
                .get_or_insert((lineno, "tab indentation is not valid YAML".into()));
            i += 1;
            continue;
        }
        if line.starts_with(' ') {
            i += 1;
            continue;
        }
        let Some(colon) = find_key_colon(line) else {
            fm.error.get_or_insert((
                lineno,
                format!("not a `key: value` line: {}", clip(line, 60)),
            ));
            i += 1;
            continue;
        };
        let key = unquote(&line[..colon]);
        let rest = line[colon + 1..].trim();
        let mut block: Vec<&str> = Vec::new();
        let mut j = i + 1;
        while j < lines.len() {
            let l = lines[j].1;
            if l.trim().is_empty()
                || l.starts_with(' ')
                || l.starts_with('\t')
                || l.starts_with("- ")
            {
                block.push(l);
                j += 1;
            } else {
                break;
            }
        }
        while block.last().is_some_and(|l| l.trim().is_empty()) {
            block.pop();
        }
        let value = if rest.is_empty() {
            let items: Vec<&str> = block
                .iter()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect();
            if !items.is_empty() && items.iter().all(|l| l.starts_with("- ") || *l == "-") {
                FmValue::List(
                    items
                        .iter()
                        .map(|l| unquote(l.trim_start_matches('-').trim()))
                        .filter(|s| !s.is_empty())
                        .collect(),
                )
            } else if items.is_empty() {
                FmValue::Str(String::new())
            } else {
                FmValue::Other(items.join("\n"))
            }
        } else if rest.starts_with('|') || rest.starts_with('>') {
            let folded = rest.starts_with('>');
            let body: Vec<&str> = block.iter().map(|l| l.trim()).collect();
            FmValue::Str(if folded {
                body.join(" ")
            } else {
                body.join("\n")
            })
        } else if rest.starts_with('[') && rest.contains(']') && !rest.trim_end().ends_with(']') {
            FmValue::Str(rest.to_string())
        } else if rest.starts_with('[') {
            let mut joined = rest.to_string();
            for l in &block {
                joined.push(' ');
                joined.push_str(l.trim());
            }
            if !joined.trim_end().ends_with(']') {
                fm.error
                    .get_or_insert((lineno, format!("unclosed list for `{key}`")));
            }
            FmValue::List(split_list(&joined))
        } else if rest.starts_with('{') {
            FmValue::Other(rest.to_string())
        } else {
            let mut s = rest.to_string();
            let quoted = s.starts_with('"') || s.starts_with('\'');
            for l in &block {
                s.push(' ');
                s.push_str(l.trim());
            }
            if quoted {
                let q = s.chars().next().unwrap_or('"');
                if !s.trim_end().ends_with(q) || s.trim().len() < 2 {
                    fm.error
                        .get_or_insert((lineno, format!("unterminated quoted value for `{key}`")));
                }
                let inner = unquote(&s);
                FmValue::Str(inner.replace("\\n", "\n").replace("\\\"", "\""))
            } else {
                FmValue::Str(s)
            }
        };
        fm.map.insert(key, value);
        i = j;
    }
    fm
}

fn find_key_colon(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut in_q: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match in_q {
            Some(q) if b == q => in_q = None,
            Some(_) => {}
            None => {
                if (b == b'"' || b == b'\'') && i == 0 {
                    in_q = Some(b)
                } else if b == b':'
                    && (i + 1 == bytes.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\t')
                {
                    return (i > 0).then_some(i);
                } else if b == b' ' && i == 0 {
                    return None;
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeCtx {
    Prose,
    Fence { shell: bool },
    Inline,
}

pub struct CodeMap {
    fences: Vec<(usize, usize, bool, String)>,
    inline: Vec<(usize, usize)>,
}

impl CodeMap {
    pub fn new(text: &str) -> Self {
        let mut fences = Vec::new();
        let mut inline = Vec::new();
        let mut pos = 0usize;
        let mut open: Option<(usize, String, usize, char)> = None;
        let mut prose_lines: Vec<(usize, usize)> = Vec::new();
        while pos < text.len() {
            let end = text[pos..]
                .find('\n')
                .map(|e| pos + e)
                .unwrap_or(text.len());
            let line = &text[pos..end];
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            let fence_char = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
            let fence_len = fence_char
                .map(|c| trimmed.chars().take_while(|x| *x == c).count())
                .unwrap_or(0);
            match &open {
                None if indent <= 3 && fence_len >= 3 => {
                    let lang = trimmed[fence_len..]
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_lowercase();
                    open = Some((end + 1, lang, fence_len, fence_char.unwrap_or('`')));
                }
                Some((start, lang, flen, fc))
                    if indent <= 3
                        && fence_char == Some(*fc)
                        && fence_len >= *flen
                        && trimmed[fence_len..].trim().is_empty() =>
                {
                    let shell = is_shell_lang(lang);
                    fences.push(((*start).min(text.len()), pos, shell, lang.clone()));
                    open = None;
                }
                Some(_) => {}
                None => prose_lines.push((pos, end)),
            }
            pos = end + 1;
        }
        if let Some((start, lang, _, _)) = open {
            let shell = is_shell_lang(&lang);
            fences.push((start.min(text.len()), text.len(), shell, lang));
        }
        for (s, e) in prose_lines {
            let line = &text[s..e];
            let b = line.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'`' {
                    let n = b[i..].iter().take_while(|c| **c == b'`').count();
                    let tick = &line[i..i + n];
                    if let Some(close) = line[i + n..].find(tick) {
                        inline.push((s + i, s + i + n + close + n));
                        i += n + close + n;
                        continue;
                    }
                    i += n;
                } else {
                    i += 1;
                }
            }
        }
        Self { fences, inline }
    }

    pub fn ctx(&self, offset: usize) -> CodeCtx {
        for (s, e, shell, _) in &self.fences {
            if offset >= *s && offset < *e {
                return CodeCtx::Fence { shell: *shell };
            }
        }
        for (s, e) in &self.inline {
            if offset >= *s && offset < *e {
                return CodeCtx::Inline;
            }
        }
        CodeCtx::Prose
    }

    pub fn shell_fences(&self) -> Vec<(usize, usize, String)> {
        self.fences
            .iter()
            .filter(|f| f.2)
            .map(|f| (f.0, f.1, f.3.clone()))
            .collect()
    }
}

fn is_shell_lang(lang: &str) -> bool {
    matches!(
        lang,
        "bash" | "sh" | "shell" | "zsh" | "console" | "shell-session" | "terminal" | "fish" | "ksh"
    )
}

pub fn looks_mentioned(line: &str, col_start_char: usize) -> bool {
    mention_score(line, col_start_char) > 0
}

pub fn mention_score(line: &str, col_start_char: usize) -> u8 {
    let before: String = line
        .chars()
        .take(col_start_char.saturating_sub(1))
        .collect();
    let quoted = before.matches('"').count() % 2 == 1
        || before.matches('“').count() > before.matches('”').count();
    let negated = negated_before(&before);
    quoted as u8 + negated as u8
}

fn negated_before(before: &str) -> bool {
    let tail: String = before
        .chars()
        .rev()
        .take(60)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let tail = tail.to_lowercase();
    const NEG: &[&str] = &[
        "never ",
        "don't ",
        "do not ",
        "dont ",
        "must not ",
        "should not ",
        "won't ",
        "cannot ",
        "refuse",
        "detect",
        "block",
        "prevent",
        "reject",
        "flag",
        "such as",
        "e.g.",
        "for example",
        "like \"",
        "attempts to",
        "attempt to",
        "nunca ",
        "não ",
        "nao ",
        "detectar",
        "bloquear",
        "evitar",
        "exemplo",
    ];
    NEG.iter().any(|n| tail.contains(n))
}

pub fn redact(value: &str) -> String {
    let v = value.trim();
    let n = v.chars().count();
    if n <= 6 {
        return "•••".into();
    }
    let head: String = v.chars().take(4).collect();
    format!("{head}…({n} chars, redacted)")
}

pub fn is_placeholder(value: &str) -> bool {
    let v = value.trim().trim_matches(|c| c == '"' || c == '\'');
    if v.is_empty() {
        return true;
    }
    let l = v.to_lowercase();
    v.starts_with('<')
        || v.starts_with("${")
        || v.starts_with("$")
        || v.starts_with("{{")
        || v.starts_with("%")
        || l.contains("your")
        || l.contains("xxx")
        || l.contains("***")
        || l.contains("•")
        || l.contains("example")
        || l.contains("changeme")
        || l.contains("placeholder")
        || l.contains("replace")
        || l.contains("insert")
        || l.contains("dummy")
        || l.contains("here")
        || l.contains("todo")
        || l.contains("redacted")
        || l.starts_with("sk-...")
        || l == "null"
        || l == "none"
        || l == "true"
        || l == "false"
        || v.chars().all(|c| {
            c == 'x' || c == 'X' || c == '*' || c == '.' || c == '-' || c == '_' || c == '0'
        })
}

pub fn entropy(s: &str) -> f64 {
    let mut counts = BTreeMap::new();
    let mut n = 0usize;
    for c in s.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    counts
        .values()
        .map(|&c| {
            let p = c as f64 / n as f64;
            -p * p.log2()
        })
        .sum()
}

pub fn is_secret_name(name: &str) -> bool {
    let l = name.to_lowercase().replace(['-', '.'], "_");
    let hits = [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
        "auth",
        "credential",
        "bearer",
        "session_key",
        "client_secret",
        "pat",
    ];
    hits.iter().any(|h| {
        if *h == "pat" || *h == "auth" {
            l.split('_').any(|p| p == *h)
        } else {
            l.contains(h)
        }
    }) && !l.ends_with("_url")
        && !l.ends_with("_file")
        && !l.ends_with("_path")
        && !l.ends_with("_helper")
        && !l.contains("tokens")
        && !l.ends_with("_ttl_ms")
}

pub fn looks_real_secret(value: &str) -> bool {
    let v = value.trim();
    !is_placeholder(v) && v.len() >= 12 && !v.contains(' ') && entropy(v) >= 3.0
}
