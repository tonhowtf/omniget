use std::collections::BTreeMap;

use chrono::{Datelike, TimeZone, Timelike};
use serde_json::{json, Value};

use super::super::model::{Platform, Post};
use super::stats::{engagement, media_type, median, originals, THIN_CELL_BELOW};

pub const MIN_POSTS: usize = 10;
pub const MIN_GROUP: usize = 3;
pub const MEDIA_LIFT: f64 = 1.25;
pub const WINDOW_DAYS: i64 = 14;
pub const MIN_PREVIOUS: usize = 4;
pub const DROP_TO: f64 = 0.6;

const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const WEEKDAY_NAMES: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

#[derive(Default)]
pub struct Facts<'a> {
    pub posts: Option<&'a [Post]>,
    pub posts_reason: Option<String>,
    pub now: i64,
    pub not_following_back: Option<u64>,
    pub new_unfollowers: Option<u64>,
    pub last_sync: Option<i64>,
    pub audience_reason: Option<String>,
    pub unread_hits: Option<u64>,
    pub monitor_reason: Option<String>,
    pub drafts: Option<u64>,
    pub missed: Option<u64>,
    pub schedule_reason: Option<String>,
}

type Rule = Result<Value, String>;

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn item(id: &str, message: String, numbers: Value, tool: String, args: Value) -> Value {
    json!({ "id": id, "message": message, "numbers": numbers, "act_with": { "tool": tool, "args": args } })
}

pub fn best_time<Tz: TimeZone>(p: Platform, posts: &[Post], tz: &Tz) -> Rule {
    let mut cells: BTreeMap<(u32, u32), Vec<u64>> = BTreeMap::new();
    let mut all: Vec<u64> = Vec::new();
    for post in originals(posts) {
        if let Some(d) = post
            .created_at
            .and_then(|ts| tz.timestamp_opt(ts, 0).single())
        {
            let e = engagement(post);
            all.push(e);
            cells
                .entry((d.weekday().num_days_from_monday(), d.hour()))
                .or_default()
                .push(e);
        }
    }
    if all.len() < MIN_POSTS {
        return Err(format!(
            "needs {MIN_POSTS} dated posts, the sample has {}",
            all.len()
        ));
    }
    let overall = median(&all).unwrap_or_default();
    let best = cells
        .iter()
        .filter(|(_, v)| v.len() as u64 >= THIN_CELL_BELOW)
        .filter_map(|(k, v)| median(v).map(|m| (*k, v.len(), m)))
        .max_by(|a, b| a.2.total_cmp(&b.2).then_with(|| b.0.cmp(&a.0)));
    let Some(((weekday, hour), n, cell)) = best else {
        return Err(format!(
            "no weekday and hour has {THIN_CELL_BELOW} posts yet among {} posts",
            all.len()
        ));
    };
    if cell <= overall {
        return Err(format!(
            "the best weekday and hour (median {cell}) is not above the overall median ({overall})"
        ));
    }
    let message = format!(
        "{} around {hour:02}:00 is your strongest time: median engagement {cell} over {n} posts, against {overall} overall.",
        WEEKDAY_NAMES[weekday as usize]
    );
    let numbers = json!({ "weekday": weekday, "hour": hour, "posts_in_cell": n, "cell_median": cell, "overall_median": overall, "posts": all.len() });
    Ok(if p == Platform::ShortPost {
        let mut v = item(
            "best_time",
            message,
            numbers,
            "social_schedule".into(),
            json!({ "platform": "x", "op": "set_slots", "slots": [{ "weekday": WEEKDAYS[weekday as usize], "time": format!("{hour:02}:00") }] }),
        );
        v["act_with"]["note"] =
            json!("set_slots replaces the whole list of posting slots; read social_schedule_list first and send the full list.");
        v
    } else {
        item(
            "best_time",
            message,
            numbers,
            format!("{}analytics", p.prefix()),
            json!({ "report": "times" }),
        )
    })
}

pub fn media_winner(p: Platform, posts: &[Post]) -> Rule {
    let own = originals(posts);
    if own.len() < MIN_POSTS {
        return Err(format!(
            "needs {MIN_POSTS} posts, the sample has {}",
            own.len()
        ));
    }
    let mut groups: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
    for post in &own {
        groups
            .entry(media_type(post))
            .or_default()
            .push(engagement(post));
    }
    let solid: Vec<(&str, &Vec<u64>)> = groups
        .iter()
        .filter(|(_, v)| v.len() >= MIN_GROUP)
        .map(|(k, v)| (*k, v))
        .collect();
    if solid.len() < 2 {
        return Err(format!(
            "needs two media types with {MIN_GROUP} posts each to compare"
        ));
    }
    let mut best: Option<(&str, usize, f64, f64, usize)> = None;
    for (kind, values) in &solid {
        let rest: Vec<u64> = groups
            .iter()
            .filter(|(k, _)| *k != kind)
            .flat_map(|(_, v)| v.iter().copied())
            .collect();
        let (Some(m), Some(others)) = (median(values), median(&rest)) else {
            continue;
        };
        if m > 0.0 && m >= others * MEDIA_LIFT && best.is_none_or(|b| m > b.2) {
            best = Some((kind, values.len(), m, others, rest.len()));
        }
    }
    let Some((kind, n, m, others, rest)) = best else {
        return Err(format!(
            "no media type has a median engagement {MEDIA_LIFT} times the others"
        ));
    };
    Ok(item(
        "media_type",
        format!("Your {kind} posts do better: median engagement {m} over {n} posts, against {others} for the other {rest}."),
        json!({ "media": kind, "posts": n, "median": m, "others_median": others, "others_posts": rest, "lift": (others > 0.0).then(|| round2(m / others)) }),
        format!("{}analytics", p.prefix()),
        json!({ "report": "media" }),
    ))
}

pub fn frequency_drop(p: Platform, posts: &[Post], now: i64) -> Rule {
    let window = WINDOW_DAYS * 86400;
    let dates: Vec<i64> = originals(posts)
        .iter()
        .filter_map(|post| post.created_at)
        .collect();
    let oldest = dates.iter().min().copied();
    if oldest.is_none_or(|o| o > now - 2 * window) {
        return Err(format!(
            "the sample does not reach {} days back",
            2 * WINDOW_DAYS
        ));
    }
    let recent = dates
        .iter()
        .filter(|t| **t >= now - window && **t <= now)
        .count();
    let previous = dates
        .iter()
        .filter(|t| **t >= now - 2 * window && **t < now - window)
        .count();
    if previous < MIN_PREVIOUS {
        return Err(format!(
            "needs {MIN_PREVIOUS} posts in the previous {WINDOW_DAYS} days, found {previous}"
        ));
    }
    if recent as f64 > previous as f64 * DROP_TO {
        return Err(format!(
            "{recent} posts in the last {WINDOW_DAYS} days against {previous} before is not a drop"
        ));
    }
    let drop = round2((previous - recent) as f64 / previous as f64 * 100.0);
    let (tool, args) = if p == Platform::ShortPost {
        (
            "social_schedule".to_string(),
            json!({ "platform": "x", "op": "add", "slot": "next" }),
        )
    } else {
        (
            format!("{}analytics", p.prefix()),
            json!({ "report": "trend", "unit": "week" }),
        )
    };
    Ok(item(
        "frequency_drop",
        format!("You posted {recent} times in the last {WINDOW_DAYS} days, down from {previous} in the {WINDOW_DAYS} days before ({drop}% fewer)."),
        json!({ "recent": recent, "previous": previous, "window_days": WINDOW_DAYS, "drop_pct": drop }),
        tool,
        args,
    ))
}

fn counted(
    id: &str,
    n: Option<u64>,
    missing: &Option<String>,
    build: impl FnOnce(u64) -> Value,
) -> Rule {
    match n {
        Some(0) => Err("the count is 0".into()),
        Some(n) => {
            let mut v = build(n);
            v["id"] = json!(id);
            Ok(v)
        }
        None => Err(missing
            .clone()
            .unwrap_or_else(|| "the number could not be read".into())),
    }
}

pub fn recommend<Tz: TimeZone>(p: Platform, f: &Facts, tz: &Tz) -> Value {
    let prefix = p.prefix();
    let no_posts = || {
        Err(f
            .posts_reason
            .clone()
            .unwrap_or_else(|| "the account's posts could not be read".into()))
    };
    let rules: Vec<(&str, Rule)> = vec![
        (
            "missed_posts",
            counted("missed_posts", f.missed, &f.schedule_reason, |n| {
                item(
                    "",
                    format!(
                        "{n} scheduled post{} came due while the app was closed and {} not sent.",
                        if n == 1 { "" } else { "s" },
                        if n == 1 { "was" } else { "were" }
                    ),
                    json!({ "missed": n }),
                    "social_schedule_list".into(),
                    json!({ "platform": p.id(), "status": ["missed"] }),
                )
            }),
        ),
        (
            "new_unfollowers",
            counted(
                "new_unfollowers",
                f.new_unfollowers,
                &f.audience_reason,
                |n| {
                    item(
                        "",
                        format!(
                            "{n} account{} stopped following you between the last two syncs.",
                            if n == 1 { "" } else { "s" }
                        ),
                        json!({ "unfollowers": n, "since": f.last_sync }),
                        format!("{prefix}audience"),
                        json!({ "view": "unfollowers", "since": f.last_sync }),
                    )
                },
            ),
        ),
        (
            "unread_hits",
            counted("unread_hits", f.unread_hits, &f.monitor_reason, |n| {
                item(
                    "",
                    format!(
                        "{n} unread post{} in your monitors.",
                        if n == 1 { "" } else { "s" }
                    ),
                    json!({ "unread": n }),
                    "social_monitors".into(),
                    json!({ "platform": p.id() }),
                )
            }),
        ),
        (
            "best_time",
            f.posts
                .map_or_else(no_posts, |posts| best_time(p, posts, tz)),
        ),
        (
            "media_type",
            f.posts
                .map_or_else(no_posts, |posts| media_winner(p, posts)),
        ),
        (
            "frequency_drop",
            f.posts
                .map_or_else(no_posts, |posts| frequency_drop(p, posts, f.now)),
        ),
        (
            "drafts",
            counted("drafts", f.drafts, &f.schedule_reason, |n| {
                item(
                    "",
                    format!(
                        "{n} draft{} waiting for a time.",
                        if n == 1 { " is" } else { "s are" }
                    ),
                    json!({ "drafts": n }),
                    "social_schedule_list".into(),
                    json!({ "platform": p.id(), "status": ["draft"] }),
                )
            }),
        ),
        (
            "not_following_back",
            counted(
                "not_following_back",
                f.not_following_back,
                &f.audience_reason,
                |n| {
                    let mut v = item("", format!("{n} account{} you follow do{} not follow back and {} not on your keep list.", if n == 1 { "" } else { "s" }, if n == 1 { "es" } else { "" }, if n == 1 { "is" } else { "are" }),
                    json!({ "accounts": n }), format!("{prefix}audience"), json!({ "view": "not_following_back", "kept": false }));
                    v["act_with"]["note"] = json!(format!("Unfollowing is one approved {prefix}follow call with undo=true per account; there is no bulk unfollow."));
                    v
                },
            ),
        ),
    ];
    let mut items = Vec::new();
    let mut quiet = Vec::new();
    for (id, rule) in rules {
        match rule {
            Ok(v) => items.push(v),
            Err(reason) => quiet.push(json!({ "id": id, "reason": reason })),
        }
    }
    let mut out = json!({
        "total": items.len(),
        "returned": items.len(),
        "truncated": false,
        "items": items,
        "not_triggered": quiet,
        "rules": "Plain rules over numbers the app already holds; each item carries the numbers that triggered it. Nothing is predicted or estimated.",
    });
    if out["total"] == 0 {
        out["empty_reason"] = json!("No rule triggered: the data is too thin or nothing needs attention. not_triggered says why for each rule.");
    }
    out
}

pub fn plain(error: &str) -> String {
    error
        .split_once(": ")
        .and_then(|(_, body)| serde_json::from_str::<Value>(body).ok())
        .and_then(|v| v["message"].as_str().map(String::from))
        .unwrap_or_else(|| error.to_string())
}

pub struct Sources {
    pub summary: Result<Value, String>,
    pub not_following_back: Result<Value, String>,
    pub unfollowers: Result<Value, String>,
    pub monitors: Result<Value, String>,
    pub schedule: Result<Value, String>,
}

pub fn facts_from<'a>(p: Platform, s: &Sources, now: i64) -> Facts<'a> {
    let mut f = Facts {
        now,
        ..Default::default()
    };
    match &s.summary {
        Err(e) => f.audience_reason = Some(plain(e)),
        Ok(summary) => {
            f.last_sync = summary["synced_at"]["followers"].as_i64();
            let both = f.last_sync.is_some() && summary["synced_at"]["following"].is_i64();
            if f.last_sync.is_none() {
                f.audience_reason = Some(format!(
                    "no audience snapshot yet; call {}audience_sync",
                    p.prefix()
                ));
            } else {
                match (&s.not_following_back, both) {
                    (Ok(v), true) => f.not_following_back = v["total"].as_u64(),
                    (Err(e), _) => f.audience_reason = Some(plain(e)),
                    _ => {}
                }
                if summary["history"]["too_short"] == true {
                    f.audience_reason = Some(format!(
                        "follower changes need two complete syncs; call {}audience_sync again later",
                        p.prefix()
                    ));
                } else {
                    match &s.unfollowers {
                        Ok(v) => f.new_unfollowers = v["total"].as_u64(),
                        Err(e) => f.audience_reason = Some(plain(e)),
                    }
                }
            }
        }
    }
    match &s.monitors {
        Ok(v) if v["total"].as_u64().unwrap_or(0) > 0 => f.unread_hits = v["unread"].as_u64(),
        Ok(_) => f.monitor_reason = Some("no monitor is saved for this network".into()),
        Err(e) => f.monitor_reason = Some(plain(e)),
    }
    match &s.schedule {
        Ok(v) if v["supported"] == false => {
            f.schedule_reason = Some(format!("scheduling is not built for {}", p.label()))
        }
        Ok(v) => {
            f.drafts = Some(v["counts"]["draft"].as_u64().unwrap_or(0));
            f.missed = Some(v["counts"]["missed"].as_u64().unwrap_or(0));
        }
        Err(e) => f.schedule_reason = Some(plain(e)),
    }
    f
}

pub async fn sources(p: Platform) -> Sources {
    use super::super::{audience, monitor, schedule, Caller};
    let screen = Caller::Screen;
    let tool = format!("{}audience", p.prefix());
    let summary = audience::call(&screen, &tool, &json!({ "view": "summary" })).await;
    let since = summary
        .as_ref()
        .ok()
        .and_then(|v| v["synced_at"]["followers"].as_i64());
    Sources {
        not_following_back: audience::call(
            &screen,
            &tool,
            &json!({ "view": "not_following_back", "kept": false, "limit": 1 }),
        )
        .await,
        unfollowers: audience::call(
            &screen,
            &tool,
            &json!({ "view": "unfollowers", "since": since, "limit": 1, "check_limit": 0 }),
        )
        .await,
        summary,
        monitors: monitor::call(&screen, "social_monitors", &json!({ "platform": p.id() })).await,
        schedule: schedule::call(
            &screen,
            "social_schedule_list",
            &json!({ "platform": p.id(), "limit": 1 }),
        )
        .await,
    }
}

pub async fn gather(p: Platform, posts: Result<&[Post], String>) -> Value {
    let found = sources(p).await;
    let mut facts = facts_from(p, &found, super::super::store::now_ms() / 1000);
    match posts {
        Ok(list) => facts.posts = Some(list),
        Err(e) => facts.posts_reason = Some(plain(&e)),
    }
    recommend(p, &facts, &chrono::Local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::social::model::{Author, Media, MediaKind, Stats};
    use chrono::Utc;

    const NOW: i64 = 1_794_000_000;
    const DAY: i64 = 86400;
    const MONDAY_9: i64 = 1_793_610_000;

    fn post(at: i64, likes: u64, image: bool) -> Post {
        Post {
            id: format!("{at}-{likes}"),
            created_at: Some(at),
            media: if image {
                vec![Media {
                    kind: MediaKind::Image,
                    url: "u".into(),
                    thumb: None,
                    width: None,
                    height: None,
                }]
            } else {
                Vec::new()
            },
            stats: Stats {
                likes: Some(likes),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn reason(r: Rule) -> String {
        r.unwrap_err()
    }

    #[test]
    fn the_best_time_needs_enough_posts_in_the_cell_and_must_beat_the_overall_median() {
        assert_eq!(MONDAY_9 % DAY, 9 * 3600);
        let mut posts: Vec<Post> = (0..3)
            .map(|w| post(MONDAY_9 - w * 7 * DAY, 100 + w as u64, false))
            .collect();
        posts.extend((0..8).map(|i| post(MONDAY_9 + DAY + i * 3600 * 5 + i * DAY, 10, false)));
        let v = best_time(Platform::ShortPost, &posts, &Utc).unwrap();
        assert_eq!(v["id"], "best_time");
        assert_eq!(
            v["numbers"],
            json!({ "weekday": 0, "hour": 9, "posts_in_cell": 3, "cell_median": 101.0, "overall_median": 10.0, "posts": 11 })
        );
        assert!(v["message"]
            .as_str()
            .unwrap()
            .starts_with("Monday around 09:00"));
        assert_eq!(v["act_with"]["tool"], "social_schedule");
        assert_eq!(
            v["act_with"]["args"]["slots"][0],
            json!({ "weekday": "mon", "time": "09:00" })
        );
        assert!(v["act_with"]["note"].is_string());
        let ig = best_time(Platform::PhotoFeed, &posts, &Utc).unwrap();
        assert_eq!(
            ig["act_with"],
            json!({ "tool": "ig_analytics", "args": { "report": "times" } })
        );
        assert!(reason(best_time(Platform::ShortPost, &posts[..9], &Utc))
            .contains("needs 10 dated posts, the sample has 9"));
        let spread: Vec<Post> = (0..12)
            .map(|i| post(MONDAY_9 + i * DAY + i * 3600, 50, false))
            .collect();
        assert!(reason(best_time(Platform::ShortPost, &spread, &Utc))
            .contains("no weekday and hour has 3 posts"));
        let flat: Vec<Post> = (0..12)
            .map(|i| post(MONDAY_9 - (i % 4) * 7 * DAY, 20, false))
            .collect();
        assert!(reason(best_time(Platform::ShortPost, &flat, &Utc))
            .contains("not above the overall median"));
        let mut repost = post(MONDAY_9, 9999, false);
        repost.reposted_by = Some(Author::default());
        let undated = Post {
            created_at: None,
            ..post(0, 9999, false)
        };
        let mut noisy = posts[..9].to_vec();
        noisy.extend([repost, undated]);
        assert!(best_time(Platform::ShortPost, &noisy, &Utc).is_err());
    }

    #[test]
    fn a_media_type_wins_only_with_enough_posts_on_both_sides_and_a_clear_margin() {
        let mut posts: Vec<Post> = (0..4).map(|i| post(i, 40 + i as u64, true)).collect();
        posts.extend((0..6).map(|i| post(100 + i, 10, false)));
        let v = media_winner(Platform::ClipFeed, &posts).unwrap();
        assert_eq!(
            v["numbers"],
            json!({ "media": "image", "posts": 4, "median": 41.5, "others_median": 10.0, "others_posts": 6, "lift": 4.15 })
        );
        assert_eq!(
            v["act_with"],
            json!({ "tool": "tt_analytics", "args": { "report": "media" } })
        );
        assert!(reason(media_winner(Platform::ShortPost, &posts[..9])).contains("needs 10 posts"));
        let lonely: Vec<Post> = (0..2)
            .map(|i| post(i, 500, true))
            .chain((0..9).map(|i| post(100 + i, 10, false)))
            .collect();
        assert!(reason(media_winner(Platform::ShortPost, &lonely)).contains("two media types"));
        let close: Vec<Post> = (0..5)
            .map(|i| post(i, 12, true))
            .chain((0..5).map(|i| post(100 + i, 10, false)))
            .collect();
        assert!(reason(media_winner(Platform::ShortPost, &close)).contains("1.25 times"));
        let zero: Vec<Post> = (0..5)
            .map(|i| post(i, 0, true))
            .chain((0..5).map(|i| post(100 + i, 0, false)))
            .collect();
        assert!(media_winner(Platform::ShortPost, &zero).is_err());
    }

    #[test]
    fn a_frequency_drop_needs_coverage_and_a_real_previous_period() {
        let days_ago = |d: i64| NOW - d * DAY;
        let mut posts: Vec<Post> = [15, 17, 19, 21, 23, 40]
            .iter()
            .map(|d| post(days_ago(*d), 1, false))
            .collect();
        posts.extend([2, 5, 9].iter().map(|d| post(days_ago(*d), 1, false)));
        let v = frequency_drop(Platform::ShortPost, &posts, NOW).unwrap();
        assert_eq!(
            v["numbers"],
            json!({ "recent": 3, "previous": 5, "window_days": 14, "drop_pct": 40.0 })
        );
        assert_eq!(v["act_with"]["tool"], "social_schedule");
        assert_eq!(
            frequency_drop(Platform::PhotoFeed, &posts, NOW).unwrap()["act_with"]["tool"],
            "ig_analytics"
        );
        posts.push(post(days_ago(1), 1, false));
        assert!(reason(frequency_drop(Platform::ShortPost, &posts, NOW)).contains("is not a drop"));
        let short: Vec<Post> = [1, 2, 20, 21, 22, 23]
            .iter()
            .map(|d| post(days_ago(*d), 1, false))
            .collect();
        assert!(reason(frequency_drop(Platform::ShortPost, &short, NOW))
            .contains("does not reach 28 days back"));
        let sparse: Vec<Post> = [1, 20, 21, 60]
            .iter()
            .map(|d| post(days_ago(*d), 1, false))
            .collect();
        assert!(reason(frequency_drop(Platform::ShortPost, &sparse, NOW))
            .contains("needs 4 posts in the previous 14 days, found 2"));
        assert!(frequency_drop(Platform::ShortPost, &[], NOW).is_err());
    }

    #[test]
    fn counts_from_the_other_engines_trigger_with_their_numbers_and_tools() {
        let f = Facts {
            now: NOW,
            not_following_back: Some(12),
            new_unfollowers: Some(1),
            last_sync: Some(777),
            unread_hits: Some(5),
            drafts: Some(2),
            missed: Some(1),
            posts_reason: Some("NOT_LOGGED_IN: sign in".into()),
            ..Default::default()
        };
        let v = recommend(Platform::ShortPost, &f, &Utc);
        assert_eq!(v["total"], 5);
        let ids: Vec<&str> = v["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|i| i["id"].as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "missed_posts",
                "new_unfollowers",
                "unread_hits",
                "drafts",
                "not_following_back"
            ]
        );
        let by = |id: &str| {
            v["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["id"] == id)
                .unwrap()
                .clone()
        };
        assert_eq!(
            by("not_following_back")["numbers"],
            json!({ "accounts": 12 })
        );
        assert_eq!(
            by("not_following_back")["act_with"]["args"],
            json!({ "view": "not_following_back", "kept": false })
        );
        assert!(by("not_following_back")["act_with"]["note"]
            .as_str()
            .unwrap()
            .contains("x_follow"));
        assert_eq!(
            by("new_unfollowers")["act_with"],
            json!({ "tool": "x_audience", "args": { "view": "unfollowers", "since": 777 } })
        );
        assert!(by("new_unfollowers")["message"]
            .as_str()
            .unwrap()
            .starts_with("1 account stopped"));
        assert_eq!(
            by("unread_hits")["act_with"],
            json!({ "tool": "social_monitors", "args": { "platform": "x" } })
        );
        assert_eq!(by("drafts")["act_with"]["args"]["status"], json!(["draft"]));
        assert_eq!(by("missed_posts")["numbers"], json!({ "missed": 1 }));
        for i in v["items"].as_array().unwrap() {
            assert!(
                i["numbers"].as_object().is_some_and(|n| !n.is_empty()),
                "{i}"
            );
            assert!(
                i["message"]
                    .as_str()
                    .unwrap()
                    .chars()
                    .any(|c| c.is_ascii_digit()),
                "{i}"
            );
        }
        let quiet: Vec<&str> = v["not_triggered"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|i| i["id"].as_str())
            .collect();
        assert_eq!(quiet, ["best_time", "media_type", "frequency_drop"]);
        assert_eq!(v["not_triggered"][0]["reason"], "NOT_LOGGED_IN: sign in");
    }

    #[test]
    fn facts_are_read_from_what_the_other_engines_answer() {
        let ready = Sources {
            summary: Ok(
                json!({ "synced_at": { "followers": 900, "following": 880 }, "history": { "too_short": false } }),
            ),
            not_following_back: Ok(json!({ "total": 12 })),
            unfollowers: Ok(json!({ "total": 3 })),
            monitors: Ok(json!({ "total": 2, "unread": 7 })),
            schedule: Ok(json!({ "supported": true, "counts": { "draft": 2, "scheduled": 9 } })),
        };
        let f = facts_from(Platform::ShortPost, &ready, 1_000);
        assert_eq!(
            (f.not_following_back, f.new_unfollowers, f.last_sync),
            (Some(12), Some(3), Some(900))
        );
        assert_eq!(
            (f.unread_hits, f.drafts, f.missed),
            (Some(7), Some(2), Some(0))
        );
        assert_eq!(f.audience_reason, None);
        let young = Sources {
            summary: Ok(
                json!({ "synced_at": { "followers": 900, "following": null }, "history": { "too_short": true } }),
            ),
            not_following_back: Ok(json!({ "total": 0, "empty_reason": "not_synced" })),
            unfollowers: Ok(json!({ "total": 0 })),
            monitors: Ok(json!({ "total": 0, "unread": 0 })),
            schedule: Ok(json!({ "supported": false, "counts": {} })),
        };
        let f = facts_from(Platform::ClipFeed, &young, 1_000);
        assert_eq!((f.not_following_back, f.new_unfollowers), (None, None));
        assert!(f.audience_reason.unwrap().contains("two complete syncs"));
        assert_eq!(f.unread_hits, None);
        assert!(f.monitor_reason.unwrap().contains("no monitor"));
        assert_eq!(f.drafts, None);
        assert!(f.schedule_reason.unwrap().contains("Clip Feed"));
        let broken = Sources {
            summary: Ok(json!({ "synced_at": { "followers": null, "following": null } })),
            not_following_back: Ok(json!({})),
            unfollowers: Ok(json!({})),
            monitors: Err("UNSUPPORTED_PLATFORM: {\"message\":\"not on Forums\",\"retryable\":false,\"next_step\":\"x\"}".into()),
            schedule: Err("STORE_UNAVAILABLE: the social database could not be opened".into()),
        };
        let f = facts_from(Platform::PhotoFeed, &broken, 1_000);
        assert!(f.audience_reason.unwrap().contains("ig_audience_sync"));
        assert_eq!(f.monitor_reason.as_deref(), Some("not on Forums"));
        assert!(f.schedule_reason.unwrap().starts_with("STORE_UNAVAILABLE"));
    }

    #[test]
    fn thin_data_returns_an_empty_list_with_the_reason_of_every_rule() {
        let few = [post(MONDAY_9, 5, false)];
        let f = Facts {
            posts: Some(&few),
            now: NOW,
            not_following_back: Some(0),
            audience_reason: Some("No audience snapshot yet; call ig_audience_sync.".into()),
            monitor_reason: Some("No monitor saved.".into()),
            schedule_reason: Some("Scheduling is not built for Photo Feed.".into()),
            ..Default::default()
        };
        let v = recommend(Platform::PhotoFeed, &f, &Utc);
        assert_eq!(v["total"], 0);
        assert_eq!(v["items"], json!([]));
        assert!(v["empty_reason"].is_string());
        let why = |id: &str| {
            v["not_triggered"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["id"] == id)
                .unwrap()["reason"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(v["not_triggered"].as_array().unwrap().len(), 8);
        assert!(why("best_time").contains("needs 10 dated posts"));
        assert!(why("media_type").contains("needs 10 posts"));
        assert!(why("frequency_drop").contains("28 days"));
        assert_eq!(why("not_following_back"), "the count is 0");
        assert!(why("new_unfollowers").contains("ig_audience_sync"));
        assert_eq!(why("unread_hits"), "No monitor saved.");
        assert!(why("drafts").contains("not built"));
        assert!(why("missed_posts").contains("not built"));
    }
}
