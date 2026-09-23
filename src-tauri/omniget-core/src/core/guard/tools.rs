//! Tool names each coding tool understands in a `tools:` / `allowed-tools:`
//! field, and detection of which tool a component's format belongs to.
//!
//! The point: a Copilot chatmode that lists `codebase, editFiles` is fine as a
//! Copilot item and only "another tool's format" inside a Claude item. It is
//! never an error.

/// Claude Code tool names (plus permission-rule syntax handled separately).
const CLAUDE: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "MultiEdit",
    "Bash",
    "BashOutput",
    "KillShell",
    "KillBash",
    "Glob",
    "Grep",
    "LS",
    "WebSearch",
    "WebFetch",
    "TodoWrite",
    "TodoRead",
    "Task",
    "Agent",
    "NotebookEdit",
    "NotebookRead",
    "SlashCommand",
    "Skill",
    "AskUserQuestion",
    "ExitPlanMode",
    "EnterPlanMode",
    "ListMcpResourcesTool",
    "ReadMcpResourceTool",
    "Monitor",
    "LSP",
    "ToolSearch",
    "*",
];

/// VS Code / GitHub Copilot chat-mode and custom-agent tool ids.
const COPILOT: &[&str] = &[
    "codebase",
    "search",
    "edit",
    "editFiles",
    "runCommands",
    "runTests",
    "runTasks",
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
    "terminalCommand",
    "new",
    "think",
    "todos",
    "runNotebooks",
    "runSubagent",
    "read",
    "execute",
    "shell",
    "web",
    "agent",
    "todo",
    "vscode",
    "github",
    "custom-agent",
    "filesystem",
    "database",
    "git",
    "selection",
];

/// Copilot ids that no other tool uses; one of them is enough to call a list Copilot's.
const COPILOT_ONLY: &[&str] = &[
    "codebase",
    "editFiles",
    "runCommands",
    "runTests",
    "runTasks",
    "problems",
    "githubRepo",
    "findTestFiles",
    "usages",
    "testFailure",
    "vscodeAPI",
    "openSimpleBrowser",
    "changes",
    "searchResults",
    "terminalLastCommand",
    "terminalSelection",
    "terminalCommand",
    "runNotebooks",
    "runSubagent",
    "extensions",
];

const OPENCODE: &[&str] = &[
    "read",
    "write",
    "edit",
    "bash",
    "grep",
    "glob",
    "list",
    "patch",
    "webfetch",
    "websearch",
    "todowrite",
    "todoread",
    "task",
    "skill",
    "lsp",
    "*",
];

const GEMINI: &[&str] = &[
    "read_file",
    "write_file",
    "replace",
    "run_shell_command",
    "glob",
    "search_file_content",
    "web_fetch",
    "google_web_search",
    "list_directory",
    "read_many_files",
    "save_memory",
    "write_todos",
    "*",
];

const QWEN: &[&str] = GEMINI;

const DROID: &[&str] = &[
    "Read",
    "LS",
    "Grep",
    "Glob",
    "Create",
    "Edit",
    "MultiEdit",
    "ApplyPatch",
    "Execute",
    "WebSearch",
    "FetchUrl",
    "TodoWrite",
    "Task",
    "read-only",
    "edit",
    "execute",
    "web",
    "mcp",
    "*",
];

/// Names a tool understands, or `None` when we do not know its list (then no
/// tool-name check is made at all).
pub fn known_tools(tool: &str) -> Option<&'static [&'static str]> {
    match tool {
        "claude" | "omniget" => Some(CLAUDE),
        "copilot" => Some(COPILOT),
        "opencode" | "kilo" => Some(OPENCODE),
        "gemini" => Some(GEMINI),
        "qwen" => Some(QWEN),
        "droid" => Some(DROID),
        _ => None,
    }
}

/// Split a tool entry into its base name (`Bash(git:*)` → `Bash`,
/// `edit/editFiles` → `edit/editFiles`, `mcp__x__y` → `mcp__x__y`).
fn base(entry: &str) -> &str {
    let e = entry.trim();
    e.split_once('(').map(|(b, _)| b.trim()).unwrap_or(e)
}

/// Is `entry` a valid tool reference for `tool`?
pub fn is_known(tool: &str, entry: &str) -> bool {
    let b = base(entry);
    if b.is_empty() {
        return true;
    }
    // MCP tool references are valid everywhere they are written in that tool's style.
    if b.starts_with("mcp__") {
        return matches!(tool, "claude" | "omniget" | "droid");
    }
    let Some(list) = known_tools(tool) else {
        return true;
    };
    if tool == "copilot" {
        // Namespaced ids (`edit/editFiles`, `github/*`, `azure-mcp/search`)
        // and MCP server tool ids are Copilot's normal shape.
        if b.contains('/') || b.contains('.') || b.contains('_') || b.contains('-') {
            return true;
        }
    }
    list.iter()
        .any(|t| t.eq_ignore_ascii_case(b) && (tool != "claude" || *t == b))
}

/// Guess the tool whose format a tool list is written in.
pub fn tools_fingerprint(entries: &[String]) -> Option<&'static str> {
    if entries.is_empty() {
        return None;
    }
    let copilot = entries.iter().filter(|e| {
        let b = base(e);
        COPILOT_ONLY.contains(&b)
            || b.split('/').next().is_some_and(|ns| {
                matches!(
                    ns,
                    "edit"
                        | "search"
                        | "read"
                        | "execute"
                        | "web"
                        | "vscode"
                        | "agent"
                        | "github"
                        | "todo"
                ) && b.contains('/')
            })
            || (b.ends_with("/*") && !b.starts_with("mcp__"))
    });
    if copilot.count() > 0 {
        return Some("copilot");
    }
    let claude_hits = entries
        .iter()
        .filter(|e| CLAUDE.contains(&base(e)) || base(e).starts_with("mcp__"))
        .count();
    if claude_hits * 2 >= entries.len() {
        return Some("claude");
    }
    let gemini_hits = entries.iter().filter(|e| GEMINI.contains(&base(e))).count();
    if gemini_hits * 2 >= entries.len() && gemini_hits > 0 {
        return Some("gemini");
    }
    // Copilot custom-agent aliases: lower-case `read, edit, search, shell`.
    let lower_aliases = entries
        .iter()
        .filter(|e| {
            matches!(
                base(e),
                "read" | "edit" | "search" | "shell" | "execute" | "web" | "agent" | "todo"
            )
        })
        .count();
    if lower_aliases * 2 >= entries.len()
        && entries
            .iter()
            .any(|e| matches!(base(e), "shell" | "search" | "execute" | "web" | "agent"))
    {
        return Some("copilot");
    }
    let oc_hits = entries
        .iter()
        .filter(|e| OPENCODE.contains(&base(e)))
        .count();
    if oc_hits * 2 >= entries.len() && oc_hits > 0 {
        return Some("opencode");
    }
    None
}

/// Guess the native tool of a component from its path and frontmatter keys.
pub fn detect_from_path(path: &str) -> Option<&'static str> {
    let p = path.replace('\\', "/").to_lowercase();
    if p.ends_with(".chatmode.md")
        || p.ends_with(".agent.md")
        || p.ends_with(".prompt.md")
        || p.ends_with(".instructions.md")
        || p.contains(".github/agents/")
        || p.contains(".github/chatmodes/")
        || p.contains(".github/prompts/")
    {
        return Some("copilot");
    }
    if p.ends_with(".mdc") || p.contains(".cursor/") {
        return Some("cursor");
    }
    if p.contains(".gemini/") || p.ends_with("gemini.md") {
        return Some("gemini");
    }
    if p.contains(".qwen/") {
        return Some("qwen");
    }
    if p.contains(".opencode/") || p.ends_with("opencode.json") || p.ends_with("opencode.jsonc") {
        return Some("opencode");
    }
    if p.contains(".codex/") || p.ends_with("config.toml") && p.contains("codex") {
        return Some("codex");
    }
    if p.contains(".windsurf/") {
        return Some("windsurf");
    }
    if p.contains(".kiro/") || p.ends_with(".kiro.hook") {
        return Some("kiro");
    }
    if p.contains(".clinerules") {
        return Some("cline");
    }
    if p.contains(".roo/") || p.ends_with(".roomodes") {
        return Some("roo");
    }
    if p.contains(".factory/") {
        return Some("droid");
    }
    if p.contains(".claude/") || p.ends_with(".mcp.json") {
        return Some("claude");
    }
    None
}

/// Known hook events per tool; `None` means we do not check.
pub fn known_events(tool: &str) -> Option<&'static [&'static str]> {
    const CLAUDE_EV: &[&str] = &[
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "Notification",
        "UserPromptSubmit",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "PreCompact",
        "PostCompact",
        "SessionStart",
        "SessionEnd",
        "PermissionRequest",
        "WorktreeCreate",
        "WorktreeRemove",
        "Setup",
        "TeammateIdle",
        "TaskCompleted",
        "ConfigChange",
        "Elicitation",
        "ElicitationResult",
        "InstructionsLoaded",
        "StopFailure",
        "CwdChanged",
        "FileChanged",
        "TaskCreated",
        "PermissionDenied",
    ];
    const GEMINI_EV: &[&str] = &[
        "BeforeTool",
        "AfterTool",
        "BeforeAgent",
        "AfterAgent",
        "SessionStart",
        "SessionEnd",
        "Notification",
        "PreCompress",
        "BeforeModel",
        "AfterModel",
        "BeforeToolSelection",
    ];
    const CURSOR_EV: &[&str] = &[
        "beforeShellExecution",
        "afterShellExecution",
        "beforeMCPExecution",
        "afterMCPExecution",
        "beforeReadFile",
        "afterFileEdit",
        "beforeSubmitPrompt",
        "stop",
        "afterAgentResponse",
        "afterAgentThought",
        "sessionStart",
        "sessionEnd",
        "preCompact",
        "subagentStop",
        "beforeTabFileRead",
        "afterTabFileEdit",
        "preToolUse",
        "postToolUse",
        "postToolUseFailure",
        "subagentStart",
    ];
    match tool {
        "claude" | "omniget" | "qwen" | "droid" | "codebuff" => Some(CLAUDE_EV),
        "gemini" => Some(GEMINI_EV),
        "cursor" => Some(CURSOR_EV),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_fingerprint() {
        let t: Vec<String> = ["codebase", "edit/editFiles", "fetch"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(tools_fingerprint(&t), Some("copilot"));
        let c: Vec<String> = ["Read", "Write", "Bash(git:*)"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(tools_fingerprint(&c), Some("claude"));
        assert!(is_known("claude", "Bash(git add:*)"));
        assert!(!is_known("claude", "codebase"));
        assert!(is_known("copilot", "edit/editFiles"));
    }
}
