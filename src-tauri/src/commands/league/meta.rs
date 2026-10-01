use serde_json::Value;

/// Picks the most played variant of a list of `{ids, play, win, pick_rate}`
/// entries. Most played beats highest win rate on purpose: a 100% win rate over
/// two games is noise.
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

/// Win rate of a variant as a percentage, when it has enough games to mean
/// something.
pub fn variant_winrate(entry: Option<&Value>) -> Option<f64> {
    let entry = entry?;
    let play = entry.get("play").and_then(Value::as_i64).unwrap_or(0);
    let win = entry.get("win").and_then(Value::as_i64).unwrap_or(0);
    if play < 5 {
        return None;
    }
    Some(((win as f64 / play as f64) * 1000.0).round() / 10.0)
}

/// Skill order as the letters the client uses, from the most played build.
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

/// The first three levels of the order, which is what people actually memorise.
pub fn first_levels(order: &[String], count: usize) -> Vec<String> {
    order.iter().take(count).cloned().collect()
}

/// Counter champions, when the source has them. The field exists but comes back
/// empty for plenty of champion/position pairs, so an empty list is normal and
/// must render as "no data" rather than as an error.
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
