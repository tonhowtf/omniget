pub fn conversation_id(node_id: &str) -> String {
    format!("canvas-{node_id}")
}

pub fn agent_id(data: &serde_json::Value) -> Option<String> {
    ["botId", "roleId"]
        .iter()
        .find_map(|k| data.get(*k).and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
