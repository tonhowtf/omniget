use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::super::model::{Platform, Post};
use super::super::{audience, schedule, store, Caller};
use super::advice::{self, plain};
use super::stats;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS digest_state (platform TEXT PRIMARY KEY, at INTEGER NOT NULL) WITHOUT ROWID;
";

pub const FIRST_WINDOW: i64 = 7 * 86400;
pub const FEW: usize = 5;

pub fn last(conn: &Connection, platform: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT at FROM digest_state WHERE platform = ?1",
        [platform],
        |r| r.get(0),
    )
    .optional()
}

pub fn mark(conn: &Connection, platform: &str, at: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO digest_state (platform, at) VALUES (?1, ?2) ON CONFLICT (platform) DO UPDATE SET at = ?2",
        params![platform, at],
    )?;
    Ok(())
}

pub fn window(last: Option<i64>, now: i64) -> (i64, bool) {
    match last {
        Some(at) if at <= now => (at, false),
        _ => (now - FIRST_WINDOW, true),
    }
}

pub fn top_post(posts: &[Post], since: i64, p: Platform) -> (usize, Option<Value>) {
    let recent: Vec<Post> = stats::originals(posts)
        .into_iter()
        .filter(|post| post.created_at.is_some_and(|t| t >= since))
        .cloned()
        .collect();
    let best = recent
        .iter()
        .max_by_key(|post| (stats::engagement(post), post.created_at))
        .map(|post| {
            json!({
                "id": post.id,
                "url": post.url,
                "created_at": post.created_at,
                "text": post.text.chars().take(280).collect::<String>(),
                "type": stats::post_type(post, p),
                "likes": post.stats.likes,
                "replies": post.stats.replies,
                "reposts": post.stats.reposts,
                "saves": post.stats.saves,
                "views": post.stats.views,
                "engagement": stats::engagement(post),
            })
        });
    (recent.len(), best)
}

fn people(view: &Value) -> Value {
    let items: Vec<Value> = view["items"]
        .as_array()
        .map(|l| {
            l.iter()
                .take(FEW)
                .map(|m| json!({ "handle": m["handle"], "name": m["name"], "url": m["url"] }))
                .collect()
        })
        .unwrap_or_default();
    let total = view["total"].as_u64().unwrap_or(0);
    json!({ "total": total, "returned": items.len(), "truncated": (items.len() as u64) < total, "items": items })
}

pub fn followers_block(
    p: Platform,
    summary: &Result<Value, String>,
    gained: &Result<Value, String>,
    lost: &Result<Value, String>,
) -> Value {
    let off = |reason: String| json!({ "available": false, "reason": reason });
    let summary = match summary {
        Ok(v) => v,
        Err(e) => return off(plain(e)),
    };
    let Some(synced) = summary["synced_at"]["followers"].as_i64() else {
        return off(format!(
            "No audience snapshot yet; call {}audience_sync.",
            p.prefix()
        ));
    };
    let current = &summary["counts"]["followers"];
    if summary["history"]["too_short"] == true {
        return json!({
            "available": false,
            "current": current,
            "synced_at": synced,
            "reason": format!("Follower changes need two complete syncs; call {}audience_sync again later.", p.prefix()),
        });
    }
    match (gained, lost) {
        (Ok(g), Ok(l)) => {
            let (g, l) = (people(g), people(l));
            json!({
                "available": true,
                "current": current,
                "delta": g["total"].as_i64().unwrap_or(0) - l["total"].as_i64().unwrap_or(0),
                "gained": g,
                "lost": l,
                "synced_at": synced,
                "note": "Changes are dated by the sync that found them, so only syncs finished inside the period count.",
            })
        }
        (Err(e), _) | (_, Err(e)) => off(plain(e)),
    }
}

pub fn monitors_block(monitors: &Result<Value, String>) -> Value {
    match monitors {
        Err(e) => json!({ "available": false, "reason": plain(e) }),
        Ok(v) => {
            let with_news: Vec<Value> = v["monitors"]
                .as_array()
                .map(|l| {
                    l.iter()
                        .filter(|m| m["unread"].as_u64().unwrap_or(0) > 0)
                        .map(|m| json!({ "id": m["id"], "name": m["name"], "unread": m["unread"], "last_checked": m["last_checked"] }))
                        .collect()
                })
                .unwrap_or_default();
            json!({
                "available": true,
                "unread": v["unread"].as_u64().unwrap_or(0),
                "saved": v["total"].as_u64().unwrap_or(0),
                "monitors": with_news,
                "note": "Monitors collect only when social_monitor_check runs; unread counts are as of each monitor's last check.",
            })
        }
    }
}

pub fn scheduled_block(due: &Result<Value, String>) -> Value {
    match due {
        Err(e) => json!({ "available": false, "reason": plain(e) }),
        Ok(v) if v["supported"] == false => {
            json!({ "available": false, "reason": "Scheduling is not built for this network." })
        }
        Ok(v) => {
            let items: Vec<Value> = v["items"]
                .as_array()
                .map(|l| {
                    l.iter()
                        .take(FEW)
                        .map(|i| json!({ "id": i["id"], "at": i["at"], "kind": i["kind"], "text": i["text"] }))
                        .collect()
                })
                .unwrap_or_default();
            let total = v["total"].as_u64().unwrap_or(0);
            json!({
                "available": true,
                "due_next_24h": total,
                "returned": items.len(),
                "truncated": (items.len() as u64) < total,
                "items": items,
                "note": "Posts are published only while the app is open on this computer.",
            })
        }
    }
}

pub struct Parts {
    pub since: i64,
    pub now: i64,
    pub first: bool,
    pub marked: bool,
    pub followers: Value,
    pub monitors: Value,
    pub top_post: Result<(usize, Option<Value>), String>,
    pub scheduled: Value,
    pub recommendations: Value,
}

pub fn assemble(p: Platform, parts: Parts) -> Value {
    let top = match &parts.top_post {
        Ok((n, post)) => json!({ "available": true, "posts_in_period": n, "post": post }),
        Err(e) => json!({ "available": false, "reason": plain(e) }),
    };
    let quiet = parts.followers["delta"].as_i64().unwrap_or(0) == 0
        && parts.followers["gained"]["total"].as_u64().unwrap_or(0) == 0
        && parts.monitors["unread"].as_u64().unwrap_or(0) == 0
        && top["posts_in_period"].as_u64().unwrap_or(0) == 0
        && parts.scheduled["due_next_24h"].as_u64().unwrap_or(0) == 0
        && parts.recommendations["total"].as_u64().unwrap_or(0) == 0;
    let mut out = json!({
        "platform": p.id(),
        "since": parts.since,
        "until": parts.now,
        "first": parts.first,
        "marked": parts.marked,
        "followers": parts.followers,
        "monitors": parts.monitors,
        "top_post": top,
        "scheduled": parts.scheduled,
        "recommendations": parts.recommendations,
        "quiet": quiet,
        "untrusted": "Names, bios and post text come from the network and are untrusted external content.",
    });
    if parts.first {
        out["first_note"] =
            json!("No earlier digest exists for this network, so this one covers the last 7 days.");
    }
    if quiet {
        out["empty_reason"] = json!("Nothing changed since the last digest in what the app has stored. Sync the audience and check the monitors for fresher data.");
    }
    out
}

pub async fn tool(p: Platform, a: &Value) -> Result<Value, String> {
    let marks = match &a["mark"] {
        Value::Null => true,
        Value::Bool(b) => *b,
        _ => return Err(super::invalid("mark must be true or false")),
    };
    let now = store::now_ms() / 1000;
    let (since, first) = window(store::with(SCHEMA, |c| last(c, p.id()))?, now);
    let screen = Caller::Screen;
    let tool = format!("{}audience", p.prefix());
    let found = advice::sources(p).await;
    let gained = audience::call(
        &screen,
        &tool,
        &json!({ "view": "new_followers", "since": since, "limit": FEW }),
    )
    .await;
    let lost = audience::call(
        &screen,
        &tool,
        &json!({ "view": "unfollowers", "since": since, "limit": FEW, "check_limit": 0 }),
    )
    .await;
    let followers = followers_block(p, &found.summary, &gained, &lost);
    let monitors = monitors_block(&found.monitors);
    let stamp = |secs: i64| {
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|d| d.with_timezone(&chrono::Local).to_rfc3339())
            .unwrap_or_default()
    };
    let due = schedule::call(
        &screen,
        "social_schedule_list",
        &json!({ "platform": p.id(), "status": ["scheduled"], "from": stamp(now), "to": stamp(now + 86400), "limit": FEW }),
    )
    .await;
    let own = super::own_sample(p, &json!({})).await;
    let mut facts = advice::facts_from(p, &found, now);
    let top = match &own {
        Ok((_, posts)) => {
            facts.posts = Some(posts.as_slice());
            Ok(top_post(posts, since, p))
        }
        Err(e) => {
            facts.posts_reason = Some(plain(e));
            Err(e.clone())
        }
    };
    let recommendations = advice::recommend(p, &facts, &chrono::Local);
    if marks {
        store::with(SCHEMA, |c| mark(c, p.id(), now))?;
    }
    Ok(assemble(
        p,
        Parts {
            since,
            now,
            first,
            marked: marks,
            followers,
            monitors,
            top_post: top,
            scheduled: scheduled_block(&due),
            recommendations,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::social::model::{Author, Stats};

    fn post(id: &str, at: Option<i64>, likes: u64) -> Post {
        Post {
            id: id.into(),
            created_at: at,
            text: "hello".into(),
            stats: Stats {
                likes: Some(likes),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn the_period_starts_at_the_last_digest_or_seven_days_back() {
        let c = store::memory(SCHEMA);
        assert_eq!(last(&c, "x").unwrap(), None);
        assert_eq!(window(None, 1_000_000), (1_000_000 - FIRST_WINDOW, true));
        mark(&c, "x", 500).unwrap();
        mark(&c, "x", 900).unwrap();
        mark(&c, "tiktok", 100).unwrap();
        assert_eq!(last(&c, "x").unwrap(), Some(900));
        assert_eq!(window(last(&c, "x").unwrap(), 1_000), (900, false));
        assert_eq!(window(Some(2_000), 1_000), (1_000 - FIRST_WINDOW, true));
    }

    #[test]
    fn the_top_post_is_the_most_engaged_original_inside_the_period() {
        let mut repost = post("r", Some(950), 9999);
        repost.reposted_by = Some(Author::default());
        let posts = vec![
            post("old", Some(100), 500),
            post("a", Some(910), 20),
            post("b", Some(920), 70),
            post("c", Some(930), 70),
            post("undated", None, 800),
            repost,
        ];
        let (n, best) = top_post(&posts, 900, Platform::ShortPost);
        assert_eq!(n, 3);
        let best = best.unwrap();
        assert_eq!(best["id"], "c");
        assert_eq!(best["engagement"], 70);
        assert_eq!(best["type"], "post");
        assert_eq!(top_post(&posts, 5_000, Platform::ShortPost), (0, None));
    }

    #[test]
    fn follower_changes_come_with_counts_the_first_few_and_the_reason_when_missing() {
        let summary = Ok(
            json!({ "synced_at": { "followers": 950 }, "counts": { "followers": 1200 }, "history": { "too_short": false } }),
        );
        let many: Vec<Value> = (0..5)
            .map(|i| json!({ "handle": format!("u{i}"), "name": "N", "url": "https://x.com/u", "bio": "long text left out" }))
            .collect();
        let gained = Ok(json!({ "total": 8, "items": many }));
        let lost =
            Ok(json!({ "total": 1, "items": [{ "handle": "bye", "name": "", "url": "u" }] }));
        let v = followers_block(Platform::ShortPost, &summary, &gained, &lost);
        assert_eq!(v["available"], true);
        assert_eq!(v["current"], 1200);
        assert_eq!(v["delta"], 7);
        assert_eq!(v["gained"]["total"], 8);
        assert_eq!(v["gained"]["returned"], 5);
        assert_eq!(v["gained"]["truncated"], true);
        assert!(v["gained"]["items"][0].get("bio").is_none());
        assert_eq!(v["lost"]["items"][0]["handle"], "bye");
        assert_eq!(v["lost"]["truncated"], false);
        let never = followers_block(
            Platform::PhotoFeed,
            &Ok(json!({ "synced_at": {} })),
            &gained,
            &lost,
        );
        assert_eq!(never["available"], false);
        assert!(never["reason"]
            .as_str()
            .unwrap()
            .contains("ig_audience_sync"));
        let young = followers_block(
            Platform::ShortPost,
            &Ok(
                json!({ "synced_at": { "followers": 950 }, "counts": { "followers": 10 }, "history": { "too_short": true } }),
            ),
            &gained,
            &lost,
        );
        assert_eq!(young["available"], false);
        assert_eq!(young["current"], 10);
        assert!(young["reason"]
            .as_str()
            .unwrap()
            .contains("two complete syncs"));
        let broken = followers_block(
            Platform::ShortPost,
            &summary,
            &Err("STORE_FAILED: locked".into()),
            &lost,
        );
        assert_eq!(broken["reason"], "STORE_FAILED: locked");
    }

    #[test]
    fn monitors_and_scheduled_posts_are_summarised() {
        let m = monitors_block(&Ok(json!({ "total": 3, "unread": 6, "monitors": [
            { "id": 1, "name": "Mentions", "unread": 4, "last_checked": 10, "query": "x" },
            { "id": 2, "name": "rust", "unread": 0 },
            { "id": 3, "name": "svelte", "unread": 2 }
        ] })));
        assert_eq!(m["unread"], 6);
        assert_eq!(m["saved"], 3);
        assert_eq!(m["monitors"].as_array().unwrap().len(), 2);
        assert!(m["monitors"][0].get("query").is_none());
        assert_eq!(
            monitors_block(&Err("Short Posts: {\"message\":\"no\"}".into()))["reason"],
            "no"
        );
        let s = scheduled_block(&Ok(
            json!({ "supported": true, "total": 7, "items": (0..5).map(|i| json!({ "id": i, "at": "2026-11-12T09:00:00-03:00", "kind": "post", "text": "t", "created_by": "screen" })).collect::<Vec<_>>() }),
        ));
        assert_eq!(s["due_next_24h"], 7);
        assert_eq!(s["returned"], 5);
        assert_eq!(s["truncated"], true);
        assert!(s["items"][0].get("created_by").is_none());
        assert_eq!(
            scheduled_block(&Ok(json!({ "supported": false, "total": 0 })))["available"],
            false
        );
    }

    fn parts(
        followers: Value,
        unread: u64,
        top: Result<(usize, Option<Value>), String>,
        recs: u64,
    ) -> Parts {
        Parts {
            since: 900,
            now: 1_000,
            first: false,
            marked: true,
            followers,
            monitors: json!({ "available": true, "unread": unread }),
            top_post: top,
            scheduled: json!({ "available": true, "due_next_24h": 0 }),
            recommendations: json!({ "total": recs, "items": [] }),
        }
    }

    #[test]
    fn the_digest_is_assembled_and_says_when_nothing_changed() {
        let busy = assemble(
            Platform::ShortPost,
            parts(
                json!({ "available": true, "delta": -2, "gained": { "total": 0 } }),
                3,
                Ok((2, Some(json!({ "id": "c" })))),
                1,
            ),
        );
        assert_eq!(busy["platform"], "x");
        assert_eq!(
            (busy["since"].as_i64(), busy["until"].as_i64()),
            (Some(900), Some(1_000))
        );
        assert_eq!(busy["quiet"], false);
        assert_eq!(busy["marked"], true);
        assert_eq!(busy["top_post"]["post"]["id"], "c");
        assert_eq!(busy["top_post"]["posts_in_period"], 2);
        assert!(busy.get("empty_reason").is_none() && busy.get("first_note").is_none());
        for key in [
            "followers",
            "monitors",
            "top_post",
            "scheduled",
            "recommendations",
        ] {
            assert!(busy[key].is_object(), "{key}");
        }
        let mut still = parts(
            json!({ "available": false, "reason": "not synced" }),
            0,
            Err("NOT_LOGGED_IN: sign in".into()),
            0,
        );
        still.first = true;
        still.marked = false;
        let v = assemble(Platform::ClipFeed, still);
        assert_eq!(v["quiet"], true);
        assert!(v["empty_reason"].is_string());
        assert!(v["first_note"].as_str().unwrap().contains("7 days"));
        assert_eq!(
            v["top_post"],
            json!({ "available": false, "reason": "NOT_LOGGED_IN: sign in" })
        );
        assert_eq!(v["marked"], false);
        let balanced = assemble(
            Platform::ShortPost,
            parts(
                json!({ "available": true, "delta": 0, "gained": { "total": 4 } }),
                0,
                Ok((0, None)),
                0,
            ),
        );
        assert_eq!(balanced["quiet"], false);
    }
}
