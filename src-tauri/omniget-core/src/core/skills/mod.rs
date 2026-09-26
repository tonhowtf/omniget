//! Agent Skills executor (SKILL.md folders). Plan §2.1 line 12.
//!
//! A skill is a directory with a `SKILL.md` at its root: YAML frontmatter
//! (`name`, `description`, optional `license`, `compatibility`, `metadata`,
//! `allowed-tools`) followed by Markdown instructions, plus any auxiliary files
//! (`scripts/`, `references/`, `assets/`). This is the same format the desktop
//! agents already use, so a folder copied out of `~/.claude/skills/` installs
//! here unchanged.
//!
//! Layout of the module:
//! - [`manifest`] parses and validates `SKILL.md` (pure, no I/O beyond one read);
//! - [`install`] installs from a folder, a zip or a git repository, lists and
//!   removes, all under `<app_data>/llm/skills/<name>/`;
//! - [`inject`] builds the short index that goes in the system prompt and opens
//!   one skill's body on demand (progressive disclosure);
//! - [`catalog`] carries the pinned `OpenRouterTeam/skills` set;
//! - [`scan`] runs NVIDIA SkillSpector over the staged copy before the install
//!   is committed, when that tool is on the `PATH`.
//!
//! Budget: parsing happens at install time, never inside a turn.
//! [`inject::index_prompt`] is the only thing that runs per turn and it only
//! concatenates strings.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

pub mod catalog;
pub mod deps;
pub mod hash;
pub mod inject;
pub mod install;
pub mod manifest;
pub mod scan;

pub use deps::{dependencies, DepKind, Dependency};
pub use hash::dir_hash;
pub use inject::{exposed_tool_name, index_prompt, index_prompt_named, open, tool_specs};
pub use install::{
    confirm_install, discard_install, install_from_dir, install_from_git, install_from_zip, list,
    remove, skills_dir, InstallOutcome,
};
pub use manifest::{parse, SkillManifest};
pub use scan::{ScanStatus, RISK_THRESHOLD};

/// `SKILL.md` is missing, malformed, or not valid UTF-8.
pub const ERR_SKILL_PARSE: &str = "ERR_SKILL_PARSE";
/// The `name` field breaks the spec, or a caller passed a name with a path
/// separator in it.
pub const ERR_SKILL_NAME: &str = "ERR_SKILL_NAME";
/// The `description` field is missing or empty.
pub const ERR_SKILL_DESCRIPTION: &str = "ERR_SKILL_DESCRIPTION";
/// Filesystem error while reading, copying or deleting.
pub const ERR_SKILL_IO: &str = "ERR_SKILL_IO";
/// No skill with that name is installed.
pub const ERR_SKILL_NOT_FOUND: &str = "ERR_SKILL_NOT_FOUND";
/// The zip is unreadable, or an entry escapes the destination directory.
pub const ERR_SKILL_ZIP: &str = "ERR_SKILL_ZIP";
/// `git` is missing, the URL was refused, or the clone failed.
pub const ERR_SKILL_GIT: &str = "ERR_SKILL_GIT";
/// The skill is over one of the install limits (file count, total bytes,
/// `SKILL.md` size).
pub const ERR_SKILL_TOO_BIG: &str = "ERR_SKILL_TOO_BIG";
/// The skill changed (updated, edited or removed) after the turn that is
/// asking for it started; nothing was read.
pub const ERR_SKILL_CHANGED: &str = "ERR_SKILL_CHANGED";
/// The bot asking for this skill is not bound to it, or the binding does not
/// allow reading it.
pub const ERR_SKILL_NOT_BOUND: &str = "ERR_SKILL_NOT_BOUND";
/// A path argument pointed outside the directory it had to stay in.
pub const ERR_SKILL_PATH: &str = "ERR_SKILL_PATH";
/// The confirm/discard token does not name a quarantined install. Either it was
/// made up, or the pending install was already confirmed, discarded, or lost to
/// a restart.
pub const ERR_SKILL_PENDING: &str = "ERR_SKILL_PENDING";

/// Error with a stable code the UI can map, in the shape of `LlmError`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillError {
    pub code: Cow<'static, str>,
    pub message: String,
}

impl SkillError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: Cow::Borrowed(code),
            message: message.into(),
        }
    }

    pub fn io(context: &str, err: &std::io::Error) -> Self {
        Self::new(ERR_SKILL_IO, format!("{context}: {err}"))
    }

    pub fn code(&self) -> &str {
        &self.code
    }
}

impl std::fmt::Display for SkillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SkillError {}

impl From<SkillError> for crate::core::llm::error::LlmError {
    fn from(err: SkillError) -> Self {
        crate::core::llm::error::LlmError {
            code: err.code,
            message: err.message,
            retryable: false,
            retry_after_ms: None,
        }
    }
}

/// Where an installed skill came from. Persisted next to the skill in
/// [`install::SIDECAR`] so the UI can show the origin and offer an update.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillSource {
    /// Copied from a local folder.
    Dir { from: String },
    /// Extracted from a local zip.
    Zip { from: String },
    /// Shallow-cloned from a git remote.
    Git {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subdir: Option<String>,
    },
    /// Read from disk with no sidecar: the folder was put there by hand.
    #[default]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_maps_to_llm_error_keeping_the_code() {
        let err = SkillError::new(ERR_SKILL_GIT, "git not found");
        let llm: crate::core::llm::error::LlmError = err.into();
        assert_eq!(llm.code, ERR_SKILL_GIT);
        assert!(!llm.retryable);
    }

    #[test]
    fn source_round_trips_through_json() {
        let src = SkillSource::Git {
            url: "https://example.invalid/skills.git".into(),
            rev: Some("abc".into()),
            subdir: None,
        };
        let json = serde_json::to_string(&src).unwrap();
        assert!(json.contains("\"kind\":\"git\""), "{json}");
        assert!(!json.contains("subdir"), "{json}");
        let back: SkillSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, src);
    }
}
