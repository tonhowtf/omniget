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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn variants() -> Vec<Value> {
        vec![
            json!({ "ids": [1, 2], "play": 10, "win": 9 }),
            json!({ "ids": [3, 4], "play": 40, "win": 20 }),
            json!({ "ids": [5], "play": 2, "win": 2 }),
        ]
    }

    #[test]
    fn the_most_played_variant_wins_over_the_luckiest_one() {
        let entries = variants();
        let picked = most_played(&entries);
        assert_eq!(id_list(picked), vec![3, 4]);
    }

    #[test]
    fn a_variant_with_too_few_games_reports_no_winrate() {
        let entries = variants();
        assert_eq!(variant_winrate(Some(&entries[1])), Some(50.0));
        assert_eq!(variant_winrate(Some(&entries[2])), None);
        assert_eq!(variant_winrate(None), None);
    }

    #[test]
    fn empty_input_never_panics() {
        assert!(most_played(&[]).is_none());
        assert!(id_list(None).is_empty());
        assert!(skill_order(&[]).is_empty());
        assert!(counters(&json!({})).is_empty());
        assert!(counters(&json!({ "counters": [] })).is_empty());
    }

    #[test]
    fn the_skill_order_comes_from_the_most_played_build() {
        let skills = vec![
            json!({ "order": ["Q", "W", "E"], "play": 3 }),
            json!({ "order": ["E", "Q", "W", "Q", "Q", "R"], "play": 44 }),
        ];
        let order = skill_order(&skills);
        assert_eq!(order, vec!["E", "Q", "W", "Q", "Q", "R"]);
        assert_eq!(first_levels(&order, 3), vec!["E", "Q", "W"]);
        // Asking for more levels than exist just returns what there is.
        assert_eq!(first_levels(&order, 99).len(), 6);
    }

    #[test]
    fn counters_are_normalised_and_skip_entries_without_a_champion() {
        let raw = json!({ "counters": [
            { "champion_id": 22, "play": 40, "win": 30 },
            { "play": 10, "win": 5 }
        ]});
        let list = counters(&raw);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["championId"], 22);
        assert_eq!(list[0]["winrate"], 75.0);
    }
}
