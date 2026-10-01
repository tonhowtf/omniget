use std::borrow::Cow;

use serde::Serialize;

use crate::core::llm::error::{LlmError, ERR_LLM_NET};

pub const ERR_MCP_SPAWN: &str = "ERR_MCP_SPAWN";
pub const ERR_MCP_PROTO: &str = "ERR_MCP_PROTO";
pub const ERR_MCP_TIMEOUT: &str = "ERR_MCP_TIMEOUT";
pub const ERR_MCP_HTTP: &str = "ERR_MCP_HTTP";
pub const ERR_MCP_TOOL: &str = "ERR_MCP_TOOL";
pub const ERR_MCP_CANCELLED: &str = "ERR_MCP_CANCELLED";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl McpError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
        }
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    pub fn spawn(message: impl Into<String>) -> Self {
        Self::new(ERR_MCP_SPAWN, message)
    }

    pub fn proto(message: impl Into<String>) -> Self {
        Self::new(ERR_MCP_PROTO, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(ERR_MCP_TIMEOUT, message).retryable()
    }

    pub fn http(message: impl Into<String>) -> Self {
        Self::new(ERR_MCP_HTTP, message)
    }

    pub fn tool(message: impl Into<String>) -> Self {
        Self::new(ERR_MCP_TOOL, message)
    }

    pub fn cancelled() -> Self {
        Self::new(ERR_MCP_CANCELLED, "cancelled by the caller")
    }
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for McpError {}

impl From<McpError> for LlmError {
    fn from(e: McpError) -> Self {
        LlmError {
            code: Cow::Borrowed(if e.code.is_empty() {
                ERR_LLM_NET
            } else {
                e.code
            }),
            message: e.message,
            retryable: e.retryable,
            retry_after_ms: None,
        }
    }
}

pub fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn from_status(status: u16, body: &str) -> McpError {
    let body = body.trim();
    let tail = if body.is_empty() {
        String::new()
    } else {
        format!(": {}", clip(body, 200))
    };
    let err = McpError::http(format!("HTTP {}{}", status, tail));
    if status == 408 || status == 429 || status >= 500 {
        err.retryable()
    } else {
        err
    }
}
