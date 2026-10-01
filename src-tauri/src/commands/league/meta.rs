use serde_json::Value;

pub fn most_played(entries: &[Value]) -> Option<&Value> {
    entries
        .iter()
        .max_by_key(|e| e.get("play").and_then(Value::as_i64).unwrap_or(0))
}

pub fn id_list(entry: Option<&Value>) -> Vec<i64> {
    entry
        .and_then(|e| e.get("ids"))
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

pub fn variant_winrate(entry: Option<&Value>) -> Option<f64> {
    let entry = entry?;
    let play = entry.get("play").and_then(Value::as_i64).unwrap_or(0);
    let win = entry.get("win").and_then(Value::as_i64).unwrap_or(0);
    if play < 5 {
        return None;
    }
    Some(((win as f64 / play as f64) * 1000.0).round() / 10.0)
}

pub fn skill_order(skills: &[Value]) -> Vec<String> {
    most_played(skills)
        .and_then(|e| e.get("order"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn first_levels(order: &[String], count: usize) -> Vec<String> {
    order.iter().take(count).cloned().collect()
}

pub fn counters(raw: &Value) -> Vec<Value> {
    raw.get("counters")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    let id = c.get("champion_id").and_then(Value::as_i64)?;
                    let play = c.get("play").and_then(Value::as_i64).unwrap_or(0);
                    let win = c.get("win").and_then(Value::as_i64).unwrap_or(0);
                    Some(serde_json::json!({
                        "championId": id,
                        "games": play,
                        "winrate": if play > 0 {
                            serde_json::json!(((win as f64 / play as f64) * 1000.0).round() / 10.0)
                        } else {
                            Value::Null
                        },
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}
