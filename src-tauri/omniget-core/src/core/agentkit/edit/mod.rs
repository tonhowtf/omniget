//! Format editors that patch a document by path and keep everything else byte for
//! byte: comments, key order, indentation, trailing commas. JSON and JSONC share
//! one engine ([`jsonc`]); TOML ([`toml`]) and YAML ([`yaml`]) have their own,
//! and each also parses its format into a [`serde_json::Value`] for reading.
//!
//! The contract every editor keeps: an insertion made by the editor followed by
//! the removal of the same path gives back the original text exactly. The
//! uninstaller relies on it to leave no byte behind.

pub mod json;
pub mod jsonc;
pub mod toml;
pub mod yaml;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AgentkitError, Result};

/// One step of a document path: an object key or an array index.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Seg {
    Index(usize),
    Key(String),
}

impl Seg {
    pub fn key(k: impl Into<String>) -> Seg {
        Seg::Key(k.into())
    }
}

impl std::fmt::Display for Seg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Seg::Key(k) => write!(f, "{k}"),
            Seg::Index(i) => write!(f, "[{i}]"),
        }
    }
}

/// Builds a key-only path.
pub fn keys<I, S>(parts: I) -> Vec<Seg>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    parts.into_iter().map(|s| Seg::Key(s.into())).collect()
}

pub fn path_display(path: &[Seg]) -> String {
    let mut out = String::new();
    for (i, s) in path.iter().enumerate() {
        match s {
            Seg::Key(k) => {
                if i > 0 {
                    out.push('.');
                }
                out.push_str(k);
            }
            Seg::Index(n) => out.push_str(&format!("[{n}]")),
        }
    }
    out
}

/// Document formats the writer can patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocFormat {
    Json,
    Jsonc,
    Toml,
    Yaml,
    /// Plain text / Markdown with marker-delimited blocks (rules in CLAUDE.md, AGENTS.md).
    Markdown,
}

impl DocFormat {
    pub fn parse(s: &str) -> Option<DocFormat> {
        match s {
            "json" => Some(DocFormat::Json),
            "jsonc" => Some(DocFormat::Jsonc),
            "toml" => Some(DocFormat::Toml),
            "yaml" | "yml" => Some(DocFormat::Yaml),
            "md" | "markdown" | "text" => Some(DocFormat::Markdown),
            _ => None,
        }
    }

    /// Text of a brand-new, empty document of this format.
    pub fn empty_doc(self) -> &'static str {
        match self {
            DocFormat::Json | DocFormat::Jsonc => "{}\n",
            DocFormat::Toml | DocFormat::Yaml | DocFormat::Markdown => "",
        }
    }
}

/// What a `set` did, so it can be undone exactly.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SetOutcome {
    /// Index in the path of the first segment that did not exist before (the
    /// whole subtree from there on is ours). `None` when the full path existed.
    pub created_at: Option<usize>,
    /// The value that was replaced, when the full path existed.
    pub prev: Option<Value>,
}

/// A document open for patching.
pub trait Editor {
    fn text(&self) -> &str;
    /// Current value at `path` (whole document for an empty path).
    fn get(&self, path: &[Seg]) -> Option<Value>;
    /// Sets `path` to `value`, creating missing parents as objects.
    fn set(&mut self, path: &[Seg], value: &Value) -> Result<SetOutcome>;
    /// Removes `path`. Returns the removed value, `None` if it was not there.
    fn remove(&mut self, path: &[Seg]) -> Result<Option<Value>>;
    /// Appends to the array at `path` (created when missing). Returns the outcome
    /// of the array creation when it had to be created.
    fn push(&mut self, path: &[Seg], value: &Value) -> Result<Option<SetOutcome>>;
    /// Removes the first element of the array at `path` equal to `value`.
    fn remove_item(&mut self, path: &[Seg], value: &Value) -> Result<bool>;

    /// The whole document as a value.
    fn value(&self) -> Value {
        self.get(&[]).unwrap_or(Value::Null)
    }
}

/// Opens `text` with the editor for `format`.
pub fn open(format: DocFormat, text: &str) -> Result<Box<dyn Editor>> {
    Ok(match format {
        DocFormat::Json => Box::new(json::JsonDoc::parse(text)?),
        DocFormat::Jsonc => Box::new(jsonc::JsoncDoc::parse(text)?),
        DocFormat::Toml => Box::new(toml::TomlDoc::parse(text)?),
        DocFormat::Yaml => Box::new(yaml::YamlDoc::parse(text)?),
        DocFormat::Markdown => {
            return Err(AgentkitError::new(
                "AGENTKIT_EDIT",
                "markdown documents are edited as text blocks, not by path",
            ))
        }
    })
}

/// Reads a whole document of any supported format into a value.
pub fn parse_value(format: DocFormat, text: &str) -> Result<Value> {
    match format {
        DocFormat::Json | DocFormat::Jsonc => jsonc::parse_value(text),
        DocFormat::Toml => toml::parse(text),
        DocFormat::Yaml => yaml::parse(text),
        DocFormat::Markdown => Ok(Value::String(text.to_string())),
    }
}

/// Order in which well-known keys are written in new objects, so a written MCP
/// server reads `type, command, args, env` instead of alphabetically.
pub(crate) fn key_rank(key: &str) -> usize {
    const ORDER: &[&str] = &[
        "$schema",
        "version",
        "name",
        "id",
        "type",
        "transport",
        "description",
        "mode",
        "kind",
        "matcher",
        "event",
        "trigger",
        "command",
        "cmd",
        "bash",
        "powershell",
        "args",
        "env",
        "envs",
        "environment",
        "env_vars",
        "cwd",
        "url",
        "httpUrl",
        "serverUrl",
        "uri",
        "headers",
        "http_headers",
        "bearer_token_env_var",
        "bearerTokenEnvVar",
        "hooks",
        "timeout",
        "timeoutSec",
        "enabled",
        "disabled",
        "autoApprove",
    ];
    ORDER.iter().position(|k| *k == key).unwrap_or(ORDER.len())
}

/// Object entries in writing order: ranked keys first, the rest alphabetically.
pub(crate) fn ordered_entries(map: &serde_json::Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut v: Vec<(&String, &Value)> = map.iter().collect();
    v.sort_by(|a, b| key_rank(a.0).cmp(&key_rank(b.0)).then_with(|| a.0.cmp(b.0)));
    v
}

/// Walks `value` along `path`.
pub fn value_at<'a>(value: &'a Value, path: &[Seg]) -> Option<&'a Value> {
    let mut cur = value;
    for seg in path {
        cur = match (seg, cur) {
            (Seg::Key(k), Value::Object(m)) => m.get(k)?,
            (Seg::Index(i), Value::Array(a)) => a.get(*i)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Builds `{p1: {p2: … value}}` for the missing tail of a path.
pub(crate) fn nest(path: &[Seg], value: &Value) -> Value {
    let mut out = value.clone();
    for seg in path.iter().rev() {
        match seg {
            Seg::Key(k) => {
                let mut m = serde_json::Map::new();
                m.insert(k.clone(), out);
                out = Value::Object(m);
            }
            Seg::Index(_) => out = Value::Array(vec![out]),
        }
    }
    out
}

pub(crate) fn edit_err(msg: impl Into<String>) -> AgentkitError {
    AgentkitError::new("AGENTKIT_EDIT", msg)
}

pub(crate) fn parse_err(what: &str, line: usize, msg: impl std::fmt::Display) -> AgentkitError {
    AgentkitError::new("AGENTKIT_PARSE", format!("{what} line {line}: {msg}"))
}

/// Line number (1-based) of a byte offset.
pub(crate) fn line_of(text: &str, pos: usize) -> usize {
    text[..pos.min(text.len())].matches('\n').count() + 1
}

/// Marker-delimited block in a Markdown/text file. The block carries its id so
/// the uninstaller finds it again.
pub mod textblock {
    pub fn begin(id: &str) -> String {
        format!("<!-- omniget:agentkit:begin {id} -->")
    }
    pub fn end(id: &str) -> String {
        format!("<!-- omniget:agentkit:end {id} -->")
    }

    /// Appends the block (or replaces it when the id is already there).
    pub fn upsert(text: &str, id: &str, content: &str) -> String {
        let body = content.trim_end_matches('\n');
        let block = format!("{}\n{}\n{}\n", begin(id), body, end(id));
        if let Some((s, e)) = find(text, id) {
            let mut out = String::with_capacity(text.len() + block.len());
            out.push_str(&text[..s]);
            out.push_str(&block);
            out.push_str(&text[e..]);
            return out;
        }
        let mut out = text.to_string();
        if out.is_empty() {
            out.push_str(&block);
        } else if out.ends_with("\n\n") {
            out.push_str(&block);
        } else if out.ends_with('\n') {
            out.push('\n');
            out.push_str(&block);
        } else {
            out.push_str("\n\n");
            out.push_str(&block);
        }
        out
    }

    /// Byte range of the block including its trailing newline.
    pub fn find(text: &str, id: &str) -> Option<(usize, usize)> {
        let b = begin(id);
        let e = end(id);
        let s = text.find(&b)?;
        let rel = text[s..].find(&e)?;
        let mut end = s + rel + e.len();
        if text[end..].starts_with('\n') {
            end += 1;
        }
        Some((s, end))
    }

    /// Content between the markers.
    pub fn content(text: &str, id: &str) -> Option<String> {
        let (s, e) = find(text, id)?;
        let block = &text[s..e];
        let inner_start = block.find('\n')? + 1;
        let inner_end = block.rfind(&end(id))?;
        Some(
            block[inner_start..inner_end]
                .trim_end_matches('\n')
                .to_string(),
        )
    }

    /// Removes the block and the blank line `upsert` put before it.
    pub fn remove(text: &str, id: &str) -> Option<String> {
        let (mut s, e) = find(text, id)?;
        if e == text.len() && s > 0 {
            if text[..s].ends_with("\n\n") {
                s -= 1;
            }
        } else if s > 0 && text[..s].ends_with("\n\n") && text[e..].starts_with('\n') {
            s -= 1;
        }
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..s]);
        out.push_str(&text[e..]);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::textblock;

    #[test]
    fn textblock_round_trip() {
        for original in ["", "# Rules\n", "# Rules\n\nmore\n", "no newline"] {
            let with = textblock::upsert(original, "x1", "Use tabs.");
            assert!(with.contains("Use tabs."));
            assert_eq!(
                textblock::content(&with, "x1").as_deref(),
                Some("Use tabs.")
            );
            let again = textblock::upsert(&with, "x1", "Use tabs.");
            assert_eq!(with, again);
            let back = textblock::remove(&with, "x1").unwrap();
            if original == "no newline" {
                assert_eq!(back, "no newline\n");
            } else {
                assert_eq!(back, original);
            }
        }
    }
}
