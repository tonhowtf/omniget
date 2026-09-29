//! One permission vocabulary for every turn (2.7, S13, `82#P5`): a job, a
//! Loop round and a chat conversation each carry a `run_mode`, and that one
//! word is what the Claude and Codex argv, the native harness grants and the
//! ACP permission options all follow.
//!
//! | run_mode | Claude (`claude -p`) | Codex (`codex exec`) | native / ACP |
//! |---|---|---|---|
//! | `approval_required` | `--permission-mode default` + prompt tool | `read-only`, `untrusted` | edits and shell ask |
//! | `auto_edits` | `acceptEdits` + prompt tool for the shell | `workspace-write`, `on-request` | edits go through, shell as granted |
//! | `auto` | `auto` (the CLI's reviewer), if the CLI has it | `workspace-write --approve-for-me` | like `auto_edits` (no reviewer) |
//! | `full_access` | `bypassPermissions` | `danger-full-access`, `never` | granted tools never ask |
//!
//! The tables are the Central's (`drivers::claude::protocol::permission_mode`,
//! `drivers::codex::launch::mode_config`): this module maps onto them instead
//! of keeping a second copy.
//!
//! The account's [`SandboxMode`] is a **ceiling**, never a mode: a read-only
//! account turns every run mode into read-only, a write account stops at
//! `auto`, and only an account the user raised to `full` lets `full_access`
//! through. Nothing here produces `full_access` by itself: it is either in
//! the job, Loop or conversation because a person picked it, or it is not.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use super::agent::GrantMode;
use super::cli_runtime::accounts::SandboxMode;
use super::drivers::AccessMode;

/// How much a turn may do without asking. The wire form is snake_case
/// (`approval_required | auto_edits | auto | full_access`); the Central's
/// kebab-case spellings are read too.
///
/// `Default` is `AutoEdits` because that is what every turn did before 2.7
/// (a write account ran `acceptEdits` with the prompt tool for the shell): a
/// row or a Loop saved before this field existed keeps its behaviour. It is
/// **not** the default of a new external job, which is
/// [`RunMode::default_for`].
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

/// Every run mode, safest first (the order of the pickers).
pub const ALL: [RunMode; 4] = [
    RunMode::ApprovalRequired,
    RunMode::AutoEdits,
    RunMode::Auto,
    RunMode::FullAccess,
];

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RunMode::ApprovalRequired => "approval_required",
            RunMode::AutoEdits => "auto_edits",
            RunMode::Auto => "auto",
            RunMode::FullAccess => "full_access",
        }
    }

    /// A typo is an error the caller shows (HTTP 400, CLI message), never a
    /// silent fallback: guessing would either grant more or ask more than the
    /// person meant.
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

    /// The mode of a new job of `kind` when nobody chose one (D2): a job
    /// started from outside the window (`run`: bridge, CLI, MCP) asks before
    /// every edit and command; a chat turn and everything that already had a
    /// behaviour (missions, triggers) keep `auto_edits`. A Loop round takes
    /// its Loop's mode, never this.
    pub fn default_for(kind: &str) -> RunMode {
        match kind {
            "run" => RunMode::ApprovalRequired,
            _ => RunMode::AutoEdits,
        }
    }

    /// The Central's name of the same mode (its driver tables).
    pub fn access(self) -> AccessMode {
        match self {
            RunMode::ApprovalRequired => AccessMode::ApprovalRequired,
            RunMode::AutoEdits => AccessMode::AutoAcceptEdits,
            RunMode::Auto => AccessMode::Auto,
            RunMode::FullAccess => AccessMode::FullAccess,
        }
    }

    /// What the account allows of this mode (see the module doc).
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

/// A run mode after the account's ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effective {
    /// The account is read-only: nothing is written, whatever the mode.
    ReadOnly,
    Mode(RunMode),
}

impl Effective {
    pub fn label(self) -> &'static str {
        match self {
            Effective::ReadOnly => "read_only",
            Effective::Mode(m) => m.as_str(),
        }
    }
}

// ── the mode of a conversation ──────────────────────────────────────────

/// Jobs and Loop rounds bind their mode for the length of the turn; the chat
/// keeps one per conversation in `<llm dir>/run-modes.json`. A binding wins
/// over the stored choice (a job on a chat conversation runs in the job's
/// mode).
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

/// The mode a job or Loop round runs `conversation` in, until [`unbind`].
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

/// Held by a job's runner: the binding goes when the job ends, however it
/// ends.
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

/// The chat's choice for `conversation` (`None` forgets it: back to the
/// chat default). Persisted when a store file is set.
pub fn choose(conversation: &str, mode: Option<RunMode>) {
    let mut guard = CHOSEN.write().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    match mode {
        Some(m) => {
            map.insert(conversation.to_string(), m);
        }
        None => {
            map.remove(conversation);
        }
    }
    if let Some(file) = STORE_FILE.read().unwrap_or_else(|e| e.into_inner()).clone() {
        if let Ok(bytes) = serde_json::to_vec_pretty(&*map) {
            let _ = std::fs::write(file, bytes);
        }
    }
}

/// The chat's stored choice, without the job bindings.
pub fn chosen(conversation: &str) -> Option<RunMode> {
    CHOSEN
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(conversation).copied())
}

/// The mode `conversation` runs in, if anything set one: a job's binding,
/// else the chat's choice. `None` = nobody chose (a CLI turn then uses the
/// chat default, `auto_edits`; the native harness keeps its grants as they
/// are).
pub fn of(conversation: &str) -> Option<RunMode> {
    BOUND
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(conversation).copied())
        .or_else(|| chosen(conversation))
}

// ── Claude Code ─────────────────────────────────────────────────────────

/// `--permission-mode` of a `claude -p` turn and the mode that it really is
/// (`auto` needs the CLI to list it; without it the turn runs `auto_edits`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaudePermission {
    pub flag: &'static str,
    pub applied: Effective,
}

/// `supports_auto`: the CLI's `--help` lists `auto` among the
/// `--permission-mode` choices (`caps::parse_help_choices`). Even then the model may
/// not take it (SP2: haiku silently runs `default`): the parser checks the
/// `system/init` event and says so.
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

// ── Codex ───────────────────────────────────────────────────────────────

/// The `codex exec` flags of a mode, and the mode they really are. Read-only
/// is the argv every Codex turn had before 2.7 (`--sandbox read-only`), so a
/// read-only account launches exactly as it did.
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

// ── native harness and ACP ──────────────────────────────────────────────

/// Tools of the native harness that change files.
const EDIT_TOOLS: &[&str] = &["fs_edit", "fs_write", "fs_apply_patch", "kb_write"];

fn is_edit(tool: &str) -> bool {
    EDIT_TOOLS.contains(&tool)
}

/// The native harness under a run mode. The agent's grant stays the ceiling
/// per tool (a `Deny` is never lifted); the mode moves `Auto`/`Ask` inside
/// it: `approval_required` makes edits and the shell ask, `auto_edits` and
/// `auto` let a granted edit through, `full_access` lets every granted tool
/// through. Stored rules and the tools that always ask come after this.
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

/// An ACP permission request under a run mode: `Some(true)` = answer "allow
/// once" without asking. `kind` is the ACP `ToolKind` (`edit`, `execute`,
/// `delete`, `read`, …). `approval_required` always asks; nothing here ever
/// answers "always".
pub fn acp_preallowed(mode: RunMode, kind: &str) -> Option<bool> {
    match mode {
        RunMode::FullAccess => Some(true),
        RunMode::AutoEdits | RunMode::Auto if kind == "edit" => Some(true),
        _ => None,
    }
}

// ── when something is denied ────────────────────────────────────────────

/// Words of a CLI's own denial of a tool call (Claude 2.1.28x with
/// `--permission-prompts none`, and the older wording).
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

/// The reason OmniGet gives next to a denied tool call (S5: a denial never
/// comes without one). `surface`: the turn had a way to ask a person.
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

/// Said once at the start of a Claude turn that asked for `auto` and got
/// something else (SP2: `auto` silently becomes `default` on some models).
pub fn auto_fallback_note(got: &str) -> String {
    format!(
        "[OmniGet] Run mode `auto` is not available for this model: the CLI runs in `{got}` mode, so edits and commands ask before they run.\n\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_form_is_snake_case_and_typos_are_errors() {
        for mode in ALL {
            assert_eq!(RunMode::parse(mode.as_str()).unwrap(), mode);
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!("\"{}\"", mode.as_str()));
            assert_eq!(serde_json::from_str::<RunMode>(&json).unwrap(), mode);
            // The Central's spelling reads as the same mode.
            let kebab = format!("\"{}\"", mode.access().as_str());
            assert_eq!(serde_json::from_str::<RunMode>(&kebab).unwrap(), mode);
            assert_eq!(RunMode::from(mode.access()), mode);
        }
        assert!(RunMode::parse("full").is_err());
        assert!(RunMode::parse("yolo")
            .unwrap_err()
            .starts_with("ERR_RUN_MODE"));
    }

    #[test]
    fn external_jobs_ask_and_nothing_defaults_to_full_access() {
        assert_eq!(RunMode::default_for("run"), RunMode::ApprovalRequired);
        for kind in ["chat", "mission", "trigger", "loop", "tool:x", ""] {
            assert_ne!(RunMode::default_for(kind), RunMode::FullAccess, "{kind}");
        }
        assert_eq!(RunMode::default(), RunMode::AutoEdits);
    }

    #[test]
    fn the_account_is_a_ceiling() {
        for mode in ALL {
            assert_eq!(mode.under(SandboxMode::ReadOnly), Effective::ReadOnly);
            assert_eq!(mode.under(SandboxMode::Full), Effective::Mode(mode));
        }
        assert_eq!(
            RunMode::FullAccess.under(SandboxMode::Write),
            Effective::Mode(RunMode::Auto)
        );
        assert_eq!(
            RunMode::AutoEdits.under(SandboxMode::Write),
            Effective::Mode(RunMode::AutoEdits)
        );
    }

    #[test]
    fn claude_flags_follow_the_table_and_auto_needs_the_cli() {
        let flag = |m, auto| claude_permission(Effective::Mode(m), auto).flag;
        assert_eq!(flag(RunMode::ApprovalRequired, true), "default");
        assert_eq!(flag(RunMode::AutoEdits, true), "acceptEdits");
        assert_eq!(flag(RunMode::Auto, true), "auto");
        assert_eq!(flag(RunMode::FullAccess, true), "bypassPermissions");
        let old = claude_permission(Effective::Mode(RunMode::Auto), false);
        assert_eq!(
            (old.flag, old.applied),
            ("acceptEdits", Effective::Mode(RunMode::AutoEdits))
        );
        assert_eq!(claude_permission(Effective::ReadOnly, true).flag, "plan");
    }

    #[test]
    fn codex_flags_follow_the_central_table() {
        let flags = |m, a| codex_flags(Effective::Mode(m), a).0.join(" ");
        assert_eq!(
            flags(RunMode::ApprovalRequired, true),
            "--sandbox read-only -c approval_policy=\"untrusted\""
        );
        assert_eq!(
            flags(RunMode::AutoEdits, true),
            "--sandbox workspace-write -c approval_policy=\"on-request\""
        );
        assert_eq!(
            flags(RunMode::Auto, true),
            "--sandbox workspace-write --approve-for-me"
        );
        assert_eq!(flags(RunMode::Auto, false), flags(RunMode::AutoEdits, true));
        assert_eq!(
            flags(RunMode::FullAccess, true),
            "--sandbox danger-full-access -c approval_policy=\"never\""
        );
        assert_eq!(
            codex_flags(Effective::ReadOnly, true).0.join(" "),
            "--sandbox read-only"
        );
    }

    #[test]
    fn native_grants_move_inside_the_agent_s_grant() {
        use GrantMode::*;
        let g = |m, t, gr| native_grant(m, t, gr);
        assert!(matches!(g(RunMode::ApprovalRequired, "fs_edit", Auto), Ask));
        assert!(matches!(
            g(RunMode::ApprovalRequired, "shell_exec", Auto),
            Ask
        ));
        assert!(matches!(
            g(RunMode::ApprovalRequired, "fs_read", Auto),
            Auto
        ));
        assert!(matches!(g(RunMode::AutoEdits, "fs_edit", Ask), Auto));
        assert!(matches!(g(RunMode::AutoEdits, "shell_exec", Ask), Ask));
        assert!(matches!(g(RunMode::FullAccess, "shell_exec", Ask), Auto));
        for m in ALL {
            assert!(matches!(g(m, "shell_exec", Deny), Deny), "{m}");
        }
        assert_eq!(acp_preallowed(RunMode::AutoEdits, "edit"), Some(true));
        assert_eq!(acp_preallowed(RunMode::AutoEdits, "execute"), None);
        assert_eq!(acp_preallowed(RunMode::ApprovalRequired, "edit"), None);
        assert_eq!(acp_preallowed(RunMode::FullAccess, "delete"), Some(true));
    }

    #[test]
    fn a_job_binding_wins_over_the_chat_choice_and_goes_with_the_job() {
        choose("rm-conv", Some(RunMode::AutoEdits));
        {
            let _b = Binding::new("rm-conv", RunMode::ApprovalRequired);
            assert_eq!(of("rm-conv"), Some(RunMode::ApprovalRequired));
        }
        assert_eq!(of("rm-conv"), Some(RunMode::AutoEdits));
        choose("rm-conv", None);
        assert_eq!(of("rm-conv"), None);
    }

    #[test]
    fn every_denial_note_says_why() {
        for applied in [
            Effective::ReadOnly,
            Effective::Mode(RunMode::ApprovalRequired),
            Effective::Mode(RunMode::AutoEdits),
        ] {
            for surface in [true, false] {
                let note = denial_note(applied, surface);
                assert!(note.starts_with("[OmniGet] Denied: "), "{note}");
                assert!(note.len() > 40, "{note}");
            }
        }
        assert!(looks_denied("Permission for this tool use was denied. It requires approval, and this session has no approval surface"));
        assert!(looks_denied(
            "Claude requested permissions to use Bash, but you haven't granted it yet."
        ));
        assert!(!looks_denied("file not found"));
    }
}
