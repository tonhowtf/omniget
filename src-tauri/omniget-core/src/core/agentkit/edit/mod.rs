pub mod json;
pub mod jsonc;
pub mod toml;
pub mod yaml;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AgentkitError, Result};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocFormat {
    Json,
    Jsonc,
    Toml,
    Yaml,
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

    pub fn empty_doc(self) -> &'static str {
        match self {
            DocFormat::Json | DocFormat::Jsonc => "{}\n",
            DocFormat::Toml | DocFormat::Yaml | DocFormat::Markdown => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SetOutcome {
    pub created_at: Option<usize>,
    pub prev: Option<Value>,
}

pub trait Editor {
    fn text(&self) -> &str;
    fn get(&self, path: &[Seg]) -> Option<Value>;
    fn set(&mut self, path: &[Seg], value: &Value) -> Result<SetOutcome>;
    fn remove(&mut self, path: &[Seg]) -> Result<Option<Value>>;
    fn push(&mut self, path: &[Seg], value: &Value) -> Result<Option<SetOutcome>>;
    fn remove_item(&mut self, path: &[Seg], value: &Value) -> Result<bool>;

    fn value(&self) -> Value {
        self.get(&[]).unwrap_or(Value::Null)
    }
}

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

pub fn parse_value(format: DocFormat, text: &str) -> Result<Value> {
    match format {
        DocFormat::Json | DocFormat::Jsonc => jsonc::parse_value(text),
        DocFormat::Toml => toml::parse(text),
        DocFormat::Yaml => yaml::parse(text),
        DocFormat::Markdown => Ok(Value::String(text.to_string())),
    }
}

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

pub(crate) fn ordered_entries(map: &serde_json::Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut v: Vec<(&String, &Value)> = map.iter().collect();
    v.sort_by(|a, b| key_rank(a.0).cmp(&key_rank(b.0)).then_with(|| a.0.cmp(b.0)));
    v
}

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

pub(crate) fn line_of(text: &str, pos: usize) -> usize {
    text[..pos.min(text.len())].matches('\n').count() + 1
}

pub mod textblock {
    pub fn begin(id: &str) -> String {
        format!("<!-- omniget:agentkit:begin {id} -->")
    }
    pub fn end(id: &str) -> String {
        format!("<!-- omniget:agentkit:end {id} -->")
    }

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
        if out.is_empty() || out.ends_with("\n\n") {
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
