pub mod client;
pub mod error;
pub mod registry;
pub mod stdio;
pub mod streamable_http;
pub mod types;

pub use client::McpClient;
pub use error::{
    McpError, ERR_MCP_CANCELLED, ERR_MCP_HTTP, ERR_MCP_PROTO, ERR_MCP_SPAWN, ERR_MCP_TIMEOUT,
    ERR_MCP_TOOL,
};
pub use registry::{McpRegistry, IDLE_TIMEOUT};
pub use stdio::StdioTransport;
pub use streamable_http::HttpTransport;
pub use types::{
    McpServerConfig, ServerInfo, ToolDef, Transport, DEFAULT_TIMEOUT_MS, PROTOCOL_VERSION,
};
