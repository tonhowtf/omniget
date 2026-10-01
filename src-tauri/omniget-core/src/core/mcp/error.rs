//! Stable error codes of the MCP client. Owned by f3-mcp-core.
//!
//! Same shape as `StreamError::code()` and `LlmError`: a `&'static str` the UI
//! can map to a sentence, a message meant for a log, and a `retryable` flag the
//! caller uses to decide whether pressing the button again can help.

use std::borrow::Cow;

use serde::Serialize;

use crate::core::llm::error::{LlmError, ERR_LLM_NET};

/// The process could not be started (binary missing, cwd gone, permission).
pub const ERR_MCP_SPAWN: &str = "ERR_MCP_SPAWN";
/// The peer is not speaking MCP: bad JSON, no response, stream cut in half.
pub const ERR_MCP_PROTO: &str = "ERR_MCP_PROTO";
/// The request took longer than the per-call deadline.
pub const ERR_MCP_TIMEOUT: &str = "ERR_MCP_TIMEOUT";
/// HTTP transport said no (status code, TLS, DNS, expired session).
pub const ERR_MCP_HTTP: &str = "ERR_MCP_HTTP";
/// The tool itself failed (`isError: true`, or a JSON-RPC error on `tools/call`).
pub const ERR_MCP_TOOL: &str = "ERR_MCP_TOOL";
/// The caller's `CancellationToken` fired. Extension over the five codes in the
/// prompt's contract: mapping a user cancel onto `ERR_MCP_TIMEOUT` would make
/// the UI say "the server is slow" when the user pressed stop.
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

    /// Same error, but the caller is told that trying again may work.
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

    /// Timeouts are retryable by definition: nothing says the server is broken.
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

/// The broker's `ToolExecutor` returns `LlmError`, so the code has to survive
/// the crossing: it is kept verbatim in `LlmError::code` (a `Cow`), which is
/// why the UI can still tell `ERR_MCP_SPAWN` from `ERR_MCP_TOOL`.
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

/// Truncates on a char boundary: slicing a UTF-8 string by byte index panics,
/// and a server's error body is exactly where a stray multi-byte character
/// shows up.
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

/// HTTP status → error. 401/403 are not retryable; 408/429/5xx are.
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
