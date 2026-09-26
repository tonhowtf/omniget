//! The tools a bot can call on its memory, its reading journeys, the web and
//! its group, behind one registry.
//!
//! Two doors, one implementation:
//! - native runtime: the broker source `assist` ([`AssistExecutor`]); the
//!   scope comes from `code_tools::current_turn()` (agent id = bot id,
//!   conversation id), resolved by [`super::ctx::resolve`];
//! - CLI/ACP runtimes: the scoped MCP projection ([`super::projection`]); the
//!   scope comes from the per-session token.
//!
//! Tool names are bare `snake_case` (provider tool-name rules) and unique
//! across toolsets. Grants are ordinary `ToolSource::Internal { name }`
//! grants on the agent, so "a box ticked" and "a tool offered" stay the same
//! check in the broker (`specs_for`).
//!
//! Everything a tool returns is data; web and skill content never widens a
//! scope or a grant (spec A20).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::ctx::AssistCtx;
use crate::core::llm::broker::ToolExecutor;
use crate::core::llm::error::LlmError;
use crate::core::llm::types::ToolSpec;

/// Broker source name of the assistant tools.
pub const ASSIST_SOURCE: &str = "assist";
pub const ERR_ASSIST_TOOL: &str = "ERR_ASSIST_TOOL";
pub const ERR_ASSIST_SCOPE: &str = "ERR_ASSIST_SCOPE";

#[async_trait]
pub trait AssistToolset: Send + Sync {
    /// Short owner name, for diagnostics.
    fn name(&self) -> &'static str;
    fn specs(&self) -> Vec<ToolSpec>;
    /// Runs one of this toolset's tools. `Err` carries a stable `ERR_*` code
    /// at the start of the string.
    async fn call(&self, ctx: &AssistCtx, tool: &str, input: Value) -> Result<Value, String>;
}

/// Every toolset, in a fixed order.
pub fn toolsets() -> Vec<Arc<dyn AssistToolset>> {
    vec![
        super::memory::toolset(),
        super::reading::toolset(),
        super::web::toolset(),
        super::groups::toolset(),
        super::bots::toolset(),
        super::media_tools::toolset(),
    ]
}

pub fn all_specs() -> Vec<ToolSpec> {
    toolsets().iter().flat_map(|t| t.specs()).collect()
}

/// Names of every assistant tool.
pub fn names() -> Vec<String> {
    all_specs().into_iter().map(|s| s.name).collect()
}

/// Runs `tool` with an explicit context (the MCP projection, tests).
pub async fn call_with_ctx(ctx: &AssistCtx, tool: &str, input: Value) -> Result<Value, String> {
    for set in toolsets() {
        if set.specs().iter().any(|s| s.name == tool) {
            let out = set.call(ctx, tool, input).await;
            super::runs::note_tool_use(ctx, tool, out.is_ok());
            return out;
        }
    }
    Err(format!("{ERR_ASSIST_TOOL}: unknown tool `{tool}`"))
}

/// The broker's door. Registered by the app with
/// `broker.register_source(ASSIST_SOURCE, all_specs(), Arc::new(AssistExecutor))`.
pub struct AssistExecutor;

#[async_trait]
impl ToolExecutor for AssistExecutor {
    async fn execute(&self, name: &str, input: Value) -> Result<String, LlmError> {
        let Some(turn) = crate::core::llm::code_tools::current_turn() else {
            return Err(LlmError::new(
                ERR_ASSIST_SCOPE,
                format!("`{name}` runs only inside a bot turn"),
            ));
        };
        let ctx = super::ctx::resolve(&turn.agent, Some(&turn.conversation))
            .with_run(Some(turn.request.clone()));
        match call_with_ctx(&ctx, name, input).await {
            Ok(v) => Ok(match v {
                Value::String(s) => s,
                other => serde_json::to_string(&other).unwrap_or_default(),
            }),
            Err(e) => {
                let mut err = LlmError::new(ERR_ASSIST_TOOL, e.clone());
                if let Some(code) = e.split(':').next().filter(|c| c.starts_with("ERR_")) {
                    err.code = std::borrow::Cow::Owned(code.to_string());
                }
                Err(err)
            }
        }
    }
}

/// A toolset with no tools (a module that offers none yet).
pub struct EmptyToolset(pub &'static str);

#[async_trait]
impl AssistToolset for EmptyToolset {
    fn name(&self) -> &'static str {
        self.0
    }
    fn specs(&self) -> Vec<ToolSpec> {
        Vec::new()
    }
    async fn call(&self, _ctx: &AssistCtx, tool: &str, _input: Value) -> Result<Value, String> {
        Err(format!("{ERR_ASSIST_TOOL}: unknown tool `{tool}`"))
    }
}

/// Small helpers for toolset implementations.
pub fn spec(name: &str, description: &str, input_schema: Value) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

pub fn need_str<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{ERR_ASSIST_TOOL}: `{key}` is required"))
}

pub fn opt_str<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_unique_and_provider_safe() {
        let names = names();
        let mut seen = std::collections::HashSet::new();
        for n in &names {
            assert!(seen.insert(n.clone()), "duplicate tool {n}");
            assert!(
                n.len() <= 64
                    && n.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "tool name {n} breaks provider rules"
            );
        }
    }
}
