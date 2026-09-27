//! Compiler for the privacy.sexy collection files (AGPL-3.0 data in
//! `data/washmybelly/privacy-sexy/`), written from the format documentation
//! (`docs/collection-files.md`, `docs/templating.md` upstream).
//!
//! - A collection has categories, scripts and functions. A script is inline
//!   (`code` + `revertCode`) or calls functions (`call`, one or a list).
//! - A function is inline code or calls other functions. Arguments of a
//!   nested call are templates compiled with the caller's arguments; an
//!   empty result drops an optional argument and fails a required one.
//! - Templates: `{{ $param }}`, `{{ $param | pipe | pipe }}`,
//!   `{{ with $param }} ... {{ . }} ... {{ end }}`. Only the outermost
//!   expressions are evaluated per pass, and the result is compiled again
//!   until no expression is left (so `with` blocks can hold substitutions).
//! - Pipes: `escapeDoubleQuotes` (`"` → `"^""`) and `inlinePowerShell`
//!   (joins a multi-line PowerShell script into one line).
//! - Segments of a call sequence are joined with `\n`, the revert code in
//!   the same order as the code.
//!
//! The output is compared byte for byte with the original TypeScript
//! compiler by the golden tests (`tests/washmybelly_fixtures/privacy/`).

use std::collections::{BTreeMap, HashMap};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;

// ── Collection model (mirrors `collection.yaml.d.ts`) ──────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Collection {
    pub os: String,
    pub actions: Vec<CategoryData>,
    #[serde(default)]
    pub functions: Vec<FunctionData>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CategoryData {
    pub category: String,
    pub children: Vec<Node>,
    #[serde(default)]
    pub docs: Docs,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Node {
    Category(CategoryData),
    Script(ScriptData),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptData {
    pub name: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default, rename = "revertCode")]
    pub revert_code: Option<String>,
    #[serde(default)]
    pub call: Option<CallsData>,
    #[serde(default)]
    pub recommend: Option<String>,
    #[serde(default)]
    pub docs: Docs,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(untagged)]
pub enum Docs {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl Docs {
    pub fn text(&self) -> String {
        match self {
            Docs::None => String::new(),
            Docs::One(s) => s.clone(),
            Docs::Many(v) => v.join("\n\n"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum CallsData {
    Many(Vec<CallData>),
    One(CallData),
}

#[derive(Debug, Clone, Deserialize)]
pub struct CallData {
    pub function: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunctionData {
    pub name: String,
    #[serde(default)]
    pub parameters: Vec<ParamData>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default, rename = "revertCode")]
    pub revert_code: Option<String>,
    #[serde(default)]
    pub call: Option<CallsData>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ParamData {
    pub name: String,
    #[serde(default)]
    pub optional: bool,
}

pub fn parse_collection(yaml: &str) -> Result<Collection, String> {
    serde_yaml::from_str(yaml).map_err(|e| format!("privacy.sexy collection: {e}"))
}

// ── Calls ─────────────────────────────────────────────────────────────

/// Argument values by parameter name (never empty).
pub type Args = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub function: String,
    pub args: Args,
}

fn scalar(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A `call` block as a call sequence; every argument must be a non-empty string.
pub fn parse_calls(data: &CallsData) -> Result<Vec<Call>, String> {
    let list: Vec<&CallData> = match data {
        CallsData::One(c) => vec![c],
        CallsData::Many(v) => v.iter().collect(),
    };
    if list.is_empty() {
        return Err("empty function call sequence".into());
    }
    list.into_iter()
        .map(|c| {
            let mut args = Args::new();
            for (k, v) in &c.parameters {
                let s = scalar(v)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("{}: parameter '{k}' is missing a value", c.function))?;
                args.insert(k.clone(), s);
            }
            Ok(Call {
                function: c.function.clone(),
                args,
            })
        })
        .collect()
}

/// Code and revert code of one script or call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Compiled {
    pub code: String,
    /// Empty when there is nothing to revert.
    pub revert: String,
}

/// What a walker visitor wants done with a call.
pub enum Visit {
    /// The visitor handled it; do not look inside.
    Stop,
    /// Look at the calls inside (for functions that call others).
    Descend,
}

const MAX_DEPTH: usize = 64;

pub struct Compiler<'a> {
    functions: HashMap<&'a str, &'a FunctionData>,
}

impl<'a> Compiler<'a> {
    pub fn new(c: &'a Collection) -> Compiler<'a> {
        Compiler {
            functions: c.functions.iter().map(|f| (f.name.as_str(), f)).collect(),
        }
    }

    pub fn function(&self, name: &str) -> Result<&'a FunctionData, String> {
        self.functions
            .get(name)
            .copied()
            .ok_or_else(|| format!("called function is not defined: \"{name}\""))
    }

    /// A script's code and revert code, exactly as privacy.sexy prints them.
    pub fn compile_script(&self, s: &ScriptData) -> Result<Compiled, String> {
        let Some(calls) = &s.call else {
            return Ok(Compiled {
                code: s.code.clone().unwrap_or_default(),
                revert: s.revert_code.clone().unwrap_or_default(),
            });
        };
        let calls = parse_calls(calls).map_err(|e| format!("{}: {e}", s.name))?;
        self.compile_calls(&calls).map_err(|e| format!("{}: {e}", s.name))
    }

    pub fn compile_calls(&self, calls: &[Call]) -> Result<Compiled, String> {
        let mut segments = Vec::new();
        for c in calls {
            self.compile_call(c, 0, &mut segments)?;
        }
        if segments.is_empty() {
            return Err("missing segments".into());
        }
        let join = |parts: Vec<&str>| {
            parts
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(Compiled {
            code: join(segments.iter().map(|s| s.code.as_str()).collect()),
            revert: join(segments.iter().map(|s| s.revert.as_str()).collect()),
        })
    }

    fn check_params(&self, f: &FunctionData, call: &Call) -> Result<(), String> {
        let unexpected: Vec<&str> = call
            .args
            .keys()
            .filter(|k| !f.parameters.iter().any(|p| &p.name == *k))
            .map(String::as_str)
            .collect();
        if unexpected.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Function \"{}\" has unexpected parameter(s) provided: \"{}\"",
                f.name,
                unexpected.join("\", \"")
            ))
        }
    }

    fn compile_call(&self, call: &Call, depth: usize, out: &mut Vec<Compiled>) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(format!("calls nested too deep at {}", call.function));
        }
        let f = self.function(&call.function)?;
        self.check_params(f, call)?;
        if let Some(code) = f.code.as_deref().filter(|c| !c.is_empty()) {
            let revert = match f.revert_code.as_deref() {
                Some(r) if !r.is_empty() => expand(r, &call.args)?,
                _ => String::new(),
            };
            out.push(Compiled {
                code: expand(code, &call.args)?,
                revert,
            });
            return Ok(());
        }
        let nested = f
            .call
            .as_ref()
            .ok_or_else(|| format!("function {} has neither code nor call", f.name))?;
        for n in parse_calls(nested)? {
            let compiled = self.nested_call(&n, &call.args).map_err(|e| {
                format!("Failed to call '{}' (callee function) from '{}' (caller function): {e}", n.function, call.function)
            })?;
            self.compile_call(&compiled, depth + 1, out)?;
        }
        Ok(())
    }

    /// A call written inside a function, with its arguments compiled
    /// against the caller's arguments. Empty optional arguments are dropped.
    pub fn nested_call(&self, nested: &Call, parent: &Args) -> Result<Call, String> {
        let f = self.function(&nested.function)?;
        let mut args = Args::new();
        for (k, v) in &nested.args {
            let value = expand(v, parent).map_err(|e| format!("Error when compiling argument for \"{k}\": {e}"))?;
            if value.is_empty() {
                let required = f.parameters.iter().any(|p| &p.name == k && !p.optional);
                if required {
                    return Err(format!("Compilation resulted in empty value for required parameter: \"{k}\""));
                }
                continue;
            }
            args.insert(k.clone(), value);
        }
        Ok(Call {
            function: nested.function.clone(),
            args,
        })
    }

    /// Walk a call sequence the way the compiler would, letting the visitor
    /// stop at the functions it understands. `inline` tells the visitor the
    /// function is code, not calls (nothing to descend into).
    pub fn walk(&self, calls: &[Call], visit: &mut dyn FnMut(&Call, bool) -> Visit) -> Result<(), String> {
        for c in calls {
            self.walk_one(c, 0, visit)?;
        }
        Ok(())
    }

    fn walk_one(&self, call: &Call, depth: usize, visit: &mut dyn FnMut(&Call, bool) -> Visit) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(format!("calls nested too deep at {}", call.function));
        }
        let f = self.function(&call.function)?;
        let inline = f.code.as_deref().is_some_and(|c| !c.is_empty());
        if let Visit::Stop = visit(call, inline) {
            return Ok(());
        }
        if inline {
            return Ok(());
        }
        if let Some(nested) = &f.call {
            for n in parse_calls(nested)? {
                let compiled = self.nested_call(&n, &call.args)?;
                self.walk_one(&compiled, depth + 1, visit)?;
            }
        }
        Ok(())
    }
}

// ── Templates ─────────────────────────────────────────────────────────

const PIPELINE: &str = r"((?:\|\s*\b[a-zA-Z]+\b\s*)*)";

static SUBST: Lazy<Regex> = Lazy::new(|| Regex::new(&format!(r"\{{\{{\s*\$([^|\s]+)\s*{PIPELINE}\s*\}}\}}")).unwrap());
static WITH_START: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{\{\s*with\s+\$([^|\s]+)\s*\}\}\s*").unwrap());
static WITH_END: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*\{\{\s*end\s*\}\}").unwrap());
static CONTEXT_VAR: Lazy<Regex> = Lazy::new(|| Regex::new(&format!(r"\{{\{{\s*\.\s*{PIPELINE}\s*\}}\}}")).unwrap());

#[derive(Debug, Clone)]
enum Kind {
    Subst {
        name: String,
        pipeline: String,
    },
    With {
        name: String,
        /// Scope as byte range of the code.
        scope: (usize, usize),
        /// `{{ . }}` inside, as absolute byte ranges with their pipeline.
        vars: Vec<(usize, usize, String)>,
    },
}

#[derive(Debug, Clone)]
struct Expr {
    start: usize,
    end: usize,
    kind: Kind,
}

impl Expr {
    fn inside_of(&self, other: &Expr) -> bool {
        !(self.start == other.start && self.end == other.end) && other.start <= self.start && other.end >= self.end
    }

    fn intersects(&self, other: &Expr) -> bool {
        (other.start < self.end && other.end > self.start) || (self.end > other.start && other.start >= self.start)
    }
}

fn find_expressions(code: &str) -> Result<Vec<Expr>, String> {
    let mut out: Vec<Expr> = SUBST
        .captures_iter(code)
        .map(|c| {
            let m = c.get(0).unwrap();
            Expr {
                start: m.start(),
                end: m.end(),
                kind: Kind::Subst {
                    name: c[1].to_string(),
                    pipeline: c.get(2).map_or("", |p| p.as_str()).to_string(),
                },
            }
        })
        .collect();
    out.extend(find_with(code)?);
    Ok(out)
}

enum WithToken {
    Start(String),
    End,
    Var(String),
}

fn find_with(code: &str) -> Result<Vec<Expr>, String> {
    let mut tokens: Vec<(usize, usize, WithToken)> = Vec::new();
    for c in WITH_START.captures_iter(code) {
        let m = c.get(0).unwrap();
        tokens.push((m.start(), m.end(), WithToken::Start(c[1].to_string())));
    }
    for m in WITH_END.find_iter(code) {
        tokens.push((m.start(), m.end(), WithToken::End));
    }
    for c in CONTEXT_VAR.captures_iter(code) {
        let m = c.get(0).unwrap();
        tokens.push((m.start(), m.end(), WithToken::Var(c.get(1).map_or("", |p| p.as_str()).to_string())));
    }
    tokens.sort_by_key(|t| t.0);
    // Open `with` blocks: (start, end of the start statement, name, vars).
    type Open = (usize, usize, String, Vec<(usize, usize, String)>);
    let mut open: Vec<Open> = Vec::new();
    let mut out = Vec::new();
    for (start, end, tok) in tokens {
        match tok {
            WithToken::Start(name) => open.push((start, end, name, Vec::new())),
            WithToken::Var(pipeline) => {
                let top = open.last_mut().ok_or("Context variable before `with` statement.")?;
                top.3.push((start, end, pipeline));
            }
            WithToken::End => {
                let (s, scope_start, name, vars) = open.pop().ok_or("Redundant `end` statement, missing `with`?")?;
                out.push(Expr {
                    start: s,
                    end,
                    kind: Kind::With {
                        name,
                        scope: (scope_start, start),
                        vars,
                    },
                });
            }
        }
    }
    if !open.is_empty() {
        return Err("Missing `end` statement, forgot `{{ end }}?".into());
    }
    Ok(out)
}

/// Compile a template with the arguments until no expression is left.
pub fn expand(code: &str, args: &Args) -> Result<String, String> {
    let mut cur = code.to_string();
    for _ in 0..256 {
        if cur.is_empty() {
            return Ok(cur);
        }
        let exprs = find_expressions(&cur)?;
        if exprs.is_empty() {
            return Ok(cur);
        }
        cur = evaluate_pass(&cur, &exprs, args)?;
    }
    Err("template expands forever".into())
}

fn evaluate_pass(code: &str, exprs: &[Expr], args: &Args) -> Result<String, String> {
    for (i, a) in exprs.iter().enumerate() {
        if a.end > code.len() {
            return Err("expression out of range".into());
        }
        for (j, b) in exprs.iter().enumerate() {
            if i == j {
                continue;
            }
            if a.start == b.start && a.end == b.end {
                return Err("Instructions at same position".into());
            }
            if a.intersects(b) && !a.inside_of(b) && !b.inside_of(a) {
                return Err(format!("Instructions intersecting unexpectedly: {}", &code[a.start.min(b.start)..a.end.max(b.end)]));
            }
        }
    }
    let mut outer: Vec<&Expr> = exprs
        .iter()
        .filter(|e| !exprs.iter().any(|o| e.inside_of(o)))
        .collect();
    outer.sort_by_key(|e| e.start);
    let mut out = String::with_capacity(code.len());
    let mut index = 0;
    for e in outer {
        out.push_str(&code[index..e.start]);
        out.push_str(&evaluate(code, e, args)?);
        index = e.end;
    }
    out.push_str(&code[index..]);
    Ok(out)
}

fn evaluate(code: &str, e: &Expr, args: &Args) -> Result<String, String> {
    match &e.kind {
        Kind::Subst { name, pipeline } => {
            let value = args.get(name).ok_or_else(|| {
                format!("Some parameters are used in code, but their values are not provided: {name}")
            })?;
            if pipeline.is_empty() {
                Ok(value.clone())
            } else {
                apply_pipeline(value, pipeline)
            }
        }
        Kind::With { name, scope, vars } => {
            let Some(value) = args.get(name).filter(|v| !v.is_empty()) else {
                return Ok(String::new());
            };
            let mut out = String::new();
            let mut index = scope.0;
            for (s, en, pipeline) in vars {
                out.push_str(&code[index..*s]);
                if pipeline.is_empty() {
                    out.push_str(value);
                } else {
                    out.push_str(&apply_pipeline(value, pipeline)?);
                }
                index = *en;
            }
            out.push_str(&code[index..scope.1]);
            Ok(out)
        }
    }
}

// ── Pipes ─────────────────────────────────────────────────────────────

fn apply_pipeline(value: &str, pipeline: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("missing value".into());
    }
    if !pipeline.trim_start().starts_with('|') {
        return Err("pipeline does not start with pipe".into());
    }
    let mut v = value.to_string();
    for name in pipeline.trim().split('|').skip(1).map(str::trim) {
        v = match name {
            "escapeDoubleQuotes" => escape_double_quotes(&v),
            "inlinePowerShell" => inline_powershell(&v),
            "" => return Err("empty pipe name".into()),
            other => return Err(format!("Unknown pipe: \"{other}\"")),
        };
    }
    Ok(v)
}

/// `"` → `"^""`: the one escape that survives cmd.exe and PowerShell.
pub fn escape_double_quotes(raw: &str) -> String {
    raw.replace('"', "\"^\"\"")
}

/// Lines split on `\r\n`, `\r` or `\n`.
fn split_lines(text: &str) -> Vec<&str> {
    static NL: Lazy<Regex> = Lazy::new(|| Regex::new(r"\r\n|\r|\n").unwrap());
    NL.split(text).collect()
}

/// Join a multi-line PowerShell script into one line: `#` comments become
/// `<# #>`, here-strings become strings, backtick and bracket
/// continuations merge, and the remaining lines end with `;`.
pub fn inline_powershell(code: &str) -> String {
    if code.is_empty() || !(code.contains('\n') || code.contains('\r')) {
        return code.to_string();
    }
    let s = inline_comments(code);
    let s = merge_here_strings(&s);
    let s = merge_backticks(&s);
    let s = merge_brackets(&s);
    merge_new_lines(&s)
}

/// JS `.` does not match line terminators; Rust's only skips `\n`.
const DOT: &str = r"[^\n\r\u{2028}\u{2029}]";

fn inline_comments(code: &str) -> String {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(&format!(r"<#{DOT}*?#>|#({DOT}*)")).unwrap());
    RE.replace_all(code, |c: &regex::Captures| match c.get(1) {
        None => c[0].to_string(),
        Some(comment) => {
            let v = comment.as_str().trim();
            if v.is_empty() {
                "<##>".to_string()
            } else {
                format!("<# {v} #>")
            }
        }
    })
    .into_owned()
}

fn merge_here_strings(code: &str) -> String {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"@'\s*(?:\r\n|\r|\n)((?:[^\u{2028}\u{2029}])+?)(?:\r\n|\r|\n)'@|@"\s*(?:\r\n|\r|\n)((?:[^\u{2028}\u{2029}])+?)(?:\r\n|\r|\n)"@"#).unwrap()
    });
    RE.replace_all(code, |c: &regex::Captures| {
        let (quote, scope) = match (c.get(1), c.get(2)) {
            (Some(s), _) => ('\'', s.as_str()),
            (_, Some(s)) => ('"', s.as_str()),
            _ => return c[0].to_string(),
        };
        let (around, escaped, sep) = if quote == '\'' {
            ("'", "''", "'+\"`r`n\"+'")
        } else {
            ("\"", "`\"", "`r`n")
        };
        let body = scope.replace(quote, escaped);
        format!("{around}{}{around}", split_lines(&body).join(sep))
    })
    .into_owned()
}

fn merge_backticks(code: &str) -> String {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r" +`\s*(?:\r\n|\r|\n)\s*").unwrap());
    RE.replace_all(code, " ").into_owned()
}

fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn merge_brackets(code: &str) -> String {
    // Opening bracket (the upstream lookbehind `(?<=.*)` always holds).
    static OPEN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*\{[\r\n][\s\r\n]*").unwrap());
    let s = OPEN.replace_all(code, " { ").into_owned();
    // Closing bracket followed by a continuation keyword.
    static CLOSE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*\}[\r\n][\s\r\n]*").unwrap());
    let s = replace_with_lookahead(&s, &CLOSE, &["elseif", "else", "catch", "finally", "until"], None);
    // `do { ... }` + newline + `while`.
    static DO_CLOSE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\r\n\s]*\}[\r\n][\r\n\s]*").unwrap());
    replace_with_lookahead(&s, &DO_CLOSE, &["while"], Some(do_block_before))
}

/// `(?<=do\s*{.*)` at `pos`: a `{` earlier on the same line, with only
/// whitespace between it and a preceding `do`.
fn do_block_before(text: &str, pos: usize) -> bool {
    let before = &text[..pos];
    let line_start = before.rfind(['\n', '\r', '\u{2028}', '\u{2029}']).map_or(0, |i| i + 1);
    before[line_start..]
        .char_indices()
        .filter(|(_, c)| *c == '{')
        .any(|(i, _)| before[..line_start + i].trim_end_matches(is_js_space).ends_with("do"))
}

/// `re` followed by one of `ahead` (a lookahead the regex crate lacks),
/// optionally with a lookbehind test at the match start. A failed candidate
/// resumes the search just after its `}` so a later match can still start
/// inside the whitespace the candidate covered, as a backtracking engine does.
fn replace_with_lookahead(text: &str, re: &Regex, ahead: &[&str], behind: Option<fn(&str, usize) -> bool>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut pos = 0;
    while pos <= text.len() {
        let Some(m) = re.find_at(text, pos) else { break };
        let brace = m.start() + m.as_str().find('}').unwrap_or(0);
        let rest = &text[m.end()..];
        let ok_ahead = ahead.iter().any(|w| rest.starts_with(w));
        let start = match behind {
            None => Some(m.start()),
            // Leftmost start inside the leading whitespace where the lookbehind holds.
            Some(test) => (m.start()..=brace).filter(|i| text.is_char_boundary(*i)).find(|i| test(text, *i)),
        };
        match start {
            Some(s) if ok_ahead => {
                out.push_str(&text[copied..s]);
                out.push_str(" } ");
                copied = m.end();
                pos = m.end();
            }
            _ => pos = brace + 1,
        }
    }
    out.push_str(&text[copied..]);
    out
}

fn merge_new_lines(code: &str) -> String {
    let lines: Vec<&str> = split_lines(code)
        .into_iter()
        .map(|l| l.trim_matches(is_js_space))
        .filter(|l| !l.is_empty())
        .collect();
    let n = lines.len();
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if i + 1 == n || l.ends_with(';') {
                l.to_string()
            } else {
                format!("{l};")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, &str)]) -> Args {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn substitutes_parameters_with_and_without_spaces() {
        let a = args(&[("name", "x"), ("v", "1")]);
        assert_eq!(expand("a {{ $name }} b {{$v}}", &a).unwrap(), "a x b 1");
    }

    #[test]
    fn a_missing_required_parameter_is_an_error() {
        let e = expand("{{ $nope }}", &args(&[])).unwrap_err();
        assert!(e.contains("nope"));
    }

    #[test]
    fn pipes_apply_in_order() {
        let a = args(&[("c", "Write-Host \"hi\"\n# note\nexit")]);
        assert_eq!(
            expand("-Command \"{{ $c | inlinePowerShell | escapeDoubleQuotes }}\"", &a).unwrap(),
            "-Command \"Write-Host \"^\"\"hi\"^\"\"; <# note #>; exit\""
        );
        assert!(expand("{{ $c | nothing }}", &a).unwrap_err().contains("Unknown pipe"));
    }

    #[test]
    fn with_renders_only_when_the_argument_is_given() {
        let t = "start {{ with $opt }}\n  value={{ . }} and {{ $req }}\n{{ end }} stop";
        assert_eq!(expand(t, &args(&[("req", "r")])).unwrap(), "start  stop");
        assert_eq!(expand(t, &args(&[("req", "r"), ("opt", "o")])).unwrap(), "start value=o and r stop");
    }

    #[test]
    fn nested_with_blocks_bind_the_innermost_dot() {
        let t = "{{ with $a }}A={{ . }} {{ with $b }}B={{ . }}{{ end }}{{ end }}";
        assert_eq!(expand(t, &args(&[("a", "1"), ("b", "2")])).unwrap(), "A=1 B=2");
        assert_eq!(expand(t, &args(&[("a", "1")])).unwrap(), "A=1");
    }

    #[test]
    fn unbalanced_with_is_an_error() {
        assert!(expand("{{ with $a }} x", &args(&[("a", "1")])).is_err());
        assert!(expand("x {{ end }}", &args(&[])).is_err());
        assert!(expand("{{ . }}", &args(&[])).is_err());
    }

    #[test]
    fn inline_powershell_merges_here_strings_backticks_and_brackets() {
        let code = "$x = @'\nline 'one'\nline two\n'@\nGet-Thing `\n   -Flag\nif ($a) {\n  b\n}\nelse {\n  c\n}";
        assert_eq!(
            inline_powershell(code),
            "$x = 'line ''one'''+\"`r`n\"+'line two'; Get-Thing -Flag; if ($a) { b } else { c }"
        );
        assert_eq!(inline_powershell("one line # c"), "one line # c");
        assert_eq!(inline_powershell("a\n#\nb <# keep #>"), "a; <##>; b <# keep #>");
    }

    #[test]
    fn do_while_brackets_merge() {
        assert_eq!(inline_powershell("do {\n  x\n}\nwhile ($y)"), "do { x } while ($y)");
    }

    const MINI: &str = r#"
os: windows
actions:
  - category: Top
    children:
      - name: Inline item
        code: echo 1
        revertCode: echo 0
      - name: Called item
        recommend: standard
        call:
          - function: Outer
            parameters:
              value: v1
          - function: Leaf
            parameters:
              text: second
functions:
  - name: Outer
    parameters:
      - name: value
      - name: extra
        optional: true
    call:
      function: Leaf
      parameters:
        text: 'outer {{ $value }}'
        undo: '{{ with $extra }}{{ . }}{{ end }}'
  - name: Leaf
    parameters:
      - name: text
      - name: undo
        optional: true
    code: 'echo {{ $text }}'
    revertCode: '{{ with $undo }}undo {{ . }}{{ end }}'
  - name: Loop
    call:
      function: Loop
"#;

    #[test]
    fn calls_compile_recursively_and_drop_empty_optionals() {
        let c = parse_collection(MINI).unwrap();
        let comp = Compiler::new(&c);
        let Node::Script(s) = &c.actions[0].children[1] else { panic!() };
        let out = comp.compile_script(s).unwrap();
        assert_eq!(out.code, "echo outer v1\necho second");
        assert_eq!(out.revert, "");
        let calls = vec![Call {
            function: "Outer".into(),
            args: args(&[("value", "a"), ("extra", "b")]),
        }];
        assert_eq!(comp.compile_calls(&calls).unwrap().revert, "undo b");
    }

    #[test]
    fn call_errors_are_reported() {
        let c = parse_collection(MINI).unwrap();
        let comp = Compiler::new(&c);
        let bad = |f: &str, a: &[(&str, &str)]| {
            comp.compile_calls(&[Call {
                function: f.into(),
                args: args(a),
            }])
            .unwrap_err()
        };
        assert!(bad("Missing", &[]).contains("not defined"));
        assert!(bad("Leaf", &[("text", "x"), ("zzz", "1")]).contains("unexpected parameter"));
        assert!(bad("Leaf", &[]).contains("text"));
        assert!(bad("Loop", &[]).contains("too deep"));
    }

    #[test]
    fn inline_scripts_pass_through() {
        let c = parse_collection(MINI).unwrap();
        let Node::Script(s) = &c.actions[0].children[0] else { panic!() };
        let out = Compiler::new(&c).compile_script(s).unwrap();
        assert_eq!(out, Compiled { code: "echo 1".into(), revert: "echo 0".into() });
    }
}
