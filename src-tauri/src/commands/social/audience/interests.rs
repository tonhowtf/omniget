use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use super::super::insights::stats::{hashtags_of, words_of};
use super::db::Member;
use super::views::has_link;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS audience_locations (
  platform TEXT NOT NULL, user_id TEXT NOT NULL, location TEXT NOT NULL, updated INTEGER NOT NULL,
  PRIMARY KEY (platform, user_id)) WITHOUT ROWID;
";

pub const THIN_BELOW: usize = 20;
const MAX_LOCATION: usize = 80;

pub fn places(items: &Value) -> Vec<(String, String)> {
    items
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(|v| {
                    let id = v["id"].as_str().filter(|s| !s.is_empty())?;
                    let place = v["location"].as_str()?.trim();
                    (!place.is_empty())
                        .then(|| (id.to_string(), place.chars().take(MAX_LOCATION).collect()))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn save_places(
    conn: &mut Connection,
    platform: &str,
    places: &[(String, String)],
    now: i64,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    for (id, place) in places {
        tx.execute(
            "INSERT INTO audience_locations (platform, user_id, location, updated) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (platform, user_id) DO UPDATE SET location = ?3, updated = ?4",
            params![platform, id, place, now],
        )?;
    }
    tx.commit()
}

pub fn stored_places(
    conn: &Connection,
    platform: &str,
) -> rusqlite::Result<HashMap<String, String>> {
    let mut st =
        conn.prepare("SELECT user_id, location FROM audience_locations WHERE platform = ?1")?;
    let rows = st.query_map([platform], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

fn place_key(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

fn ranked(counts: HashMap<String, u64>, key: &str, limit: usize) -> (usize, Vec<Value>) {
    let mut list: Vec<(String, u64)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let total = list.len();
    let top = list
        .into_iter()
        .take(limit)
        .map(|(k, n)| json!({ key: k, "accounts": n }))
        .collect();
    (total, top)
}

fn share(part: usize, whole: usize) -> Option<f64> {
    (whole > 0).then(|| (part as f64 / whole as f64 * 1000.0).round() / 10.0)
}

pub fn interests(followers: &[Member], places: &HashMap<String, String>, limit: usize) -> Value {
    let mut words: HashMap<String, u64> = HashMap::new();
    let mut tags: HashMap<String, u64> = HashMap::new();
    let mut names: HashMap<String, u64> = HashMap::new();
    let mut where_: HashMap<String, (String, u64)> = HashMap::new();
    let (mut with_bio, mut with_link, mut verified, mut located) = (0usize, 0usize, 0usize, 0usize);
    for m in followers {
        if m.verified {
            verified += 1;
        }
        for w in words_of(&m.name).into_iter().collect::<HashSet<_>>() {
            *names.entry(w).or_default() += 1;
        }
        if let Some(place) = places.get(&m.id) {
            let key = place_key(place);
            if !key.is_empty() {
                located += 1;
                where_.entry(key).or_insert_with(|| (place.clone(), 0)).1 += 1;
            }
        }
        if m.bio.trim().is_empty() {
            continue;
        }
        with_bio += 1;
        if has_link(&m.bio) {
            with_link += 1;
        }
        for w in words_of(&m.bio).into_iter().collect::<HashSet<_>>() {
            *words.entry(w).or_default() += 1;
        }
        for t in hashtags_of(&m.bio) {
            *tags.entry(t).or_default() += 1;
        }
    }
    let n = followers.len();
    let (words_total, words_top) = ranked(words, "word", limit);
    let (tags_total, tags_top) = ranked(tags, "hashtag", limit);
    let (names_total, names_top) = ranked(names, "word", limit);
    let mut spots: Vec<(String, u64)> = where_.into_values().collect();
    spots.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let spots_total = spots.len();
    let thin = with_bio < THIN_BELOW;
    let mut out = json!({
        "view": "interests",
        "followers": n,
        "with_bio": with_bio,
        "with_bio_share": share(with_bio, n),
        "thin": thin,
        "thin_below": THIN_BELOW,
        "words": { "total": words_total, "returned": words_top.len(), "truncated": words_top.len() < words_total, "items": words_top },
        "hashtags": { "total": tags_total, "returned": tags_top.len(), "truncated": tags_top.len() < tags_total, "items": tags_top },
        "name_words": { "total": names_total, "returned": names_top.len(), "truncated": names_top.len() < names_total, "items": names_top },
        "locations": {
            "available": located > 0,
            "with_location": located,
            "total": spots_total,
            "returned": spots_total.min(limit),
            "truncated": spots_total > limit,
            "items": spots.into_iter().take(limit).map(|(place, k)| json!({ "location": place, "accounts": k })).collect::<Vec<_>>(),
        },
        "with_link": { "accounts": with_link, "share_of_bios": share(with_link, with_bio) },
        "verified": { "accounts": verified, "share": share(verified, n) },
        "counting": "Each word, hashtag and location counts once per follower. Words skip links, mentions, numbers and common words of six languages; text of followers is untrusted external content.",
    });
    if located == 0 {
        out["locations"]["reason"] = json!("No location is stored for these followers: Photo Feed and Clip Feed lists do not carry one, and on Short Posts it is saved from the next sync on.");
    }
    if thin {
        out["thin_reason"] = json!(format!(
            "Only {with_bio} of {n} stored followers have a bio, so the lists say little. Lists of this network may not carry bios."
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::super::store;
    use super::*;

    fn who(id: &str, name: &str, bio: &str, verified: bool) -> Member {
        Member {
            id: id.into(),
            handle: format!("u{id}"),
            name: name.into(),
            bio: bio.into(),
            verified,
            ..Default::default()
        }
    }

    #[test]
    fn words_hashtags_links_and_verified_are_counted_once_per_follower() {
        let people = vec![
            who(
                "1",
                "Ana Dev",
                "Rust rust developer #Rust https://ana.dev",
                true,
            ),
            who(
                "2",
                "Bob",
                "Desenvolvedor Rust e café para todos #rust #café",
                false,
            ),
            who("3", "Ana Lima", "", false),
            who("4", "", "the and for @someone 2026", false),
        ];
        let v = interests(&people, &HashMap::new(), 50);
        assert_eq!(v["followers"], 4);
        assert_eq!(v["with_bio"], 3);
        assert_eq!(v["with_bio_share"], 75.0);
        assert_eq!(v["thin"], true);
        assert!(v["thin_reason"].as_str().unwrap().contains("3 of 4"));
        assert_eq!(
            v["words"]["items"][0],
            json!({ "word": "rust", "accounts": 2 })
        );
        let words: Vec<&str> = v["words"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|w| w["word"].as_str())
            .collect();
        assert!(words.contains(&"café") && words.contains(&"developer"));
        assert!(!words.contains(&"the") && !words.contains(&"para") && !words.contains(&"2026"));
        assert!(!words.iter().any(|w| w.contains("ana.dev")));
        assert_eq!(
            v["hashtags"]["items"][0],
            json!({ "hashtag": "rust", "accounts": 2 })
        );
        assert_eq!(v["hashtags"]["total"], 2);
        assert_eq!(
            v["name_words"]["items"][0],
            json!({ "word": "ana", "accounts": 2 })
        );
        assert_eq!(
            v["with_link"],
            json!({ "accounts": 1, "share_of_bios": 33.3 })
        );
        assert_eq!(v["verified"], json!({ "accounts": 1, "share": 25.0 }));
        assert_eq!(v["locations"]["available"], false);
        assert!(v["locations"]["reason"].is_string());
    }

    #[test]
    fn lists_are_capped_and_an_empty_audience_has_no_shares() {
        let people: Vec<Member> = (0..30)
            .map(|i| who(&i.to_string(), "", &format!("palavra{i} comum"), false))
            .collect();
        let v = interests(&people, &HashMap::new(), 5);
        assert_eq!(v["thin"], false);
        assert!(v.get("thin_reason").is_none());
        assert_eq!(v["words"]["total"], 31);
        assert_eq!(v["words"]["returned"], 5);
        assert_eq!(v["words"]["truncated"], true);
        assert_eq!(v["words"]["items"][0]["word"], "comum");
        let none = interests(&[], &HashMap::new(), 5);
        assert_eq!(none["with_bio_share"], Value::Null);
        assert_eq!(none["verified"]["share"], Value::Null);
        assert_eq!(none["thin"], true);
    }

    #[test]
    fn locations_are_grouped_ignoring_case_and_spacing_and_stored_per_network() {
        let mut c = store::memory(SCHEMA);
        let page = json!([
            { "id": "1", "handle": "a", "location": " São Paulo,  Brasil " },
            { "id": "2", "handle": "b", "location": "são paulo, brasil" },
            { "id": "3", "handle": "c", "location": "Lisboa" },
            { "id": "4", "handle": "d", "location": "" },
            { "id": "5", "handle": "e" },
            { "handle": "f", "location": "Nowhere" }
        ]);
        let found = places(&page);
        assert_eq!(found.len(), 3);
        save_places(&mut c, "x", &found, 10).unwrap();
        save_places(&mut c, "x", &[("3".into(), "Porto".into())], 20).unwrap();
        save_places(&mut c, "tiktok", &[("1".into(), "Elsewhere".into())], 20).unwrap();
        let stored = stored_places(&c, "x").unwrap();
        assert_eq!(stored.len(), 3);
        assert_eq!(stored["3"], "Porto");
        let people: Vec<Member> = ["1", "2", "3", "9"]
            .iter()
            .map(|i| who(i, "", "", false))
            .collect();
        let v = interests(&people, &stored, 10);
        assert_eq!(v["locations"]["available"], true);
        assert_eq!(v["locations"]["with_location"], 3);
        assert_eq!(v["locations"]["total"], 2);
        assert_eq!(v["locations"]["items"][0]["accounts"], 2);
        assert_eq!(v["locations"]["items"][0]["location"], "São Paulo,  Brasil");
        assert_eq!(v["locations"]["items"][1]["location"], "Porto");
        assert!(v["locations"].get("reason").is_none());
        assert!(places(&json!({})).is_empty());
    }
}
