use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use super::agent::GrantMode;
use super::cli_runtime::accounts::SandboxMode;
use super::drivers::AccessMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    #[serde(alias = "approval-required")]
    ApprovalRequired,
    #[default]
    #[serde(
        alias = "auto-edits",
        alias = "auto-accept-edits",
        alias = "accept_edits"
    )]
    AutoEdits,
    Auto,
    #[serde(alias = "full-access")]
    FullAccess,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RunMode::ApprovalRequired => "approval_required",
            RunMode::AutoEdits => "auto_edits",
            RunMode::Auto => "auto",
            RunMode::FullAccess => "full_access",
        }
    }

    pub fn parse(s: &str) -> Result<RunMode, String> {
        match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "approval_required" | "ask" => Ok(RunMode::ApprovalRequired),
            "auto_edits" | "auto_accept_edits" | "accept_edits" => Ok(RunMode::AutoEdits),
            "auto" => Ok(RunMode::Auto),
            "full_access" => Ok(RunMode::FullAccess),
            other => Err(format!(
                "ERR_RUN_MODE: run_mode must be approval_required, auto_edits, auto or full_access (got `{other}`)"
            )),
        }
    }

    pub fn default_for(kind: &str) -> RunMode {
        match kind {
            "run" => RunMode::ApprovalRequired,
            _ => RunMode::AutoEdits,
        }
    }

    pub fn access(self) -> AccessMode {
        match self {
            RunMode::ApprovalRequired => AccessMode::ApprovalRequired,
            RunMode::AutoEdits => AccessMode::AutoAcceptEdits,
            RunMode::Auto => AccessMode::Auto,
            RunMode::FullAccess => AccessMode::FullAccess,
        }
    }

    pub fn under(self, ceiling: SandboxMode) -> Effective {
        match ceiling {
            SandboxMode::ReadOnly => Effective::ReadOnly,
            SandboxMode::Write if self == RunMode::FullAccess => Effective::Mode(RunMode::Auto),
            SandboxMode::Write | SandboxMode::Full => Effective::Mode(self),
        }
    }
}

impl std::fmt::Display for RunMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<AccessMode> for RunMode {
    fn from(mode: AccessMode) -> Self {
        match mode {
            AccessMode::ApprovalRequired => RunMode::ApprovalRequired,
            AccessMode::AutoAcceptEdits => RunMode::AutoEdits,
            AccessMode::Auto => RunMode::Auto,
            AccessMode::FullAccess => RunMode::FullAccess,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effective {
    ReadOnly,
    Mode(RunMode),
}

static BOUND: RwLock<Option<HashMap<String, RunMode>>> = RwLock::new(None);
static CHOSEN: RwLock<Option<HashMap<String, RunMode>>> = RwLock::new(None);
static STORE_FILE: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn set_store_file(path: PathBuf) {
    let loaded: HashMap<String, RunMode> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    *CHOSEN.write().unwrap_or_else(|e| e.into_inner()) = Some(loaded);
    *STORE_FILE.write().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

pub fn bind(conversation: &str, mode: RunMode) {
    BOUND
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(conversation.to_string(), mode);
}

pub fn unbind(conversation: &str) {
    if let Some(map) = BOUND.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
        map.remove(conversation);
    }
}

pub struct Binding(String);

impl Binding {
    pub fn new(conversation: &str, mode: RunMode) -> Self {
        bind(conversation, mode);
        Self(conversation.to_string())
    }
}

impl Drop for Binding {
    fn drop(&mut self) {
        unbind(&self.0);
    }
}

pub fn chosen(conversation: &str) -> Option<RunMode> {
    CHOSEN
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(conversation).copied())
}

pub fn of(conversation: &str) -> Option<RunMode> {
    BOUND
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(conversation).copied())
        .or_else(|| chosen(conversation))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaudePermission {
    pub flag: &'static str,
    pub applied: Effective,
}

pub fn claude_permission(effective: Effective, supports_auto: bool) -> ClaudePermission {
    use super::drivers::claude::protocol::permission_mode;
    use super::drivers::InteractionMode;
    let applied = match effective {
        Effective::Mode(RunMode::Auto) if !supports_auto => Effective::Mode(RunMode::AutoEdits),
        other => other,
    };
    let flag = match applied {
        Effective::ReadOnly => "plan",
        Effective::Mode(mode) => permission_mode(mode.access(), InteractionMode::Default),
    };
    ClaudePermission { flag, applied }
}

pub fn codex_flags(effective: Effective, approve_for_me: bool) -> (Vec<String>, Effective) {
    use super::drivers::codex::launch::mode_config;
    let mode = match effective {
        Effective::ReadOnly => {
            return (
                vec!["--sandbox".into(), "read-only".into()],
                Effective::ReadOnly,
            )
        }
        Effective::Mode(RunMode::Auto) if approve_for_me => {
            return (
                vec![
                    "--sandbox".into(),
                    "workspace-write".into(),
                    "--approve-for-me".into(),
                ],
                effective,
            )
        }
        Effective::Mode(RunMode::Auto) => RunMode::AutoEdits,
        Effective::Mode(m) => m,
    };
    let config = mode_config(mode.access());
    let word = |v: serde_json::Value| v.as_str().unwrap_or_default().to_string();
    let sandbox = word(serde_json::to_value(&config.sandbox).unwrap_or_default());
    let policy = word(serde_json::to_value(&config.approval_policy).unwrap_or_default());
    (
        vec![
            "--sandbox".into(),
            sandbox,
            "-c".into(),
            format!("approval_policy=\"{policy}\""),
        ],
        Effective::Mode(mode),
    )
}

const EDIT_TOOLS: &[&str] = &["fs_edit", "fs_write", "fs_apply_patch", "kb_write"];

fn is_edit(tool: &str) -> bool {
    EDIT_TOOLS.contains(&tool)
}

pub fn native_grant(mode: RunMode, tool: &str, grant: GrantMode) -> GrantMode {
    if matches!(grant, GrantMode::Deny) {
        return grant;
    }
    let edit = is_edit(tool);
    let shell = tool == "shell_exec";
    match mode {
        RunMode::ApprovalRequired if edit || shell => GrantMode::Ask,
        RunMode::AutoEdits | RunMode::Auto if edit => GrantMode::Auto,
        RunMode::FullAccess => GrantMode::Auto,
        _ => grant,
    }
}

pub fn acp_preallowed(mode: RunMode, kind: &str) -> Option<bool> {
    match mode {
        RunMode::FullAccess => Some(true),
        RunMode::AutoEdits | RunMode::Auto if kind == "edit" => Some(true),
        _ => None,
    }
}

const DENIAL_MARKERS: &[&str] = &[
    "requires approval",
    "permission for this tool use was denied",
    "haven't granted",
    "have not granted",
    "requested permissions to use",
    "permission to use",
];

pub fn looks_denied(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    DENIAL_MARKERS.iter().any(|m| lower.contains(m))
}

pub fn denial_note(applied: Effective, surface: bool) -> String {
    let why = match applied {
        Effective::ReadOnly => {
            "the CLI account is read-only (raise it in LLM > Accounts to let turns write)".to_string()
        }
        Effective::Mode(mode) if !surface => format!(
            "run mode `{mode}` needs a person to approve this, and this turn has no approval surface (no prompt tool). Run it from the app to answer, or pick a run mode that allows it"
        ),
        Effective::Mode(mode) => format!(
            "run mode `{mode}` asked a person and the answer was no"
        ),
    };
    format!("[OmniGet] Denied: {why}.")
}

pub fn auto_fallback_note(got: &str) -> String {
    format!(
        "[OmniGet] Run mode `auto` is not available for this model: the CLI runs in `{got}` mode, so edits and commands ask before they run.\n\n"
    )
}
