//! Normalização do gerador (port de `scripts/agentkit-catalog/build.mjs`):
//! descrição curta, frontmatter sintetizado, modelo lixo, tools do Copilot,
//! licenças e exclusão das skills proprietárias da Anthropic.

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

pub const DESC_MAX: usize = 300;

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("regex"))
}

fn strip_examples(s: &str) -> String {
    static A: OnceLock<Regex> = OnceLock::new();
    static B: OnceLock<Regex> = OnceLock::new();
    static C: OnceLock<Regex> = OnceLock::new();
    let s = re(&A, r"(?is)<example>.*?</example>").replace_all(s, " ");
    let s = re(&B, r"(?is)<commentary>.*?</commentary>").replace_all(&s, " ");
    re(&C, r"(?is)<example>.*$")
        .replace_all(&s, " ")
        .into_owned()
}

fn strip_trailing_lead(s: &str) -> String {
    static T: OnceLock<Regex> = OnceLock::new();
    re(
        &T,
        r"(?i)\s*(Examples?( include)?|For example|e\.g\.|Specifically|Such as|Including|Use cases?)\s*:?\s*$",
    )
    .replace(s, "")
    .trim()
    .to_string()
}

/// Descrição curta: sem `<example>`/`<commentary>`, espaço colapsado, ≤ 300 chars.
/// Devolve também as marcas de normalização aplicadas.
pub fn short_description(raw: &str) -> (String, Vec<&'static str>) {
    let mut marks = Vec::new();
    let unescaped = raw.replace("\\n", "\n");
    let mut s = strip_examples(&unescaped);
    static TAGS: OnceLock<Regex> = OnceLock::new();
    s = re(
        &TAGS,
        r"(?i)</?(example|commentary|context|user|assistant)>",
    )
    .replace_all(&s, " ")
    .into_owned();
    if s != unescaped {
        marks.push("stripped_example_blocks");
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s = strip_trailing_lead(&s);
    if s.chars().count() > DESC_MAX {
        let cut: String = s.chars().take(DESC_MAX - 1).collect();
        let base = match cut.rfind(' ') {
            Some(sp) if sp as f64 > (DESC_MAX as f64) * 0.6 => cut[..sp].to_string(),
            _ => cut,
        };
        s = format!(
            "{}…",
            base.trim_end_matches(|c: char| c.is_whitespace() || ",;:.-".contains(c))
        );
        marks.push("truncated_description");
    }
    (s, marks)
}

/// Descrição longa sem blocos `<example>` (vai no frontmatter do índice).
pub fn clean_description(raw: &str) -> String {
    let s = strip_examples(raw);
    let lines: Vec<&str> = s.lines().map(|l| l.trim_end()).collect();
    let mut out = String::new();
    let mut blank = 0;
    for l in lines {
        if l.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l);
        out.push('\n');
    }
    strip_trailing_lead(out.trim())
}

/// Primeiro parágrafo útil de um corpo Markdown.
pub fn description_from_body(body: &str) -> String {
    let mut heading: Option<String> = None;
    let mut para: Vec<String> = Vec::new();
    let mut in_code = false;
    for l in body.lines() {
        let t = l.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if t.is_empty() {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        if t.starts_with('#') {
            if heading.is_none() {
                heading = Some(t.trim_start_matches('#').trim().to_string());
            }
            if !para.is_empty() {
                break;
            }
            continue;
        }
        if ["---", "***", "<!--", "|", ">"]
            .iter()
            .any(|p| t.starts_with(p))
        {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        let t = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("* "))
            .unwrap_or(t);
        para.push(t.to_string());
        if para.join(" ").len() > 400 {
            break;
        }
    }
    let s = if para.is_empty() {
        heading.unwrap_or_default()
    } else {
        para.join(" ")
    };
    static LINK: OnceLock<Regex> = OnceLock::new();
    let s = s.replace("**", "").replace("__", "").replace('`', "");
    re(&LINK, r"\[([^\]]+)\]\([^)]+\)")
        .replace_all(&s, "$1")
        .into_owned()
}

pub fn humanize(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Modelo válido para Claude Code: alias ou id datado `claude-…-YYYYMMDD`.
pub fn valid_claude_model(m: &str) -> bool {
    static ID: OnceLock<Regex> = OnceLock::new();
    let v = m.trim();
    matches!(
        v,
        "sonnet" | "opus" | "haiku" | "inherit" | "default" | "opusplan"
    ) || re(&ID, r"^claude-[a-z0-9.-]+-\d{8}$").is_match(v)
}

const CLAUDE_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "MultiEdit",
    "Bash",
    "Grep",
    "Glob",
    "LS",
    "WebSearch",
    "WebFetch",
    "TodoWrite",
    "Task",
    "Agent",
    "NotebookEdit",
    "NotebookRead",
    "BashOutput",
    "KillShell",
    "Skill",
    "AskUserQuestion",
];
const COPILOT_TOOLS: &[&str] = &[
    "codebase",
    "search",
    "editFiles",
    "runCommands",
    "runTests",
    "problems",
    "githubRepo",
    "fetch",
    "findTestFiles",
    "usages",
    "testFailure",
    "vscodeAPI",
    "openSimpleBrowser",
    "changes",
    "extensions",
    "searchResults",
    "terminalLastCommand",
    "terminalSelection",
    "runTasks",
    "runNotebooks",
    "new",
    "terminalCommand",
    "filesystem",
    "database",
    "github",
    "websearch",
    "think",
    "todos",
    "read",
    "edit",
    "shell",
    "execute",
    "web",
    "agent",
    "todo",
    "vscode",
    "runSubagent",
];

/// Lista de ferramentas de um frontmatter (`"A, B"` ou `[A, B]`).
pub fn tool_list(tools: &Value) -> Vec<String> {
    let raw: Vec<String> = match tools {
        Value::Array(a) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v.to_string())
            })
            .collect(),
        Value::String(s) => s.split(',').map(str::to_string).collect(),
        _ => Vec::new(),
    };
    raw.into_iter()
        .map(|t| t.trim().trim_matches(|c| c == '\'' || c == '"').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn is_vscode_tool(t: &str) -> bool {
    static SLASH: OnceLock<Regex> = OnceLock::new();
    COPILOT_TOOLS.contains(&t)
        || re(&SLASH, r"^[a-z][\w.-]*/[\w*]").is_match(t)
        || t.starts_with("azure_")
        || t.starts_with("mssql_")
        || t.starts_with("pgsql_")
        || t == "microsoft.docs.mcp"
}

/// Origem do `tools` de um agente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolsOrigin {
    Claude,
    /// Chatmode do Copilot com ids de ferramenta do VS Code.
    Copilot,
    /// Ferramentas do Claude misturadas com ids estranhos.
    Mixed,
}

pub fn tools_origin(tools: &Value, model: Option<&str>) -> ToolsOrigin {
    static CALL: OnceLock<Regex> = OnceLock::new();
    static CAP: OnceLock<Regex> = OnceLock::new();
    let list = tool_list(tools);
    let has_claude = list.iter().any(|t| {
        CLAUDE_TOOLS.contains(&t.as_str())
            || re(&CALL, r"^(Bash|Read|Write|Edit|WebFetch)\(").is_match(t)
            || t.starts_with("mcp__")
            || t == "*"
    });
    let foreign = list.iter().any(|t| {
        !CLAUDE_TOOLS.contains(&t.as_str())
            && !t.starts_with("mcp__")
            && t != "*"
            && !re(&CAP, r"^[A-Z][A-Za-z]+(\(.*\))?$").is_match(t)
    });
    if model.is_some_and(|m| m.to_ascii_lowercase().contains("(copilot)")) {
        return ToolsOrigin::Copilot;
    }
    if !has_claude && list.iter().any(|t| is_vscode_tool(t)) {
        return ToolsOrigin::Copilot;
    }
    if has_claude && foreign {
        return ToolsOrigin::Mixed;
    }
    ToolsOrigin::Claude
}

/// Licença em texto livre → SPDX (ou `Proprietary`); `None` quando só aponta para arquivo.
pub fn normalize_license(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Object(o) => o
            .get("type")
            .or_else(|| o.get("name"))
            .or_else(|| o.get("spdx"))
            .and_then(Value::as_str)?
            .to_string(),
        _ => return None,
    };
    normalize_license_str(&s)
}

pub fn normalize_license_str(s: &str) -> Option<String> {
    let v = s.trim();
    if v.is_empty() {
        return None;
    }
    let low = v.to_ascii_lowercase();
    if low.contains("proprietary") {
        return Some("Proprietary".into());
    }
    if (low.contains("complete terms in")
        || low.contains("see license")
        || low.contains("license.txt")
        || low.contains("license.md"))
        && !v.contains("MIT")
        && !v.contains("Apache")
    {
        return None;
    }
    static MAP: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        [
            (r"(?i)^mit( license)?$", "MIT"),
            (
                r"(?i)^apache[- ]?(license)?[ ,-]*(v(ersion)?)? ?2(\.0)?$",
                "Apache-2.0",
            ),
            (r"(?i)^cc[- ]?by[- ]?4(\.0)?$", "CC-BY-4.0"),
            (r"(?i)^cc[- ]?by[- ]?sa[- ]?4(\.0)?$", "CC-BY-SA-4.0"),
            (r"(?i)^cc0([- ]?1(\.0)?)?$", "CC0-1.0"),
            (r"(?i)^bsd[- ]?3([- ]?clause)?$", "BSD-3-Clause"),
            (r"(?i)^bsd[- ]?2([- ]?clause)?$", "BSD-2-Clause"),
            (r"(?i)^isc$", "ISC"),
            (r"(?i)^gpl[- ]?3(\.0)?(-only|-or-later)?$", "GPL-3.0"),
            (r"(?i)^agpl[- ]?3(\.0)?(-only|-or-later)?$", "AGPL-3.0"),
            (r"(?i)^mpl[- ]?2(\.0)?$", "MPL-2.0"),
            (r"(?i)^unlicense$", "Unlicense"),
        ]
        .into_iter()
        .map(|(p, s)| (Regex::new(p).expect("license regex"), s))
        .collect()
    });
    for (r, spdx) in map {
        if r.is_match(v) {
            return Some((*spdx).into());
        }
    }
    Some(v.chars().take(80).collect())
}

/// Detecta a licença pelo texto de um arquivo LICENSE.
pub fn detect_license_text(text: &str) -> Option<&'static str> {
    let t: String = text.chars().take(4000).collect();
    let low = t.to_ascii_lowercase();
    let has = |s: &str| low.contains(&s.to_ascii_lowercase());
    static APACHE: OnceLock<Regex> = OnceLock::new();
    static GPL3: OnceLock<Regex> = OnceLock::new();
    static GPL2: OnceLock<Regex> = OnceLock::new();
    static MPL: OnceLock<Regex> = OnceLock::new();
    static MITH: OnceLock<Regex> = OnceLock::new();
    static ISC: OnceLock<Regex> = OnceLock::new();
    if (has("proprietary") || has("all rights reserved")) && has("anthropic") {
        return Some("Proprietary");
    }
    if has("Permission is hereby granted, free of charge")
        || re(&MITH, r"(?im)^\s*MIT License").is_match(&t)
    {
        return Some("MIT");
    }
    if re(&APACHE, r"(?is)Apache License.{0,60}Version 2\.0").is_match(&t) {
        return Some("Apache-2.0");
    }
    if has("GNU AFFERO GENERAL PUBLIC LICENSE") {
        return Some("AGPL-3.0");
    }
    if has("GNU LESSER GENERAL PUBLIC LICENSE") {
        return Some("LGPL-3.0");
    }
    if re(&GPL3, r"(?is)GNU GENERAL PUBLIC LICENSE.{0,120}Version 3").is_match(&t) {
        return Some("GPL-3.0");
    }
    if re(&GPL2, r"(?is)GNU GENERAL PUBLIC LICENSE.{0,120}Version 2").is_match(&t) {
        return Some("GPL-2.0");
    }
    if re(&MPL, r"(?is)Mozilla Public License.{0,40}2\.0").is_match(&t) {
        return Some("MPL-2.0");
    }
    if has("Attribution-ShareAlike 4.0") {
        return Some("CC-BY-SA-4.0");
    }
    if has("Attribution 4.0 International") || has("CC-BY-4.0") || has("CC BY 4.0") {
        return Some("CC-BY-4.0");
    }
    if has("CC0 1.0") || has("Creative Commons Zero") {
        return Some("CC0-1.0");
    }
    if has("This is free and unencumbered software") {
        return Some("Unlicense");
    }
    if has("SIL OPEN FONT LICENSE") {
        return Some("OFL-1.1");
    }
    if re(&ISC, r"(?im)^\s*ISC License").is_match(&t) {
        return Some("ISC");
    }
    if has("Redistribution and use in source and binary forms") {
        return Some(if has("Neither the name") {
            "BSD-3-Clause"
        } else {
            "BSD-2-Clause"
        });
    }
    if has("proprietary") || has("all rights reserved") {
        return Some("Proprietary");
    }
    None
}

/// Motivo para excluir uma skill (proprietárias da Anthropic), ou `None`.
pub fn skill_exclusion(name: &str, license: Option<&str>) -> Option<&'static str> {
    if ["docx", "pdf", "pptx", "xlsx", "pdf-anthropic"].contains(&name) {
        return Some("anthropic_proprietary_document_skill");
    }
    if name.ends_with("-official") {
        return Some("anthropic_official_duplicate");
    }
    if license == Some("Proprietary") {
        return Some("proprietary_license");
    }
    None
}

/// Tags/keywords: lista YAML ou string separada por vírgula/espaço.
pub fn to_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| match x {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Null => None,
                o => Some(o.to_string()),
            })
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split(|c: char| c == ',' || c.is_whitespace())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// `author` (string ou `{name}`), ou `metadata.author`.
pub fn author_of(fm: &Value) -> Option<String> {
    let a = fm
        .get("author")
        .filter(|v| !v.is_null())
        .or_else(|| fm.get("metadata").and_then(|m| m.get("author")))?;
    match a {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Object(o) => o.get("name").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

/// Encurta strings enormes de um JSON guardado como frontmatter.
pub fn slim_json(v: &Value) -> Value {
    match v {
        Value::String(s) if s.chars().count() > 2000 => {
            Value::String(format!("{}…", s.chars().take(2000).collect::<String>()))
        }
        Value::Array(a) => Value::Array(a.iter().map(slim_json).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, x)| (k.clone(), slim_json(x))).collect())
        }
        other => other.clone(),
    }
}
