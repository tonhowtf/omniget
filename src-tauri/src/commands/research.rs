pub mod ads;
pub mod catalog;
pub mod client;
pub mod mcp;
pub mod reels;
pub mod stats;
pub mod tiktok;

use serde_json::{json, Value};

pub(crate) static NULL: Value = Value::Null;

pub(crate) fn fail(
    code: &str,
    message: &str,
    retryable: bool,
    next_step: &str,
    extra: Value,
) -> String {
    let mut detail = json!({
        "message": message,
        "retryable": retryable,
        "next_step": next_step,
    });
    if let Value::Object(more) = extra {
        for (key, value) in more {
            detail[key] = value;
        }
    }
    format!("{code}: {detail}")
}

pub(crate) fn uint(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub(crate) fn strings(value: &Value, max_items: usize, max_len: usize) -> Vec<String> {
    let raw: Vec<&str> = match value {
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        Value::String(one) => vec![one.as_str()],
        _ => Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for item in raw {
        let item: String = item.trim().chars().take(max_len).collect();
        if !item.is_empty() && !out.contains(&item) {
            out.push(item);
        }
    }
    out.truncate(max_items);
    out
}

#[tauri::command]
pub async fn research_key_status() -> Result<Value, String> {
    Ok(json!({ "configured": client::key_configured().await? }))
}

#[tauri::command]
pub async fn research_key_set(key: String) -> Result<Value, String> {
    client::set_key(key).await?;
    Ok(json!({ "configured": true }))
}

#[tauri::command]
pub async fn research_key_clear() -> Result<Value, String> {
    client::clear_key().await?;
    Ok(json!({ "configured": false }))
}

#[tauri::command]
pub async fn research_tool(name: String, args: Value) -> Result<Value, String> {
    mcp::call(&name, &args).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_is_a_code_followed_by_json_with_the_recovery_fields() {
        let error = fail(
            "NOT_FOUND",
            "nothing",
            false,
            "check it",
            json!({"status": 404}),
        );
        let (code, detail) = error.split_once(": ").unwrap();
        let detail: Value = serde_json::from_str(detail).unwrap();
        assert_eq!(code, "NOT_FOUND");
        assert_eq!(
            detail,
            json!({"message": "nothing", "retryable": false, "next_step": "check it", "status": 404})
        );
    }

    #[test]
    fn counts_are_read_from_numbers_and_numeric_strings() {
        assert_eq!(uint(&json!(12)), Some(12));
        assert_eq!(uint(&json!(12.9)), Some(12));
        assert_eq!(uint(&json!("41136495")), Some(41_136_495));
        assert_eq!(uint(&json!(-1)), None);
        assert_eq!(uint(&json!(null)), None);
    }

    #[test]
    fn string_lists_are_trimmed_deduplicated_and_capped() {
        assert_eq!(
            strings(&json!([" a ", "a", "", 3, "bcd"]), 5, 2),
            vec!["a", "bc"]
        );
        assert_eq!(strings(&json!(["a", "b", "c"]), 2, 10), vec!["a", "b"]);
        assert_eq!(strings(&json!("one"), 2, 10), vec!["one"]);
        assert!(strings(&json!(null), 2, 10).is_empty());
    }
}
