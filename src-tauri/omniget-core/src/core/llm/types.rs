//! Shared LLM types: the contract every Phase 2 agent codes against.
//! Owned by f2-llm-providers. DRAFT written by the orchestrator from
//! docs/agents/f2-llm-providers.md §5 so the six other agents can start today;
//! the owner finalises it with additive changes and announces breaking ones in
//! its handoff first. `ProviderId` wraps the `ai_keys::Kind::id` string
//! ("openai", "anthropic", "openrouter", "ollama", …) because `Kind` is a
//! struct table, not an enum.

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

/// One model call. `cancel` is not serialisable on purpose: it lives for the
/// duration of the turn only.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub model: ModelRef,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub params: GenParams,
    pub cancel: CancellationToken,
    pub agent_id: Option<String>,
}

/// One convention for every runtime: `input_tokens` is the WHOLE input the
/// model processed, cache included (OpenAI `prompt_tokens` style);
/// `cache_read_tokens` and `cache_write_tokens` are slices of it. Caps bill
/// [`Usage::billable_tokens`], never the raw sum.
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

/// Price of a cache read relative to fresh input (Anthropic 0.1x).
pub const CACHE_READ_WEIGHT: f64 = 0.1;
/// Price of a cache write relative to fresh input (Anthropic 5-minute 1.25x).
pub const CACHE_WRITE_WEIGHT: f64 = 1.25;

/// Input tokens weighted by price, in fresh-input equivalents:
/// `fresh + ceil(0.1 * read) + ceil(1.25 * write)`.
///
/// `input` is the whole input (cache included). A legacy record in the raw
/// Anthropic convention (input = fresh only, so `input < read + write`) is
/// read as fresh-only, so stored jobs keep billing right.
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
    /// Input weighted by price ([`billable_input`]).
    pub fn billable_input_tokens(&self) -> u64 {
        billable_input(
            self.input_tokens as u64,
            self.cache_read_tokens as u64,
            self.cache_write_tokens as u64,
        )
    }

    /// What budget caps count: weighted input plus output.
    pub fn billable_tokens(&self) -> u64 {
        self.billable_input_tokens() + self.output_tokens as u64
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
    /// What a CLI or ACP agent's own tool answered. Native tools never emit
    /// this: the Coordinator runs them and already has the result.
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
    Usage {
        usage: Usage,
    },
    /// One per model request of a turn while context pruning is on: what the
    /// history weighed before and after the omissions (the same chars/4
    /// estimate the budget gate uses) and what the provider then billed as
    /// input. Measured values only; no ratio is derived here.
    PruneReceipt {
        request: u32,
        omitted: u32,
        est_tokens_before: u32,
        est_tokens_after: u32,
        input_tokens: Option<u32>,
        /// `input_tokens` weighted by price ([`Usage::billable_input_tokens`]),
        /// what a cap checked between requests counts.
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

#[cfg(test)]
mod billing_tests {
    use super::*;

    fn usage(input: u32, read: u32, write: u32, output: u32) -> Usage {
        Usage {
            input_tokens: input,
            cache_read_tokens: read,
            cache_write_tokens: write,
            output_tokens: output,
            ..Usage::default()
        }
    }

    #[test]
    fn the_mission_example_bills_130k_not_945k() {
        // Whole-input convention (cache included).
        let u = usage(20 + 906_405 + 1_000, 906_405, 1_000, 38_291);
        assert_eq!(u.billable_tokens(), 130_202);
        // Legacy raw Anthropic record (input = fresh only) bills the same.
        let legacy = usage(20, 906_405, 1_000, 38_291);
        assert_eq!(legacy.billable_tokens(), 130_202);
    }

    #[test]
    fn a_native_total_input_is_not_counted_twice() {
        let u = usage(50_000, 40_000, 0, 0);
        assert_eq!(u.billable_input_tokens(), 14_000);
    }

    #[test]
    fn no_cache_bills_input_plus_output() {
        assert_eq!(usage(100, 0, 0, 50).billable_tokens(), 150);
    }

    #[test]
    fn old_receipts_without_billable_still_deserialize() {
        let e: TurnEvent = serde_json::from_value(serde_json::json!({
            "type": "prune_receipt", "request": 1, "omitted": 0,
            "est_tokens_before": 10, "est_tokens_after": 5, "input_tokens": 7
        }))
        .unwrap();
        assert!(matches!(
            e,
            TurnEvent::PruneReceipt {
                billable_input_tokens: None,
                ..
            }
        ));
        let u: Usage = serde_json::from_value(serde_json::json!({"input_tokens": 3})).unwrap();
        assert_eq!(u.input_tokens, 3);
    }
}
