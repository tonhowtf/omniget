//! Text helpers shared by the validators: line/column lookup, snippets with a
//! length cap, a small YAML-frontmatter reader (no YAML library, on purpose:
//! frontmatter in agent files is a flat key/value block and a full YAML engine
//! would accept things no tool reads), Markdown code-span masking and secret
//! redaction.

use std::collections::BTreeMap;

/// Byte offsets of every line start, for offset → (line, column).
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

    /// 1-based line and column (column counted in chars).
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

    /// The whole line (without newline) holding `offset`.
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

/// Cut `s` to at most `max` chars on a char boundary, with an ellipsis.
pub fn clip(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut out: String = t.chars().take(max).collect();
    out.push('…');
    out
}

/// Largest char boundary ≤ `i`.
pub fn floor_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest char boundary ≥ `i`.
pub fn ceil_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

// ------------------------------------------------------------- frontmatter

/// A frontmatter value as the tools read it.
#[derive(Debug, Clone, PartialEq)]
pub enum FmValue {
    Str(String),
    List(Vec<String>),
    /// Nested mapping or anything else: kept as the raw text.
    Other(String),
}

impl FmValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            FmValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Items of a list, or of a comma-separated string.
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

/// Split a comma-separated list, respecting `Bash(git add:*)` parentheses.
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
    /// True when the file opens with a `---` line.
    pub present: bool,
    /// Parsed top-level keys.
    pub map: BTreeMap<String, FmValue>,
    /// Byte offset where the body starts (after the closing `---`).
    pub body_offset: usize,
    /// Parse problem, with the 1-based line it happened on.
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

/// Read the frontmatter block at the top of a Markdown file.
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
    // Find the closing line.
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
            // Continuation of a plain multi-line scalar; YAML accepts it only
            // under a key, which we already consumed. Stray indentation here
            // means the previous value was a plain scalar that wrapped.
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
        // Gather the indented block under this key.
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
            // `argument-hint: [name] | --flag`: tools read it as a string.
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
    // `key:` where key is a plain or quoted scalar, colon followed by space/EOL.
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

// ------------------------------------------------------------- code masking

/// Where in a Markdown file an offset sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeCtx {
    Prose,
    /// Inside a fenced block; the flag says it is a shell-ish fence.
    Fence {
        shell: bool,
    },
    Inline,
}

/// Fenced block and inline-code ranges of a Markdown text.
pub struct CodeMap {
    fences: Vec<(usize, usize, bool, String)>,
    inline: Vec<(usize, usize)>,
}

impl CodeMap {
    pub fn new(text: &str) -> Self {
        let mut fences = Vec::new();
        let mut inline = Vec::new();
        let mut pos = 0usize;
        let mut open: Option<(usize, String, usize, char)> = None; // (body start, lang, fence len, char)
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
                        .trim()
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

    /// `(body start, body end, language)` of the shell fences.
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

/// True when the match looks mentioned rather than meant: quoted on its line,
/// or preceded by a negation/detection word ("never", "detect", "block"...).
pub fn looks_mentioned(line: &str, col_start_char: usize) -> bool {
    mention_score(line, col_start_char) > 0
}

/// 0, 1 or 2: one point for being quoted, one for a negation/example word
/// before it ("never", "detect", "e.g."). Two points = clearly a mention.
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

// ------------------------------------------------------------- secrets

/// Redact a secret-looking value: first 4 chars, then its length.
pub fn redact(value: &str) -> String {
    let v = value.trim();
    let n = v.chars().count();
    if n <= 6 {
        return "•••".into();
    }
    let head: String = v.chars().take(4).collect();
    format!("{head}…({n} chars, redacted)")
}

/// True for placeholder values that are meant to be filled in by the user.
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

/// Shannon entropy in bits per char.
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

/// True when a name reads like it holds a secret (`GITHUB_TOKEN`, `apiKey`).
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

/// True when a value looks like a real secret rather than a placeholder.
pub fn looks_real_secret(value: &str) -> bool {
    let v = value.trim();
    !is_placeholder(v) && v.len() >= 12 && !v.contains(' ') && entropy(v) >= 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_flat_and_lists() {
        let t = "---\nname: a\ndescription: \"hello: world\"\ntools: Read, Bash(git add:*), Grep\nlist:\n  - x\n  - 'y'\nblock: |\n  one\n  two\n---\n# Body\n";
        let fm = parse_frontmatter(t);
        assert!(fm.present);
        assert!(fm.error.is_none(), "{:?}", fm.error);
        assert_eq!(fm.str("name"), Some("a"));
        assert_eq!(fm.str("description"), Some("hello: world"));
        assert_eq!(
            fm.get("tools").unwrap().items(),
            vec!["Read", "Bash(git add:*)", "Grep"]
        );
        assert_eq!(fm.get("list").unwrap().items(), vec!["x", "y"]);
        assert_eq!(fm.str("block"), Some("one\ntwo"));
        assert!(t[fm.body_offset..].starts_with("# Body"));
    }

    #[test]
    fn frontmatter_unclosed() {
        let fm = parse_frontmatter("---\nname: a\n# no close\n");
        assert!(fm.error.is_some());
    }

    #[test]
    fn code_map_fences() {
        let t = "text `inline` more\n```bash\nrm -rf /\n```\nafter";
        let m = CodeMap::new(t);
        assert_eq!(m.ctx(t.find("inline").unwrap()), CodeCtx::Inline);
        assert_eq!(
            m.ctx(t.find("rm -rf").unwrap()),
            CodeCtx::Fence { shell: true }
        );
        assert_eq!(m.ctx(t.find("after").unwrap()), CodeCtx::Prose);
    }

    #[test]
    fn secret_heuristics() {
        assert!(is_placeholder("<your-token>"));
        assert!(is_placeholder("YOUR_API_KEY"));
        assert!(looks_real_secret("ghp_a8Kd92LmQz71Xc0PbN4t"));
        assert!(is_secret_name("GITHUB_PERSONAL_ACCESS_TOKEN"));
        assert!(!is_secret_name("MAX_MCP_OUTPUT_TOKENS"));
        assert!(!is_secret_name("CLAUDE_CODE_API_KEY_HELPER_TTL_MS"));
    }
}
