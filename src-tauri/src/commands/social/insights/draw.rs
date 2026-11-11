use std::collections::HashSet;

use sha2::{Digest, Sha256};

pub const METHOD: &str = "Handles are lowercased, deduplicated and sorted by SHA-256(seed + ':' + handle) as hex, ascending. Conditions are checked in that order until the places are filled.";

pub fn normalize(handle: &str) -> String {
    handle.trim().trim_start_matches('@').to_lowercase()
}

pub fn pool(handles: &[String], exclude: &[String]) -> Vec<String> {
    let skip: HashSet<String> = exclude.iter().map(|h| normalize(h)).collect();
    let mut seen = HashSet::new();
    let mut out: Vec<String> = handles
        .iter()
        .map(|h| normalize(h))
        .filter(|h| !h.is_empty() && !skip.contains(h) && seen.insert(h.clone()))
        .collect();
    out.sort();
    out
}

pub fn rank(seed: &str, handle: &str) -> String {
    hex::encode(Sha256::digest(format!("{seed}:{handle}").as_bytes()))
}

pub fn order(seed: &str, pool: &[String]) -> Vec<String> {
    let mut ranked: Vec<(String, &String)> = pool.iter().map(|h| (rank(seed, h), h)).collect();
    ranked.sort();
    ranked.into_iter().map(|(_, h)| h.clone()).collect()
}

pub fn split(eligible: Vec<String>, winners: usize) -> (Vec<String>, Vec<String>) {
    let mut first = eligible;
    let rest = first.split_off(winners.min(first.len()));
    (first, rest)
}

fn cell(raw: &str) -> String {
    let guarded = if raw.starts_with(['=', '+', '-', '@']) {
        format!("'{raw}")
    } else {
        raw.to_string()
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

pub fn csv(result: &serde_json::Value) -> String {
    let seed = result["seed"].as_str().unwrap_or_default();
    let platform = result["platform"].as_str().unwrap_or_default();
    let mut out = String::from("place,role,handle,rank,platform,seed\r\n");
    let mut place = 0;
    for (role, key) in [("winner", "winners"), ("runner_up", "runner_ups")] {
        for entry in result[key]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            place += 1;
            out.push_str(&format!(
                "{place},{role},{},{},{},{}\r\n",
                cell(entry["handle"].as_str().unwrap_or_default()),
                cell(entry["rank"].as_str().unwrap_or_default()),
                cell(platform),
                cell(seed),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_exports_as_csv_with_winners_first_and_safe_cells() {
        let result = serde_json::json!({
            "platform": "x",
            "seed": "launch, \"2026\"",
            "winners": [{ "handle": "ana", "rank": "0a" }, { "handle": "=cmd", "rank": "1b" }],
            "runner_ups": [{ "handle": "bob", "rank": "2c" }],
        });
        let text = csv(&result);
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(lines[0], "place,role,handle,rank,platform,seed");
        assert_eq!(lines[1], "1,winner,ana,0a,x,\"launch, \"\"2026\"\"\"");
        assert!(lines[2].starts_with("2,winner,'=cmd,1b,x,"));
        assert!(lines[3].starts_with("3,runner_up,bob,2c,"));
        assert_eq!(lines.len(), 5);
        assert_eq!(
            csv(&serde_json::json!({})),
            "place,role,handle,rank,platform,seed\r\n"
        );
    }

    fn handles(list: &[&str]) -> Vec<String> {
        list.iter().map(|h| h.to_string()).collect()
    }

    #[test]
    fn the_pool_is_deduplicated_case_insensitive_and_respects_the_exclude_list() {
        assert_eq!(
            pool(
                &handles(&["@Ana", "ana ", "bob", "", "Carl", "dora"]),
                &handles(&["@CARL"])
            ),
            handles(&["ana", "bob", "dora"])
        );
    }

    #[test]
    fn the_same_seed_always_gives_the_same_order_whatever_the_input_order() {
        let a = pool(&handles(&["ana", "bob", "carl", "dora", "eve"]), &[]);
        let b = pool(&handles(&["eve", "dora", "carl", "bob", "ana"]), &[]);
        assert_eq!(order("seed-1", &a), order("seed-1", &b));
        assert_ne!(order("seed-1", &a), order("seed-2", &a));
        let mut sorted = order("seed-1", &a);
        sorted.sort();
        assert_eq!(sorted, a);
    }

    #[test]
    fn the_rank_can_be_checked_by_anyone_with_sha256() {
        assert_eq!(rank("abc", "ana"), hex::encode(Sha256::digest(b"abc:ana")));
        let ordered = order("abc", &handles(&["ana", "bob"]));
        assert!(rank("abc", &ordered[0]) < rank("abc", &ordered[1]));
    }

    #[test]
    fn winners_come_first_and_the_rest_are_runner_ups() {
        let (w, r) = split(handles(&["a", "b", "c"]), 2);
        assert_eq!(w, handles(&["a", "b"]));
        assert_eq!(r, handles(&["c"]));
        let (w, r) = split(handles(&["a"]), 3);
        assert_eq!(w, handles(&["a"]));
        assert!(r.is_empty());
    }
}
