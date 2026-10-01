use serde::{Deserialize, Serialize};

use super::manifest::SkillManifest;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DepKind {
    Web,
    ProjectRead,
    ProjectWrite,
    Shell,
    Memory,
    Mcp { server: String, tool: String },
    Tool { name: String },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub raw: String,
    #[serde(flatten)]
    pub kind: DepKind,
}

pub fn dependencies(skill: &SkillManifest) -> Vec<Dependency> {
    let mut raw: Vec<String> = skill.allowed_tools.clone();
    for key in ["requires", "omniget-requires", "omniget_requires"] {
        if let Some(v) = skill.metadata.get(key) {
            raw.extend(
                v.split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string()),
            );
        }
    }
    let mut out: Vec<Dependency> = Vec::new();
    for r in raw {
        let kind = classify(&r);
        if out
            .iter()
            .any(|d| d.kind == kind && kind != DepKind::Unknown)
        {
            continue;
        }
        if kind == DepKind::Unknown && out.iter().any(|d| d.raw == r) {
            continue;
        }
        out.push(Dependency { raw: r, kind });
    }
    out
}

pub fn classify(raw: &str) -> DepKind {
    let base = raw.split('(').next().unwrap_or(raw).trim();
    let lower = base.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("mcp__") {
        if let Some((server, tool)) = base["mcp__".len()..].split_once("__") {
            if !server.is_empty() && !tool.is_empty() && !rest.is_empty() {
                return DepKind::Mcp {
                    server: server.to_string(),
                    tool: tool.to_string(),
                };
            }
        }
        return DepKind::Unknown;
    }
    if let Some(rest) = base.strip_prefix("mcp:") {
        if let Some((server, tool)) = rest.split_once(':') {
            if !server.is_empty() && !tool.is_empty() {
                return DepKind::Mcp {
                    server: server.to_string(),
                    tool: tool.to_string(),
                };
            }
        }
        return DepKind::Unknown;
    }
    match lower.as_str() {
        "websearch" | "webfetch" | "web" | "web_search" | "web_fetch" | "browser" => DepKind::Web,
        "read" | "glob" | "grep" | "ls" | "notebookread" | "fs_read" | "fs_list" | "fs_glob"
        | "fs_grep" => DepKind::ProjectRead,
        "write" | "edit" | "multiedit" | "notebookedit" | "fs_write" | "fs_edit"
        | "fs_apply_patch" => DepKind::ProjectWrite,
        "bash" | "shell" | "shell_exec" | "powershell" => DepKind::Shell,
        "memory" => DepKind::Memory,
        "todowrite" => DepKind::Tool {
            name: "todo_write".into(),
        },
        _ if lower.starts_with("memory_") => DepKind::Memory,
        _ if !lower.is_empty()
            && base
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            && base.contains('_') =>
        {
            DepKind::Tool {
                name: base.to_string(),
            }
        }
        _ => DepKind::Unknown,
    }
}
