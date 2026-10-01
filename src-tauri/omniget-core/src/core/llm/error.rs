use std::borrow::Cow;

use serde::{Deserialize, Serialize};

pub const ERR_LLM_AUTH: &str = "ERR_LLM_AUTH";
pub const ERR_LLM_RATE: &str = "ERR_LLM_RATE";
pub const ERR_LLM_NET: &str = "ERR_LLM_NET";
pub const ERR_LLM_CANCELLED: &str = "ERR_LLM_CANCELLED";
pub const ERR_LLM_MODEL: &str = "ERR_LLM_MODEL";
pub const ERR_LLM_PARSE: &str = "ERR_LLM_PARSE";
pub const ERR_LLM_BUDGET: &str = "ERR_LLM_BUDGET";
pub const ERR_LLM_STUB: &str = "ERR_LLM_STUB";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmError {
    pub code: Cow<'static, str>,
    pub message: String,
    pub retryable: bool,
    pub retry_after_ms: Option<u64>,
}

impl LlmError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: Cow::Borrowed(code),
            message: message.into(),
            retryable: false,
            retry_after_ms: None,
        }
    }

    pub fn stub() -> Self {
        Self::new(ERR_LLM_STUB, "not implemented yet")
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for LlmError {}
