use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use super::error::LlmError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderId(pub String);

impl ProviderId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider: ProviderId,
    pub model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
    },
    Image {
        mime: String,
        data_b64: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<ContentPart>,
}

impl Message {
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            parts: vec![ContentPart::Text { text: text.into() }],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenParams {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub stop: Vec<String>,
    #[serde(default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub model: ModelRef,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub params: GenParams,
    pub cancel: CancellationToken,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub first_token_ms: Option<u32>,
    pub total_ms: u32,
    pub cost_usd: Option<f64>,
}

pub const CACHE_READ_WEIGHT: f64 = 0.1;
pub const CACHE_WRITE_WEIGHT: f64 = 1.25;

pub fn billable_input(input: u64, cache_read: u64, cache_write: u64) -> u64 {
    let cached = cache_read.saturating_add(cache_write);
    let fresh = if input >= cached {
        input - cached
    } else {
        input
    };
    fresh
        + (cache_read as f64 * CACHE_READ_WEIGHT).ceil() as u64
        + (cache_write as f64 * CACHE_WRITE_WEIGHT).ceil() as u64
}

impl Usage {
    pub fn billable_input_tokens(&self) -> u64 {
        billable_input(
            self.input_tokens as u64,
            self.cache_read_tokens as u64,
            self.cache_write_tokens as u64,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolUse,
    ContentFilter,
    Cancelled,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnEvent {
    Started {
        request_id: String,
    },
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallDelta {
        id: String,
        input_json_delta: String,
    },
    ToolCallEnd {
        id: String,
    },
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
    Usage {
        usage: Usage,
    },
    PruneReceipt {
        request: u32,
        omitted: u32,
        est_tokens_before: u32,
        est_tokens_after: u32,
        input_tokens: Option<u32>,
        #[serde(default)]
        billable_input_tokens: Option<u32>,
    },
    Finished {
        reason: FinishReason,
    },
    Error {
        error: LlmError,
    },
}
