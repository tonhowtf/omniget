use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::super::model::{Platform, Post, Stats};
use super::stats::originals;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS insight_post_metrics (
  platform TEXT NOT NULL, post_id TEXT NOT NULL, at INTEGER NOT NULL, account TEXT NOT NULL,
  likes INTEGER, replies INTEGER, reposts INTEGER, saves INTEGER, views INTEGER,
  PRIMARY KEY (platform, post_id, at)) WITHOUT ROWID;
";

pub const NOTE: &str = "The series has a point only for the moments this app read the account's posts and a number had changed; nothing is recorded while the app is closed and nothing is back-filled.";

const COLS: &str = "likes, replies, reposts, saves, views";

fn n(v: Option<u64>) -> Option<i64> {
    v.map(|x| x.min(i64::MAX as u64) as i64)
}

fn last(conn: &Connection, platform: &str, id: &str) -> rusqlite::Result<Option<[Option<i64>; 5]>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM insight_post_metrics WHERE platform = ?1 AND post_id = ?2 ORDER BY at DESC LIMIT 1"),
        params![platform, id],
        |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?]),
    )
    .optional()
}

pub fn record(
    conn: &mut Connection,
    platform: &str,
    account: &str,
    posts: &[Post],
    at_ms: i64,
) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    let mut added = 0;
    for p in originals(posts) {
        let Stats {
            likes,
            replies,
            reposts,
            views,
            saves,
        } = p.stats.clone();
        let now = [n(likes), n(replies), n(reposts), n(saves), n(views)];
        if p.id.is_empty() || now.iter().all(Option::is_none) {
            continue;
        }
        if last(&tx, platform, &p.id)? == Some(now) {
            continue;
        }
        added += tx.execute(
            &format!("INSERT OR IGNORE INTO insight_post_metrics (platform, post_id, at, account, {COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"),
            params![platform, p.id, at_ms, account.to_lowercase(), now[0], now[1], now[2], now[3], now[4]],
        )?;
    }
    tx.commit()?;
    Ok(added)
}

pub fn series(conn: &Connection, platform: &str, id: &str) -> rusqlite::Result<Value> {
    let mut st = conn.prepare(&format!(
        "SELECT at, account, {COLS} FROM insight_post_metrics WHERE platform = ?1 AND post_id = ?2 ORDER BY at"
    ))?;
    let rows = st.query_map(params![platform, id], |r| {
        Ok((
            r.get::<_, String>(1)?,
            json!({
                "at": r.get::<_, i64>(0)?,
                "likes": r.get::<_, Option<i64>>(2)?,
                "replies": r.get::<_, Option<i64>>(3)?,
                "reposts": r.get::<_, Option<i64>>(4)?,
                "saves": r.get::<_, Option<i64>>(5)?,
                "views": r.get::<_, Option<i64>>(6)?,
            }),
        ))
    })?;
    let mut account = String::new();
    let mut points = Vec::new();
    for row in rows {
        let (who, point) = row?;
        account = who;
        points.push(point);
    }
    let delta = |key: &str| -> Option<i64> {
        Some(points.last()?[key].as_i64()? - points.first()?[key].as_i64()?)
    };
    let change = json!({ "likes": delta("likes"), "replies": delta("replies"), "reposts": delta("reposts"), "saves": delta("saves"), "views": delta("views") });
    let mut out = json!({
        "post_id": id,
        "account": account,
        "total": points.len(),
        "returned": points.len(),
        "truncated": false,
        "first_at": points.first().map(|p| p["at"].clone()),
        "last_at": points.last().map(|p| p["at"].clone()),
        "change": change,
        "points": points,
        "note": NOTE,
    });
    if out["total"] == 0 {
        out["empty_reason"] = json!("No point is stored for this post: it is not one of the signed-in account's posts read by the analytics tool so far.");
        out["next_step"] = json!("Run the analytics tool without a handle so the account's own posts are read, then ask again; the series grows each time the numbers change.");
    }
    Ok(out)
}

pub fn tracked(
    conn: &Connection,
    platform: &str,
    limit: usize,
    offset: usize,
) -> rusqlite::Result<Value> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT post_id) FROM insight_post_metrics WHERE platform = ?1",
        [platform],
        |r| r.get(0),
    )?;
    let mut st = conn.prepare(
        "SELECT post_id, COUNT(*), MIN(at), MAX(at) FROM insight_post_metrics WHERE platform = ?1
         GROUP BY post_id ORDER BY MAX(at) DESC, post_id DESC LIMIT ?2 OFFSET ?3",
    )?;
    let rows = st.query_map(params![platform, limit as i64, offset as i64], |r| {
        Ok(json!({
            "post_id": r.get::<_, String>(0)?,
            "points": r.get::<_, i64>(1)?,
            "first_at": r.get::<_, i64>(2)?,
            "last_at": r.get::<_, i64>(3)?,
        }))
    })?;
    let items: Vec<Value> = rows.collect::<rusqlite::Result<_>>()?;
    let mut out = json!({
        "total": total,
        "returned": items.len(),
        "offset": offset,
        "truncated": ((offset + items.len()) as i64) < total,
        "items": items,
        "note": NOTE,
    });
    if total == 0 {
        out["empty_reason"] = json!("No post of the signed-in account was read yet. Run the analytics tool without a handle first.");
    }
    Ok(out)
}

pub fn post_id(p: Platform, raw: &str) -> String {
    let raw = raw.trim();
    if p == Platform::ShortPost {
        if let Some(id) = super::super::shortpost::tweet_id(raw) {
            return id;
        }
    }
    raw.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(raw)
        .split(['?', '#'])
        .next()
        .unwrap_or(raw)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::super::super::store;
    use super::*;
    use crate::commands::social::model::Author;

    fn post(id: &str, likes: u64, views: Option<u64>) -> Post {
        Post {
            id: id.into(),
            stats: Stats {
                likes: Some(likes),
                views,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn each_look_stores_a_point_only_when_a_number_changed() {
        let mut c = store::memory(SCHEMA);
        let first = [post("1", 10, Some(100)), post("2", 5, None)];
        assert_eq!(record(&mut c, "x", "Ana", &first, 1_000).unwrap(), 2);
        assert_eq!(record(&mut c, "x", "ana", &first, 2_000).unwrap(), 0);
        let later = [post("1", 12, Some(180)), post("2", 5, None)];
        assert_eq!(record(&mut c, "x", "ana", &later, 3_000).unwrap(), 1);
        let s = series(&c, "x", "1").unwrap();
        assert_eq!(s["total"], 2);
        assert_eq!(s["account"], "ana");
        assert_eq!(s["first_at"], 1_000);
        assert_eq!(s["last_at"], 3_000);
        assert_eq!(s["points"][0]["likes"], 10);
        assert_eq!(s["points"][1]["views"], 180);
        assert_eq!(s["points"][1]["saves"], Value::Null);
        assert_eq!(s["change"]["likes"], 2);
        assert_eq!(s["change"]["views"], 80);
        assert_eq!(s["change"]["replies"], Value::Null);
        assert!(s["note"].as_str().unwrap().contains("only for the moments"));
        assert_eq!(series(&c, "x", "2").unwrap()["total"], 1);
        assert_eq!(series(&c, "instagram", "1").unwrap()["total"], 0);
    }

    #[test]
    fn reposts_and_posts_without_numbers_are_not_tracked() {
        let mut c = store::memory(SCHEMA);
        let mut repost = post("9", 99, None);
        repost.reposted_by = Some(Author::default());
        let bare = Post {
            id: "8".into(),
            ..Default::default()
        };
        assert_eq!(record(&mut c, "x", "ana", &[repost, bare], 1).unwrap(), 0);
        let none = series(&c, "x", "9").unwrap();
        assert!(none["empty_reason"].is_string() && none["next_step"].is_string());
        assert_eq!(none["first_at"], Value::Null);
        let list = tracked(&c, "x", 10, 0).unwrap();
        assert_eq!(list["total"], 0);
        assert!(list["empty_reason"].is_string());
    }

    #[test]
    fn tracked_posts_are_listed_newest_look_first_and_paged() {
        let mut c = store::memory(SCHEMA);
        record(
            &mut c,
            "x",
            "ana",
            &[post("1", 1, None), post("2", 1, None)],
            10,
        )
        .unwrap();
        record(&mut c, "x", "ana", &[post("1", 2, None)], 20).unwrap();
        record(&mut c, "tiktok", "ana", &[post("7", 2, None)], 30).unwrap();
        let all = tracked(&c, "x", 1, 0).unwrap();
        assert_eq!(all["total"], 2);
        assert_eq!(all["returned"], 1);
        assert_eq!(all["truncated"], true);
        assert_eq!(
            all["items"][0],
            json!({ "post_id": "1", "points": 2, "first_at": 10, "last_at": 20 })
        );
        let rest = tracked(&c, "x", 1, 1).unwrap();
        assert_eq!(rest["items"][0]["post_id"], "2");
        assert_eq!(rest["truncated"], false);
    }

    #[test]
    fn a_post_is_found_by_id_or_link() {
        assert_eq!(
            post_id(Platform::ShortPost, "https://x.com/ana/status/123?s=20"),
            "123"
        );
        assert_eq!(post_id(Platform::ShortPost, " 123 "), "123");
        assert_eq!(
            post_id(
                Platform::ClipFeed,
                "https://www.tiktok.com/@ana/video/7412?lang=en"
            ),
            "7412"
        );
        assert_eq!(post_id(Platform::PhotoFeed, "3141_22"), "3141_22");
    }
}
