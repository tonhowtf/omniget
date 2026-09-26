//! What a skill says it needs, read from its manifest.
//!
//! Two fields carry it:
//! - `allowed-tools` (the Agent Skills field; Claude Code reads it as "tools
//!   this skill may use"): `Read Bash(pdftk:*) WebSearch mcp__files__read`;
//! - `metadata.requires` (or `metadata.omniget-requires`): OmniGet tool names
//!   or the same spellings, space- or comma-separated.
//!
//! Both are read as **dependencies**, never as grants. A skill that lists
//! `Bash` does not get a shell: the bot either already has the capability
//! (and, for scripts, the binding allows running them) or the skill is shown
//! as missing that dependency and is left out of the turn (spec A02, A20).

use serde::{Deserialize, Serialize};

use super::manifest::SkillManifest;

/// The kind of thing a dependency asks for. The bot layer maps each kind to
/// the tools that satisfy it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DepKind {
    /// Search or fetch on the web.
    Web,
    /// Read files of a project folder.
    ProjectRead,
    /// Change files of a project folder.
    ProjectWrite,
    /// Run a command (the skill's scripts included).
    Shell,
    /// The bot's personal memory.
    Memory,
    /// One tool of one MCP server.
    Mcp { server: String, tool: String },
    /// One OmniGet tool, by its exact name.
    Tool { name: String },
    /// A name we cannot check (a tool of another agent product). Shown, not
    /// enforced.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// As written in the manifest.
    pub raw: String,
    #[serde(flatten)]
    pub kind: DepKind,
}

/// Every dependency the manifest declares, deduplicated by kind, in the order
/// they first appear.
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

/// One spelling to its kind.
pub fn classify(raw: &str) -> DepKind {
    // `Bash(git:*)` → `bash`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn skill(allowed: &[&str], requires: Option<&str>) -> SkillManifest {
        let mut metadata = BTreeMap::new();
        if let Some(r) = requires {
            metadata.insert("requires".to_string(), r.to_string());
        }
        SkillManifest {
            name: "s".into(),
            description: "d".into(),
            allowed_tools: allowed.iter().map(|s| s.to_string()).collect(),
            path: PathBuf::from("/nowhere/s"),
            source: Default::default(),
            license: None,
            compatibility: None,
            metadata,
            body_bytes: 0,
            scan: Default::default(),
        }
    }

    #[test]
    fn claude_spellings_map_to_kinds() {
        let deps = dependencies(&skill(
            &[
                "Read",
                "Bash(pdftk:*)",
                "WebSearch",
                "WebFetch",
                "mcp__files__read_file",
                "Task",
            ],
            Some("download_enqueue, memory_recall"),
        ));
        let kinds: Vec<&DepKind> = deps.iter().map(|d| &d.kind).collect();
        assert!(kinds.contains(&&DepKind::ProjectRead));
        assert!(kinds.contains(&&DepKind::Shell));
        assert!(kinds.contains(&&DepKind::Web));
        assert!(kinds.contains(&&DepKind::Mcp {
            server: "files".into(),
            tool: "read_file".into()
        }));
        assert!(kinds.contains(&&DepKind::Tool {
            name: "download_enqueue".into()
        }));
        assert!(kinds.contains(&&DepKind::Memory));
        assert!(
            kinds.contains(&&DepKind::Unknown),
            "Task is not ours to check"
        );
        // WebSearch and WebFetch are one dependency.
        assert_eq!(kinds.iter().filter(|k| ***k == DepKind::Web).count(), 1);
    }

    #[test]
    fn nothing_declared_means_nothing_required() {
        assert!(dependencies(&skill(&[], None)).is_empty());
    }
}
