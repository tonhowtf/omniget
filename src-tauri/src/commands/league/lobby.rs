use serde_json::Value;

pub fn is_leader(lobby: &Value) -> bool {
    lobby
        .get("localMember")
        .and_then(|m| m.get("isLeader"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub fn pending_swaps(session: &Value) -> Vec<(&'static str, i64)> {
    let mut pending = Vec::new();
    for (field, kind) in [
        ("positionSwaps", "position-swaps"),
        ("pickOrderSwaps", "pick-order-swaps"),
    ] {
        for entry in session
            .get(field)
            .and_then(Value::as_array)
            .unwrap_or(&vec![])
        {
            let state = entry.get("state").and_then(Value::as_str).unwrap_or("");
            let id = entry.get("id").and_then(Value::as_i64).unwrap_or(-1);
            if state == "RECEIVED" && id >= 0 {
                pending.push((kind, id));
            }
        }
    }
    pending
}
