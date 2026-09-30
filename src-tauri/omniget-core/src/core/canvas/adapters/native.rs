//! Native nodes (our own runtime): no PTY. A message becomes a user message
//! of the node's conversation, sent through the LLM coordinator, envelope
//! included; state and turn end come from the bus, the answer is the final
//! text of the turn.

/// The conversation a native node keeps across turns.
pub fn conversation_id(node_id: &str) -> String {
    format!("canvas-{node_id}")
}

/// The roster agent a native node runs: `botId` of its data, else `roleId`.
pub fn agent_id(data: &serde_json::Value) -> Option<String> {
    ["botId", "roleId"]
        .iter()
        .find_map(|k| data.get(*k).and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
