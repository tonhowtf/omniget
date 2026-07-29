use serde_json::Value;

/// Only the lobby leader can start matchmaking, so anything that re-queues has to
/// check this first — asking as a member just fails.
pub fn is_leader(lobby: &Value) -> bool {
    lobby
        .get("localMember")
        .and_then(|m| m.get("isLeader"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Swap requests waiting for an answer, as `(kind, id)`. `kind` is the path
/// segment the client expects back.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn leadership_comes_from_the_local_member_flag() {
        assert!(is_leader(&json!({ "localMember": { "isLeader": true } })));
        assert!(!is_leader(&json!({ "localMember": { "isLeader": false } })));
        // A missing flag must never be read as "I am the leader".
        assert!(!is_leader(&json!({ "localMember": {} })));
        assert!(!is_leader(&json!({})));
        assert!(!is_leader(&Value::Null));
    }

    #[test]
    fn only_requests_waiting_for_an_answer_are_returned() {
        let session = json!({
            "positionSwaps": [
                { "id": 1, "state": "RECEIVED" },
                { "id": 2, "state": "SENT" },
                { "id": 3, "state": "INVALID" }
            ],
            "pickOrderSwaps": [
                { "id": 7, "state": "RECEIVED" },
                { "id": 8, "state": "ACCEPTED" }
            ]
        });
        assert_eq!(
            pending_swaps(&session),
            vec![("position-swaps", 1), ("pick-order-swaps", 7)]
        );
    }

    #[test]
    fn a_session_without_swap_fields_is_handled() {
        assert!(pending_swaps(&json!({})).is_empty());
        assert!(pending_swaps(&json!({ "positionSwaps": [] })).is_empty());
        // Malformed entries are skipped rather than panicking.
        assert!(
            pending_swaps(&json!({ "positionSwaps": [{}, { "state": "RECEIVED" }] })).is_empty()
        );
    }
}
