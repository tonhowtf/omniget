use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS monitors (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    platform TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    query TEXT NOT NULL,
    options TEXT NOT NULL DEFAULT '{}',
    created INTEGER NOT NULL,
    last_checked INTEGER
);
CREATE TABLE IF NOT EXISTS monitor_hits (
    monitor_id INTEGER NOT NULL,
    post_id TEXT NOT NULL,
    seen_at INTEGER NOT NULL,
    posted_at INTEGER,
    post TEXT NOT NULL,
    read INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (monitor_id, post_id)
);
CREATE INDEX IF NOT EXISTS monitor_hits_unread ON monitor_hits (monitor_id, read);
CREATE TABLE IF NOT EXISTS monitor_days (
    monitor_id INTEGER NOT NULL,
    day TEXT NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (monitor_id, day)
);
";

pub const MENTIONS: &str = "mentions";
pub const SURGE_WINDOW_DAYS: i64 = 7;
pub const SURGE_MIN_HISTORY_DAYS: i64 = 3;
pub const SURGE_MIN_TODAY: i64 = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub id: i64,
    pub platform: String,
    pub name: String,
    pub kind: String,
    pub query: String,
    pub options: Value,
    pub created: i64,
    pub last_checked: Option<i64>,
    pub unread: i64,
    pub total: i64,
}

impl Monitor {
    pub fn json(&self) -> Value {
        json!({
            "id": self.id,
            "platform": self.platform,
            "name": self.name,
            "kind": self.kind,
            "query": self.query,
            "input": self.options,
            "builtin": self.kind == MENTIONS,
            "created": self.created,
            "last_checked": self.last_checked,
            "unread": self.unread,
            "total_stored": self.total,
        })
    }
}

const SELECT: &str =
    "SELECT m.id, m.platform, m.name, m.kind, m.query, m.options, m.created, m.last_checked,
    (SELECT COUNT(*) FROM monitor_hits h WHERE h.monitor_id = m.id AND h.read = 0),
    (SELECT COUNT(*) FROM monitor_hits h WHERE h.monitor_id = m.id)
    FROM monitors m";

fn monitor(r: &Row) -> rusqlite::Result<Monitor> {
    let options: String = r.get(5)?;
    Ok(Monitor {
        id: r.get(0)?,
        platform: r.get(1)?,
        name: r.get(2)?,
        kind: r.get(3)?,
        query: r.get(4)?,
        options: serde_json::from_str(&options).unwrap_or_else(|_| json!({})),
        created: r.get(6)?,
        last_checked: r.get(7)?,
        unread: r.get(8)?,
        total: r.get(9)?,
    })
}

pub fn list(conn: &Connection, platform: &str) -> rusqlite::Result<Vec<Monitor>> {
    let mut st = conn.prepare(&format!(
        "{SELECT} WHERE m.platform = ?1 ORDER BY (m.kind = '{MENTIONS}') DESC, m.created ASC, m.id ASC"
    ))?;
    let rows = st.query_map([platform], monitor)?;
    rows.collect()
}

pub fn get(conn: &Connection, platform: &str, id: i64) -> rusqlite::Result<Option<Monitor>> {
    conn.query_row(
        &format!("{SELECT} WHERE m.platform = ?1 AND m.id = ?2"),
        params![platform, id],
        monitor,
    )
    .optional()
}

pub fn ensure_mentions(conn: &Connection, platform: &str, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO monitors (platform, name, kind, query, options, created)
         SELECT ?1, 'Mentions', ?2, '', '{}', ?3
         WHERE NOT EXISTS (SELECT 1 FROM monitors WHERE platform = ?1 AND kind = ?2)",
        params![platform, MENTIONS, now],
    )?;
    Ok(())
}

pub fn create(
    conn: &Connection,
    platform: &str,
    name: &str,
    kind: &str,
    query: &str,
    options: &Value,
    now: i64,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO monitors (platform, name, kind, query, options, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![platform, name, kind, query, options.to_string(), now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update(
    conn: &Connection,
    id: i64,
    name: &str,
    kind: &str,
    query: &str,
    options: &Value,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE monitors SET name = ?2, kind = ?3, query = ?4, options = ?5 WHERE id = ?1",
        params![id, name, kind, query, options.to_string()],
    )?;
    Ok(())
}

pub fn remove(conn: &Connection, id: i64) -> rusqlite::Result<usize> {
    let hits = conn.execute("DELETE FROM monitor_hits WHERE monitor_id = ?1", [id])?;
    conn.execute("DELETE FROM monitor_days WHERE monitor_id = ?1", [id])?;
    conn.execute("DELETE FROM monitors WHERE id = ?1", [id])?;
    Ok(hits)
}

pub fn ingest(
    conn: &Connection,
    id: i64,
    posts: &[Value],
    now_ms: i64,
    day_of: &dyn Fn(i64) -> String,
) -> rusqlite::Result<Vec<Value>> {
    let mut fresh = Vec::new();
    for post in posts {
        let Some(post_id) = post["id"].as_str().filter(|x| !x.is_empty()) else {
            continue;
        };
        let posted = post["created_at"].as_i64();
        let added = conn.execute(
            "INSERT OR IGNORE INTO monitor_hits (monitor_id, post_id, seen_at, posted_at, post) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, post_id, now_ms, posted, post.to_string()],
        )?;
        if added == 0 {
            continue;
        }
        conn.execute(
            "INSERT INTO monitor_days (monitor_id, day, count) VALUES (?1, ?2, 1)
             ON CONFLICT (monitor_id, day) DO UPDATE SET count = count + 1",
            params![id, day_of(posted.unwrap_or(now_ms / 1000))],
        )?;
        fresh.push(post.clone());
    }
    Ok(fresh)
}

pub fn touch(conn: &Connection, id: i64, now_ms: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE monitors SET last_checked = ?2 WHERE id = ?1",
        params![id, now_ms],
    )?;
    Ok(())
}

pub fn hits(
    conn: &Connection,
    id: i64,
    unread_only: bool,
    limit: usize,
    offset: usize,
) -> rusqlite::Result<(Vec<Value>, i64)> {
    let filter = if unread_only { " AND read = 0" } else { "" };
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM monitor_hits WHERE monitor_id = ?1{filter}"),
        [id],
        |r| r.get(0),
    )?;
    let mut st = conn.prepare(&format!(
        "SELECT post, seen_at, read FROM monitor_hits WHERE monitor_id = ?1{filter}
         ORDER BY COALESCE(posted_at, seen_at / 1000) DESC, post_id DESC LIMIT ?2 OFFSET ?3"
    ))?;
    let rows = st.query_map(params![id, limit as i64, offset as i64], |r| {
        let post: String = r.get(0)?;
        let seen: i64 = r.get(1)?;
        let read: i64 = r.get(2)?;
        Ok(json!({
            "post": serde_json::from_str::<Value>(&post).unwrap_or(Value::Null),
            "seen_at": seen,
            "read": read != 0,
        }))
    })?;
    Ok((rows.collect::<rusqlite::Result<Vec<_>>>()?, total))
}

pub fn mark(
    conn: &Connection,
    id: i64,
    post_ids: Option<&[String]>,
    read: bool,
) -> rusqlite::Result<usize> {
    let flag = i64::from(read);
    match post_ids {
        None => conn.execute(
            "UPDATE monitor_hits SET read = ?2 WHERE monitor_id = ?1 AND read != ?2",
            params![id, flag],
        ),
        Some(ids) => {
            let mut changed = 0;
            for post_id in ids {
                changed += conn.execute(
                    "UPDATE monitor_hits SET read = ?3 WHERE monitor_id = ?1 AND post_id = ?2 AND read != ?3",
                    params![id, post_id, flag],
                )?;
            }
            Ok(changed)
        }
    }
}

pub fn days(conn: &Connection, id: i64, limit: usize) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut st = conn.prepare(
        "SELECT day, count FROM (SELECT day, count FROM monitor_days WHERE monitor_id = ?1 ORDER BY day DESC LIMIT ?2) ORDER BY day ASC",
    )?;
    let rows = st.query_map(params![id, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

pub fn surge(days: &[(String, i64)], today: &str, first_day: &str, multiple: f64) -> Value {
    let count = |day: &str| days.iter().find(|(d, _)| d == day).map_or(0, |(_, c)| *c);
    let today_count = count(today);
    let parse = |d: &str| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
    let history = match (parse(today), parse(first_day)) {
        (Some(t), Some(f)) => (t - f).num_days().clamp(0, SURGE_WINDOW_DAYS),
        _ => 0,
    };
    let mut base = json!({
        "surge": false,
        "today": today_count,
        "trailing_average": Value::Null,
        "days_used": history,
        "multiple": multiple,
        "min_today": SURGE_MIN_TODAY,
    });
    let Some(t) = parse(today).filter(|_| history >= SURGE_MIN_HISTORY_DAYS) else {
        base["reason"] = json!(format!(
            "Needs {SURGE_MIN_HISTORY_DAYS} full days of history since the monitor was created; it has {history}."
        ));
        return base;
    };
    let sum: i64 = (1..=history)
        .map(|back| {
            count(
                &(t - chrono::Duration::days(back))
                    .format("%Y-%m-%d")
                    .to_string(),
            )
        })
        .sum();
    let average = sum as f64 / history as f64;
    base["trailing_average"] = json!((average * 100.0).round() / 100.0);
    base["surge"] =
        json!(today_count >= SURGE_MIN_TODAY && today_count as f64 > multiple * average);
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::social::store;

    fn day(secs: i64) -> String {
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    }

    fn post(id: &str, ts: i64) -> Value {
        json!({ "id": id, "text": format!("post {id}"), "created_at": ts })
    }

    const NOV_10: i64 = 1_794_268_800;

    #[test]
    fn new_hits_are_stored_once_and_counted_by_the_day_they_were_posted() {
        let conn = store::memory(SCHEMA);
        let id = create(&conn, "x", "Rust", "search", "rust", &json!({}), 1).unwrap();
        let first = ingest(
            &conn,
            id,
            &[
                post("1", NOV_10),
                post("2", NOV_10 + 60),
                post("3", NOV_10 + 86_400),
            ],
            5,
            &day,
        )
        .unwrap();
        assert_eq!(first.len(), 3);
        let again = ingest(
            &conn,
            id,
            &[
                post("3", NOV_10 + 86_400),
                post("4", NOV_10 + 86_500),
                json!({ "text": "no id" }),
            ],
            6,
            &day,
        )
        .unwrap();
        assert_eq!(again, vec![post("4", NOV_10 + 86_500)]);
        assert_eq!(
            days(&conn, id, 30).unwrap(),
            vec![("2026-11-10".to_string(), 2), ("2026-11-11".to_string(), 2)]
        );
        let m = get(&conn, "x", id).unwrap().unwrap();
        assert_eq!((m.unread, m.total), (4, 4));
        assert_eq!(get(&conn, "instagram", id).unwrap(), None);
    }

    #[test]
    fn a_post_without_a_date_counts_on_the_day_it_was_seen() {
        let conn = store::memory(SCHEMA);
        let id = create(&conn, "tiktok", "Cats", "search", "cats", &json!({}), 1).unwrap();
        ingest(&conn, id, &[json!({ "id": "9" })], NOV_10 * 1000, &day).unwrap();
        assert_eq!(
            days(&conn, id, 30).unwrap(),
            vec![("2026-11-10".to_string(), 1)]
        );
    }

    #[test]
    fn hits_are_paged_newest_first_and_can_be_marked_read() {
        let conn = store::memory(SCHEMA);
        let id = create(&conn, "x", "Rust", "search", "rust", &json!({}), 1).unwrap();
        let posts: Vec<Value> = (1..=5).map(|i| post(&i.to_string(), NOV_10 + i)).collect();
        ingest(&conn, id, &posts, 5, &day).unwrap();
        let (page, total) = hits(&conn, id, false, 2, 1).unwrap();
        assert_eq!(total, 5);
        assert_eq!(
            page.iter()
                .map(|h| h["post"]["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["4", "3"]
        );
        assert_eq!(page[0]["read"], false);
        assert_eq!(
            mark(
                &conn,
                id,
                Some(&["4".to_string(), "nope".to_string()]),
                true
            )
            .unwrap(),
            1
        );
        assert_eq!(hits(&conn, id, true, 10, 0).unwrap().1, 4);
        assert_eq!(mark(&conn, id, None, true).unwrap(), 4);
        assert_eq!(get(&conn, "x", id).unwrap().unwrap().unread, 0);
        assert_eq!(mark(&conn, id, Some(&["4".to_string()]), false).unwrap(), 1);
        assert_eq!(list(&conn, "x").unwrap()[0].unread, 1);
    }

    #[test]
    fn the_mentions_inbox_is_created_once_and_listed_first() {
        let conn = store::memory(SCHEMA);
        let search = create(&conn, "x", "Rust", "search", "rust", &json!({}), 1).unwrap();
        ensure_mentions(&conn, "x", 50).unwrap();
        ensure_mentions(&conn, "x", 60).unwrap();
        let all = list(&conn, "x").unwrap();
        assert_eq!(
            all.iter().map(|m| m.kind.as_str()).collect::<Vec<_>>(),
            vec!["mentions", "search"]
        );
        assert_eq!(all[0].json()["builtin"], true);
        assert_eq!(all[1].id, search);
        assert!(list(&conn, "tiktok").unwrap().is_empty());
    }

    #[test]
    fn update_and_remove_change_only_that_monitor() {
        let conn = store::memory(SCHEMA);
        let a = create(&conn, "x", "A", "search", "a", &json!({}), 1).unwrap();
        let b = create(&conn, "x", "B", "search", "b", &json!({}), 2).unwrap();
        ingest(&conn, a, &[post("1", NOV_10), post("2", NOV_10)], 5, &day).unwrap();
        ingest(&conn, b, &[post("1", NOV_10)], 5, &day).unwrap();
        update(&conn, a, "A2", "search", "a2", &json!({ "query": "a2" })).unwrap();
        touch(&conn, a, 99).unwrap();
        let m = get(&conn, "x", a).unwrap().unwrap();
        assert_eq!(
            (m.name.as_str(), m.query.as_str(), m.last_checked, m.total),
            ("A2", "a2", Some(99), 2)
        );
        assert_eq!(m.options["query"], "a2");
        assert_eq!(remove(&conn, a).unwrap(), 2);
        assert_eq!(get(&conn, "x", a).unwrap(), None);
        assert!(days(&conn, a, 30).unwrap().is_empty());
        assert_eq!(get(&conn, "x", b).unwrap().unwrap().total, 1);
    }

    #[test]
    fn a_surge_is_today_above_a_multiple_of_the_trailing_average() {
        let series = |today: i64| -> Vec<(String, i64)> {
            vec![
                ("2026-11-07".into(), 2),
                ("2026-11-08".into(), 4),
                ("2026-11-10".into(), 3),
                ("2026-11-11".into(), today),
            ]
        };
        let v = surge(&series(10), "2026-11-11", "2026-11-07", 3.0);
        assert_eq!(v["trailing_average"], 2.25);
        assert_eq!(v["days_used"], 4);
        assert_eq!(v["today"], 10);
        assert_eq!(v["surge"], true);
        assert_eq!(
            surge(&series(6), "2026-11-11", "2026-11-07", 3.0)["surge"],
            false
        );
        assert_eq!(
            surge(&series(10), "2026-11-11", "2026-11-07", 5.0)["surge"],
            false
        );
        let long = surge(&series(10), "2026-11-11", "2026-10-01", 3.0);
        assert_eq!(long["days_used"], 7);
        assert_eq!(long["trailing_average"], 1.29);
    }

    #[test]
    fn no_surge_without_history_or_with_a_handful_of_posts() {
        let young = surge(
            &[("2026-11-11".into(), 50)],
            "2026-11-11",
            "2026-11-09",
            3.0,
        );
        assert_eq!(young["surge"], false);
        assert!(young["reason"].is_string());
        let quiet = surge(&[("2026-11-11".into(), 4)], "2026-11-11", "2026-11-01", 3.0);
        assert_eq!(quiet["surge"], false);
        assert_eq!(quiet["trailing_average"], 0.0);
        let first = surge(&[("2026-11-11".into(), 5)], "2026-11-11", "2026-11-01", 3.0);
        assert_eq!(first["surge"], true);
    }
}
