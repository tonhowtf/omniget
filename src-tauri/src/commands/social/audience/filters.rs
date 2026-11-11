use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};

use super::super::model::Platform;
use super::super::store;
use super::views::SORTS;
use super::{fail, now_s};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS audience_filters (
  platform TEXT NOT NULL, name TEXT NOT NULL COLLATE NOCASE, filter TEXT NOT NULL, created INTEGER NOT NULL,
  PRIMARY KEY (platform, name)) WITHOUT ROWID;
";

pub const MAX_SAVED: usize = 30;
pub const MAX_NAME: usize = 40;

pub const QUICK: [(&str, &str, &str); 5] = [
    (
        "likely_inactive",
        "5 posts or fewer in total; the inactive view checks real last-post dates",
        r#"{"max_posts":5}"#,
    ),
    (
        "no_avatar_no_bio",
        "default profile picture and an empty bio",
        r#"{"has_avatar":false,"has_bio":false}"#,
    ),
    (
        "follows_many_few_followers",
        "follows 1000 accounts or more with at most one follower per ten followed",
        r#"{"min_following":1000,"max_ratio":0.1}"#,
    ),
    ("verified", "verified accounts", r#"{"verified":true}"#),
    ("private", "private accounts", r#"{"private":true}"#),
];

const NUMBERS: [&str; 8] = [
    "min_followers",
    "max_followers",
    "min_following",
    "max_following",
    "min_posts",
    "max_posts",
    "min_ratio",
    "max_ratio",
];
const FLAGS: [&str; 6] = [
    "verified",
    "private",
    "has_avatar",
    "has_bio",
    "has_link",
    "kept",
];

fn invalid(message: &str) -> String {
    fail(
        "INVALID_ARGUMENTS",
        message,
        false,
        "Fix the argument and call the tool again.",
    )
}

pub fn quick(name: &str) -> Option<Map<String, Value>> {
    QUICK
        .iter()
        .find(|(id, _, _)| id.eq_ignore_ascii_case(name))
        .and_then(|(_, _, body)| serde_json::from_str(body).ok())
}

pub fn name_of(raw: &Value) -> Result<String, String> {
    let name = raw.as_str().unwrap_or_default().trim().to_string();
    let len = name.chars().count();
    if len == 0 || len > MAX_NAME || name.chars().any(char::is_control) {
        return Err(invalid(
            "name must be 1 to 40 characters without line breaks",
        ));
    }
    Ok(name)
}

pub fn clean(filter: &Value) -> Result<Map<String, Value>, String> {
    let given = filter
        .as_object()
        .ok_or_else(|| invalid("filter must be an object with the filter fields of the view"))?;
    let mut out = Map::new();
    for (k, v) in given {
        if v.is_null() {
            continue;
        }
        let key = k.as_str();
        let ok = if NUMBERS.contains(&key) {
            v.as_f64().is_some_and(|n| n >= 0.0)
        } else if FLAGS.contains(&key) {
            v.is_boolean()
        } else if key == "q" {
            v.as_str()
                .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 200)
        } else if key == "sort" {
            v.as_str()
                .is_some_and(|s| SORTS.iter().any(|(id, _)| *id == s))
        } else if key == "inactive_days" {
            v.as_i64().is_some_and(|n| (1..=3650).contains(&n))
        } else {
            return Err(invalid(&format!(
                "{key} cannot be saved in a filter; allowed: q, sort, inactive_days, {}, {}",
                NUMBERS.join(", "),
                FLAGS.join(", ")
            )));
        };
        if !ok {
            return Err(invalid(&format!("{key} has a value the view would reject")));
        }
        out.insert(k.clone(), v.clone());
    }
    if out.is_empty() {
        return Err(fail(
            "EMPTY_FILTER",
            "a filter needs at least one field",
            false,
            "Pass filter with fields such as {\"min_followers\": 1000, \"verified\": true}.",
        ));
    }
    Ok(out)
}

pub fn merge(a: &Value, saved: &Map<String, Value>) -> Value {
    let mut out = a.as_object().cloned().unwrap_or_default();
    out.remove("filter");
    for (k, v) in saved {
        if out.get(k).is_none_or(Value::is_null) {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

pub fn saved(conn: &Connection, platform: &str) -> rusqlite::Result<Vec<(String, Value, i64)>> {
    let mut st = conn.prepare(
        "SELECT name, filter, created FROM audience_filters WHERE platform = ?1 ORDER BY name",
    )?;
    let rows = st.query_map([platform], |r| {
        let body: String = r.get(1)?;
        Ok((
            r.get(0)?,
            serde_json::from_str(&body).unwrap_or_else(|_| json!({})),
            r.get(2)?,
        ))
    })?;
    rows.collect()
}

pub fn get(
    conn: &Connection,
    platform: &str,
    name: &str,
) -> rusqlite::Result<Option<Map<String, Value>>> {
    let body: Option<String> = conn
        .query_row(
            "SELECT filter FROM audience_filters WHERE platform = ?1 AND name = ?2",
            params![platform, name],
            |r| r.get(0),
        )
        .optional()?;
    Ok(body.and_then(|b| serde_json::from_str(&b).ok()))
}

#[derive(Debug, PartialEq)]
pub enum Saved {
    Created,
    Replaced,
    Full,
}

pub fn save(
    conn: &Connection,
    platform: &str,
    name: &str,
    filter: &Map<String, Value>,
    now: i64,
) -> rusqlite::Result<Saved> {
    let exists = get(conn, platform, name)?.is_some();
    if !exists && saved(conn, platform)?.len() >= MAX_SAVED {
        return Ok(Saved::Full);
    }
    conn.execute(
        "INSERT INTO audience_filters (platform, name, filter, created) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (platform, name) DO UPDATE SET filter = ?3",
        params![
            platform,
            name,
            Value::Object(filter.clone()).to_string(),
            now
        ],
    )?;
    Ok(if exists {
        Saved::Replaced
    } else {
        Saved::Created
    })
}

pub fn remove(conn: &Connection, platform: &str, name: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "DELETE FROM audience_filters WHERE platform = ?1 AND name = ?2",
        params![platform, name],
    )? > 0)
}

pub fn listing(conn: &Connection, platform: &str) -> rusqlite::Result<Value> {
    let quick: Vec<Value> = QUICK
        .iter()
        .map(|(id, about, body)| {
            json!({ "name": id, "kind": "quick", "about": about, "filter": serde_json::from_str::<Value>(body).unwrap_or_default() })
        })
        .collect();
    let own: Vec<Value> = saved(conn, platform)?
        .into_iter()
        .map(|(name, filter, created)| json!({ "name": name, "kind": "saved", "filter": filter, "created": created }))
        .collect();
    let total = quick.len() + own.len();
    Ok(json!({
        "view": "filters",
        "total": total,
        "returned": total,
        "truncated": false,
        "saved": own.len(),
        "max_saved": MAX_SAVED,
        "items": quick.into_iter().chain(own).collect::<Vec<_>>(),
    }))
}

fn not_found(p: Platform, name: &str) -> String {
    fail(
        "FILTER_NOT_FOUND",
        &format!("no quick or saved filter is called {name}"),
        false,
        &format!(
            "Call {}audience with view=filters to list the names.",
            p.prefix()
        ),
    )
}

pub fn list(p: Platform) -> Result<Value, String> {
    store::with(SCHEMA, |c| listing(c, p.id()))
}

pub fn apply(p: Platform, a: &Value) -> Result<(Value, Value), String> {
    if a["filter"].is_null() {
        return Ok((a.clone(), Value::Null));
    }
    let name = name_of(&a["filter"])?;
    let (kind, fields) = match quick(&name) {
        Some(f) => ("quick", f),
        None => (
            "saved",
            store::with(SCHEMA, |c| get(c, p.id(), &name))?.ok_or_else(|| not_found(p, &name))?,
        ),
    };
    Ok((
        merge(a, &fields),
        json!({ "name": name, "kind": kind, "filter": fields }),
    ))
}

pub fn mark(p: Platform, a: &Value) -> Result<Value, String> {
    let name = name_of(&a["name"])?;
    if quick(&name).is_some() {
        return Err(fail(
            "RESERVED_NAME",
            &format!("{name} is a quick filter and cannot be saved over or removed"),
            false,
            "Pick another name.",
        ));
    }
    if a["remove"].as_bool().unwrap_or(false) {
        let (removed, all) = store::with(SCHEMA, |c| {
            Ok((remove(c, p.id(), &name)?, listing(c, p.id())?))
        })?;
        if !removed {
            return Err(not_found(p, &name));
        }
        return Ok(
            json!({ "list": "filters", "name": name, "removed": true, "total": all["saved"], "filters": all["items"] }),
        );
    }
    let fields = clean(&a["filter"])?;
    let (state, all) = store::with(SCHEMA, |c| {
        Ok((
            save(c, p.id(), &name, &fields, now_s())?,
            listing(c, p.id())?,
        ))
    })?;
    if state == Saved::Full {
        return Err(fail(
            "TOO_MANY_FILTERS",
            "30 filters are already saved for this network",
            false,
            "Remove one with list=filters, name and remove=true, then save again.",
        ));
    }
    Ok(json!({
        "list": "filters",
        "name": name,
        "removed": false,
        "replaced": state == Saved::Replaced,
        "filter": fields,
        "total": all["saved"],
        "filters": all["items"],
        "next_step": format!("Apply it with {}audience and filter set to this name.", p.prefix()),
    }))
}

#[cfg(test)]
mod tests {
    use super::super::db::Member;
    use super::super::views::{Query, Row};
    use super::*;

    fn row(m: Member) -> Row {
        Row {
            ratio: super::super::views::ratio(&m),
            member: m,
            ..Default::default()
        }
    }

    fn matches(name: &str, m: Member) -> bool {
        let q = Query::parse(&Value::Object(quick(name).unwrap())).unwrap();
        q.filter.matches(&row(m))
    }

    #[test]
    fn quick_filters_match_what_their_name_says() {
        let ghost = Member {
            posts: Some(0),
            ..Default::default()
        };
        let busy = Member {
            posts: Some(900),
            avatar: Some("https://pbs.example/a.jpg".into()),
            bio: "rust".into(),
            verified: true,
            private: true,
            followers: Some(40),
            following: Some(2000),
            ..Default::default()
        };
        assert!(matches("likely_inactive", ghost.clone()));
        assert!(!matches("likely_inactive", busy.clone()));
        assert!(!matches("likely_inactive", Member::default()));
        assert!(matches("no_avatar_no_bio", ghost.clone()));
        assert!(!matches("no_avatar_no_bio", busy.clone()));
        assert!(matches("follows_many_few_followers", busy.clone()));
        assert!(!matches(
            "follows_many_few_followers",
            Member {
                followers: Some(5000),
                following: Some(2000),
                ..Default::default()
            }
        ));
        assert!(!matches("follows_many_few_followers", ghost.clone()));
        assert!(matches("verified", busy.clone()) && !matches("verified", ghost.clone()));
        assert!(matches("private", busy) && !matches("private", ghost));
        assert!(quick("VERIFIED").is_some());
        assert!(quick("nope").is_none());
        for (name, _, _) in QUICK {
            assert!(
                clean(&Value::Object(quick(name).unwrap())).is_ok(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_saved_filter_keeps_only_valid_filter_fields() {
        let ok = clean(&json!({ "min_followers": 100, "verified": true, "q": "rust", "sort": "followers", "max_ratio": null })).unwrap();
        assert_eq!(ok.len(), 4);
        for bad in [
            json!({ "limit": 5 }),
            json!({ "min_followers": -1 }),
            json!({ "verified": "yes" }),
            json!({ "sort": "nope" }),
            json!({ "q": " " }),
            json!({ "inactive_days": 0 }),
            json!("verified"),
        ] {
            assert!(
                clean(&bad).unwrap_err().starts_with("INVALID_ARGUMENTS"),
                "{bad}"
            );
        }
        let e = clean(&json!({ "verified": null })).unwrap_err();
        assert!(e.starts_with("EMPTY_FILTER"));
        let body: Value = serde_json::from_str(e.split_once(": ").unwrap().1).unwrap();
        assert!(body["next_step"].is_string());
        assert!(name_of(&json!(" Big ones ")).is_ok_and(|n| n == "Big ones"));
        assert!(name_of(&json!("")).is_err());
        assert!(name_of(&json!("a\nb")).is_err());
        assert!(name_of(&json!("x".repeat(41))).is_err());
        assert!(name_of(&json!(3)).is_err());
    }

    #[test]
    fn filters_are_saved_listed_replaced_and_removed_per_network() {
        let c = store::memory(SCHEMA);
        let f = clean(&json!({ "min_followers": 1000 })).unwrap();
        assert_eq!(save(&c, "x", "Big", &f, 10).unwrap(), Saved::Created);
        let g = clean(&json!({ "min_followers": 5000, "verified": true })).unwrap();
        assert_eq!(save(&c, "x", "big", &g, 20).unwrap(), Saved::Replaced);
        assert_eq!(get(&c, "x", "BIG").unwrap(), Some(g.clone()));
        assert_eq!(get(&c, "instagram", "big").unwrap(), None);
        let all = listing(&c, "x").unwrap();
        assert_eq!(all["total"], QUICK.len() + 1);
        assert_eq!(all["saved"], 1);
        assert_eq!(all["items"][0]["kind"], "quick");
        let mine = &all["items"][QUICK.len()];
        assert_eq!(mine["name"], "Big");
        assert_eq!(mine["created"], 10);
        assert_eq!(mine["filter"]["min_followers"], 5000);
        assert_eq!(listing(&c, "tiktok").unwrap()["saved"], 0);
        assert!(remove(&c, "x", "big").unwrap());
        assert!(!remove(&c, "x", "big").unwrap());
        for i in 0..MAX_SAVED {
            assert_eq!(
                save(&c, "x", &format!("f{i}"), &f, 1).unwrap(),
                Saved::Created
            );
        }
        assert_eq!(save(&c, "x", "one more", &f, 1).unwrap(), Saved::Full);
        assert_eq!(save(&c, "x", "f3", &g, 1).unwrap(), Saved::Replaced);
    }

    #[test]
    fn applying_a_filter_fills_the_arguments_and_explicit_ones_win() {
        let saved = clean(&json!({ "min_followers": 1000, "verified": true, "sort": "followers" }))
            .unwrap();
        let merged = merge(
            &json!({ "view": "followers", "filter": "big", "verified": false, "limit": 10 }),
            &saved,
        );
        assert_eq!(merged["min_followers"], 1000);
        assert_eq!(merged["verified"], false);
        assert_eq!(merged["sort"], "followers");
        assert_eq!(merged["limit"], 10);
        assert!(merged.get("filter").is_none());
        let q = Query::parse(&merged).unwrap();
        assert!(!q.filter.matches(&row(Member {
            followers: Some(5000),
            verified: true,
            ..Default::default()
        })));
        assert!(q.filter.matches(&row(Member {
            followers: Some(5000),
            ..Default::default()
        })));
        let (same, applied) = apply(Platform::X, &json!({ "view": "fans" })).unwrap();
        assert_eq!(same, json!({ "view": "fans" }));
        assert!(applied.is_null());
        let (args, applied) = apply(
            Platform::X,
            &json!({ "view": "fans", "filter": "verified" }),
        )
        .unwrap();
        assert_eq!(args["verified"], true);
        assert_eq!(applied["kind"], "quick");
    }
}
