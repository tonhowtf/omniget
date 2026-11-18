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
pub use inject::{exposed_tool_name, index_prompt_named};
pub use install::{skills_dir, InstallOutcome};
pub use manifest::{parse, SkillManifest};
pub use scan::{ScanStatus, RISK_THRESHOLD};

pub const ERR_SKILL_PARSE: &str = "ERR_SKILL_PARSE";
pub const ERR_SKILL_NAME: &str = "ERR_SKILL_NAME";
pub const ERR_SKILL_DESCRIPTION: &str = "ERR_SKILL_DESCRIPTION";
pub const ERR_SKILL_IO: &str = "ERR_SKILL_IO";
pub const ERR_SKILL_NOT_FOUND: &str = "ERR_SKILL_NOT_FOUND";
pub const ERR_SKILL_GIT: &str = "ERR_SKILL_GIT";
pub const ERR_SKILL_TOO_BIG: &str = "ERR_SKILL_TOO_BIG";
pub const ERR_SKILL_CHANGED: &str = "ERR_SKILL_CHANGED";
pub const ERR_SKILL_NOT_BOUND: &str = "ERR_SKILL_NOT_BOUND";
pub const ERR_SKILL_PATH: &str = "ERR_SKILL_PATH";

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillSource {
    Dir {
        from: String,
    },
    Zip {
        from: String,
    },
    Git {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subdir: Option<String>,
    },
    #[default]
    Unknown,
}
