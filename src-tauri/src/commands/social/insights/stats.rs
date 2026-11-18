use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Datelike, Duration, TimeZone, Timelike};
use serde_json::{json, Value};

use crate::commands::social::model::{
    b, opt_s, s, u, Author, Media, MediaKind, Platform, Post, Stats,
};

pub const MIN_SAMPLE: usize = 20;
pub const LOW_CONFIDENCE_BELOW: usize = 10;
pub const THIN_CELL_BELOW: u64 = 3;

fn author_from(v: &Value) -> Author {
    Author {
        id: s(&v["id"]),
        handle: s(&v["handle"]),
        name: s(&v["name"]),
        avatar: opt_s(&v["avatar"]),
        verified: b(&v["verified"]),
    }
}

pub fn post_from(v: &Value) -> Option<Post> {
    let id = opt_s(&v["id"])?;
    let media = v["media"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|m| Media {
                    kind: if m["kind"] == "video" {
                        MediaKind::Video
                    } else {
                        MediaKind::Image
                    },
                    url: s(&m["url"]),
                    thumb: opt_s(&m["thumb"]),
                    width: u(&m["width"]),
                    height: u(&m["height"]),
                })
                .collect()
        })
        .unwrap_or_default();
    let st = &v["stats"];
    Some(Post {
        id,
        url: s(&v["url"]),
        author: author_from(&v["author"]),
        text: s(&v["text"]),
        created_at: v["created_at"].as_i64(),
        media,
        stats: Stats {
            likes: u(&st["likes"]),
            replies: u(&st["replies"]),
            reposts: u(&st["reposts"]),
            views: u(&st["views"]),
            saves: u(&st["saves"]),
        },
        liked: v["liked"].as_bool(),
        reposted: v["reposted"].as_bool(),
        reply_to: opt_s(&v["reply_to"]),
        reposted_by: v["reposted_by"]
            .is_object()
            .then(|| author_from(&v["reposted_by"])),
        quoted: post_from(&v["quoted"]).map(Box::new),
    })
}

pub fn engagement(p: &Post) -> u64 {
    [
        p.stats.likes,
        p.stats.replies,
        p.stats.reposts,
        p.stats.saves,
    ]
    .into_iter()
    .flatten()
    .sum()
}

pub fn post_type(p: &Post, platform: Platform) -> &'static str {
    match platform {
        Platform::ShortPost if p.reposted_by.is_some() => "repost",
        Platform::ShortPost if p.reply_to.is_some() => "reply",
        Platform::ShortPost if p.quoted.is_some() => "quote",
        Platform::ShortPost => "post",
        Platform::PhotoFeed if p.url.contains("/reel/") => "reel",
        Platform::PhotoFeed => "feed",
        Platform::ClipFeed if p.url.contains("/photo/") => "photo",
        Platform::ClipFeed => "video",
        _ => "post",
    }
}

pub fn media_type(p: &Post) -> &'static str {
    if p.url.contains("/reel/") {
        return "reel";
    }
    match p.media.as_slice() {
        [] => "text",
        [one] if one.kind == MediaKind::Video => "video",
        [_] => "image",
        _ => "carousel",
    }
}

pub fn originals(posts: &[Post]) -> Vec<&Post> {
    posts.iter().filter(|p| p.reposted_by.is_none()).collect()
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

pub fn mean(values: &[u64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<u64>() as f64 / values.len() as f64)
}

pub fn median(values: &[u64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_unstable();
    let mid = v.len() / 2;
    Some(if v.len() % 2 == 1 {
        v[mid] as f64
    } else {
        (v[mid - 1] + v[mid]) as f64 / 2.0
    })
}

fn spread(values: &[u64]) -> Value {
    match (mean(values), median(values)) {
        (Some(mean), Some(median)) => json!({
            "available": true,
            "posts": values.len(),
            "total": values.iter().sum::<u64>(),
            "mean": round2(mean),
            "median": round2(median),
        }),
        _ => json!({ "available": false }),
    }
}

pub fn unavailable(platform: Platform) -> Vec<Value> {
    let hidden = |metric: &str, reason: &str| json!({ "metric": metric, "reason": reason });
    let mut out = vec![
        hidden("reach", "Only the account owner sees it, inside the network's own analytics page; a web session does not receive it with the posts."),
        hidden("demographics", "Audience age, gender and location are never sent to a web session."),
        hidden("profile_visits", "Shown only in the network's own analytics page."),
    ];
    match platform {
        Platform::ShortPost => {
            out.push(hidden("impressions", "Short Posts sends the public view count of each post (the views column). The owner-only impression breakdown (detail expands, profile clicks, link clicks) is not sent."));
            out.push(hidden("quotes", "The Short Posts reader in this app adds quote posts to the repost count, so quotes have no column of their own. x_engagement with kind=quotes lists them for one post."));
        }
        Platform::PhotoFeed => {
            out.push(hidden("impressions", "Photo Feed sends plays only for reels; impressions of photo posts are owner-only insights."));
            out.push(hidden("saves", "Photo Feed does not send save counts to a web session, not even for your own posts."));
            out.push(hidden(
                "shares",
                "Photo Feed does not send share counts to a web session.",
            ));
            out.push(hidden("likes_when_hidden", "When the author hides like counts the number is absent and the post is left out of the likes average."));
        }
        Platform::ClipFeed => {
            out.push(hidden("impressions", "Clip Feed sends the public play count (the views column). Watch time, completion rate and traffic sources are owner-only."));
        }
        Platform::Forums => {
            out.push(hidden("views", "Forums shows view counts only to the author, inside its own post insights; the listings a web session reads do not carry them."));
            out.push(hidden(
                "reposts",
                "Crosspost counts are not in the user listing.",
            ));
            out.push(hidden(
                "saves",
                "Forums never shows how many people saved a post.",
            ));
            out.push(hidden("upvote_ratio", "The user listing carries the score (the likes column) and the comment count (the replies column), not the upvote ratio."));
        }
        Platform::PinBoard => {
            out.push(hidden("impressions", "Pin impressions and outbound clicks exist only in Pin Boards Analytics for business accounts; the pin list carries saves, reactions and comments when Pin Boards sends them."));
            out.push(hidden(
                "reposts",
                "On Pin Boards a repost is a save, counted in the saves column.",
            ));
        }
        Platform::CodeHub => {
            out.push(hidden(
                "likes",
                "Code Hub activity events carry no reaction counts.",
            ));
            out.push(hidden(
                "replies",
                "Code Hub activity events carry no comment counts.",
            ));
            out.push(hidden("engagement", "Nothing to add up per event; use report=repos for stars, forks and open issues per repository."));
            out.push(hidden(
                "older_activity",
                "The events API keeps about 90 days and 300 events per user.",
            ));
        }
    }
    out
}

pub fn summary(posts: &[Post], platform: Platform) -> Value {
    let mut types: BTreeMap<&str, u64> = BTreeMap::new();
    for p in posts {
        *types.entry(post_type(p, platform)).or_default() += 1;
    }
    let own = originals(posts);
    let column = |f: fn(&Post) -> Option<u64>| -> Value {
        spread(&own.iter().filter_map(|p| f(p)).collect::<Vec<_>>())
    };
    let dates: Vec<i64> = posts.iter().filter_map(|p| p.created_at).collect();
    let eng: Vec<u64> = own.iter().map(|p| engagement(p)).collect();
    let mut engagement_block = spread(&eng);
    engagement_block["formula"] =
        json!("likes + replies + reposts + saves, each where the network shows it");
    json!({
        "sample": posts.len(),
        "analyzed": own.len(),
        "from": dates.iter().min(),
        "to": dates.iter().max(),
        "types": types,
        "metrics": {
            "likes": column(|p| p.stats.likes),
            "replies": column(|p| p.stats.replies),
            "reposts": column(|p| p.stats.reposts),
            "saves": column(|p| p.stats.saves),
            "views": column(|p| p.stats.views),
        },
        "engagement": engagement_block,
        "confidence": confidence(own.len()),
        "unavailable": unavailable(platform),
    })
}

pub fn confidence(n: usize) -> &'static str {
    if n < LOW_CONFIDENCE_BELOW {
        "low"
    } else if n < MIN_SAMPLE {
        "medium"
    } else {
        "high"
    }
}

pub fn outlier_label(lift: f64) -> Option<&'static str> {
    if lift >= 5.0 {
        Some("huge")
    } else if lift >= 2.0 {
        Some("strong")
    } else if lift >= 1.5 {
        Some("mild")
    } else {
        None
    }
}

fn primary(p: &Post, platform: Platform) -> Option<u64> {
    match platform {
        Platform::ClipFeed => p.stats.views,
        _ => Some(engagement(p)),
    }
}

pub fn baseline(posts: &[Post], platform: Platform) -> Option<f64> {
    let values: Vec<u64> = originals(posts)
        .iter()
        .filter_map(|p| primary(p, platform))
        .collect();
    median(&values).filter(|m| *m > 0.0)
}

fn snippet(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 280 {
        return flat;
    }
    let mut cut: String = flat.chars().take(279).collect();
    cut.push('…');
    cut
}

pub fn rows(posts: &[Post], platform: Platform) -> Vec<Value> {
    let base = baseline(posts, platform);
    originals(posts)
        .into_iter()
        .map(|p| {
            let lift = base
                .zip(primary(p, platform))
                .map(|(m, v)| round2(v as f64 / m));
            json!({
                "id": p.id,
                "url": p.url,
                "created_at": p.created_at,
                "type": post_type(p, platform),
                "media": media_type(p),
                "text": snippet(&p.text),
                "likes": p.stats.likes,
                "replies": p.stats.replies,
                "reposts": p.stats.reposts,
                "saves": p.stats.saves,
                "views": p.stats.views,
                "engagement": engagement(p),
                "lift": lift,
                "outlier": lift.and_then(outlier_label),
            })
        })
        .collect()
}

pub const SORT_KEYS: &[&str] = &[
    "created_at",
    "likes",
    "replies",
    "reposts",
    "saves",
    "views",
    "engagement",
    "lift",
];

pub fn sort_rows(rows: &mut [Value], key: &str, ascending: bool) {
    let value = |r: &Value| r[key].as_f64();
    rows.sort_by(|a, b| {
        let ord = match (value(a), value(b)) {
            (Some(x), Some(y)) => {
                let o = x.total_cmp(&y);
                if ascending {
                    o
                } else {
                    o.reverse()
                }
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        ord.then_with(|| s(&a["id"]).cmp(&s(&b["id"])))
    });
}

pub fn outliers(posts: &[Post], platform: Platform) -> Value {
    let n = originals(posts).len();
    let metric = if platform == Platform::ClipFeed {
        "views"
    } else {
        "engagement"
    };
    let Some(base) = baseline(posts, platform) else {
        return json!({
            "metric": metric, "median": Value::Null, "sample": n, "confidence": confidence(n),
            "min_recommended": MIN_SAMPLE, "total": 0, "items": [],
            "reason": "The median of the sample is zero or the metric is missing, so no lift can be computed.",
        });
    };
    let mut items: Vec<Value> = rows(posts, platform)
        .into_iter()
        .filter(|r| !r["outlier"].is_null())
        .collect();
    sort_rows(&mut items, "lift", false);
    json!({
        "metric": metric,
        "median": round2(base),
        "sample": n,
        "confidence": confidence(n),
        "min_recommended": MIN_SAMPLE,
        "labels": { "mild": "1.5x to 2x the median", "strong": "2x to 5x", "huge": "5x and above" },
        "total": items.len(),
        "items": items,
    })
}

pub fn times<Tz: TimeZone>(posts: &[Post], tz: &Tz) -> Value {
    let mut cells: BTreeMap<(u32, u32), Vec<u64>> = BTreeMap::new();
    let mut undated = 0u64;
    for p in originals(posts) {
        match p.created_at.and_then(|ts| tz.timestamp_opt(ts, 0).single()) {
            Some(d) => cells
                .entry((d.weekday().num_days_from_monday(), d.hour()))
                .or_default()
                .push(engagement(p)),
            None => undated += 1,
        }
    }
    let list: Vec<Value> = cells
        .iter()
        .map(|((weekday, hour), v)| {
            json!({
                "weekday": weekday,
                "hour": hour,
                "posts": v.len(),
                "engagement": v.iter().sum::<u64>(),
                "mean": mean(v).map(round2),
                "thin": (v.len() as u64) < THIN_CELL_BELOW,
            })
        })
        .collect();
    let best = list
        .iter()
        .filter(|c| c["thin"] == false)
        .max_by(|a, b| {
            a["mean"]
                .as_f64()
                .unwrap_or_default()
                .total_cmp(&b["mean"].as_f64().unwrap_or_default())
        })
        .cloned();
    json!({
        "weekday_key": "0 = Monday … 6 = Sunday",
        "thin_below": THIN_CELL_BELOW,
        "undated": undated,
        "total": list.len(),
        "cells": list,
        "best": best,
    })
}

pub fn by_media(posts: &[Post]) -> Vec<Value> {
    let mut groups: BTreeMap<&str, Vec<&Post>> = BTreeMap::new();
    for p in originals(posts) {
        groups.entry(media_type(p)).or_default().push(p);
    }
    let mut out: Vec<Value> = groups
        .into_iter()
        .map(|(kind, list)| {
            let eng: Vec<u64> = list.iter().map(|p| engagement(p)).collect();
            let views: Vec<u64> = list.iter().filter_map(|p| p.stats.views).collect();
            json!({
                "media": kind,
                "posts": list.len(),
                "engagement_mean": mean(&eng).map(round2),
                "engagement_median": median(&eng).map(round2),
                "views_mean": mean(&views).map(round2),
            })
        })
        .collect();
    out.sort_by(|a, b| b["posts"].as_u64().cmp(&a["posts"].as_u64()));
    out
}

pub fn hashtags_of(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut prev: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let starts = (c == '#' || c == '＃')
            && !prev.is_some_and(|p| p.is_alphanumeric() || p == '/' || p == '&');
        prev = Some(c);
        if !starts {
            continue;
        }
        let mut tag = String::new();
        while let Some(n) = chars.peek().copied() {
            if n.is_alphanumeric() || n == '_' {
                tag.push(n);
                prev = Some(n);
                chars.next();
            } else {
                break;
            }
        }
        let tag = tag.to_lowercase();
        if !tag.is_empty() && !tag.chars().all(|c| c.is_numeric()) && !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

pub fn by_hashtag(posts: &[Post]) -> Vec<Value> {
    let mut groups: HashMap<String, Vec<u64>> = HashMap::new();
    for p in originals(posts) {
        for tag in hashtags_of(&p.text) {
            groups.entry(tag).or_default().push(engagement(p));
        }
    }
    let mut out: Vec<Value> = groups
        .into_iter()
        .map(|(tag, eng)| {
            json!({
                "hashtag": tag,
                "uses": eng.len(),
                "engagement_mean": mean(&eng).map(round2),
                "engagement_median": median(&eng).map(round2),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b["uses"]
            .as_u64()
            .cmp(&a["uses"].as_u64())
            .then_with(|| s(&a["hashtag"]).cmp(&s(&b["hashtag"])))
    });
    out
}

const STOP_WORDS: &str = "the and for that this with you your are was were not but have has had from they their them what when where which who will would can could should about into over out just than then too very all any our his her its she him been being more most some such only own same also there here how why did does doing its it's i'm don't \
que não nao uma para com por mais como mas foi ele ela eles elas isso esse essa este esta são sao tem têm ser está estão estou seu sua seus suas meu minha nos nas dos das pelo pela aos muito também tambem quando onde quem qual porque pra pro você voce vocês voces nós nos já ja só so até ate sem sobre entre depois antes \
los las del una unos unas por para con pero como más mas fue son está están esta este esto ese esa eso sus mis tus muy también cuando donde quien cual porque sin sobre entre hay todo todos ser \
les des une dans pour pas sur avec est sont mais plus que qui quoi ce cette ces son ses mon mes ton tes nous vous ils elles leur leurs été être avoir fait comme tout tous très aussi \
gli una uno nel nella nei nelle con per non che chi come più piu sono era erano suo sua suoi sue mio mia questo questa questi queste anche molto tutto tutti essere \
der die das und ist nicht mit auf für fur von den dem des ein eine einen einem einer auch sich als wie bei aus nach oder aber wenn wir ihr sie sind war";

fn stop_words() -> HashSet<&'static str> {
    STOP_WORDS.split_whitespace().collect()
}

pub fn words_of(text: &str) -> Vec<String> {
    let stop = stop_words();
    let mut out = Vec::new();
    for token in text.split_whitespace() {
        let lower = token.to_lowercase();
        if lower.contains("://")
            || lower.starts_with("www.")
            || lower.starts_with('@')
            || lower.starts_with('#')
            || lower.starts_with('＃')
        {
            continue;
        }
        for word in lower.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’')) {
            let word = word
                .trim_matches(|c| c == '\'' || c == '’')
                .replace('’', "'");
            let len = word.chars().count();
            let min = if word.is_ascii() { 3 } else { 2 };
            if len < min || word.chars().all(|c| c.is_numeric()) || stop.contains(word.as_str()) {
                continue;
            }
            out.push(word);
        }
    }
    out
}

pub fn by_word(posts: &[Post]) -> Vec<Value> {
    let mut uses: HashMap<String, (u64, u64)> = HashMap::new();
    for p in originals(posts) {
        let words = words_of(&p.text);
        let distinct: HashSet<&String> = words.iter().collect();
        for w in &distinct {
            uses.entry((*w).clone()).or_default().1 += 1;
        }
        for w in words {
            uses.entry(w).or_default().0 += 1;
        }
    }
    let mut out: Vec<Value> = uses
        .into_iter()
        .map(|(word, (uses, posts))| json!({ "word": word, "uses": uses, "posts": posts }))
        .collect();
    out.sort_by(|a, b| {
        b["uses"]
            .as_u64()
            .cmp(&a["uses"].as_u64())
            .then_with(|| s(&a["word"]).cmp(&s(&b["word"])))
    });
    out
}

pub fn trend<Tz: TimeZone>(posts: &[Post], tz: &Tz, weekly: bool) -> Vec<Value> {
    let mut buckets: BTreeMap<chrono::NaiveDate, Vec<u64>> = BTreeMap::new();
    for p in originals(posts) {
        let Some(d) = p.created_at.and_then(|ts| tz.timestamp_opt(ts, 0).single()) else {
            continue;
        };
        let mut day = d.date_naive();
        if weekly {
            day -= Duration::days(i64::from(day.weekday().num_days_from_monday()));
        }
        buckets.entry(day).or_default().push(engagement(p));
    }
    buckets
        .into_iter()
        .map(|(day, eng)| {
            json!({
                "start": day.format("%Y-%m-%d").to_string(),
                "posts": eng.len(),
                "engagement": eng.iter().sum::<u64>(),
                "mean": mean(&eng).map(round2),
            })
        })
        .collect()
}

pub fn same_sample(sets: &[(String, Vec<Post>)]) -> (usize, Vec<(String, Vec<Post>)>) {
    let n = sets
        .iter()
        .map(|(_, posts)| originals(posts).len())
        .min()
        .unwrap_or(0);
    let trimmed = sets
        .iter()
        .map(|(handle, posts)| {
            let mut own: Vec<Post> = originals(posts).into_iter().cloned().collect();
            own.sort_by_key(|b| std::cmp::Reverse(b.created_at));
            own.truncate(n);
            (handle.clone(), own)
        })
        .collect();
    (n, trimmed)
}

pub fn compare(sets: &[(String, Vec<Post>)], platform: Platform) -> Value {
    let (n, trimmed) = same_sample(sets);
    let accounts: Vec<Value> = trimmed
        .iter()
        .map(|(handle, posts)| json!({ "handle": handle, "summary": summary(posts, platform) }))
        .collect();
    json!({ "sample_each": n, "confidence": confidence(n), "total": accounts.len(), "accounts": accounts })
}

pub const CSV_COLUMNS: &[&str] = &[
    "id",
    "url",
    "created_at",
    "type",
    "media",
    "likes",
    "replies",
    "reposts",
    "saves",
    "views",
    "engagement",
    "lift",
    "outlier",
    "text",
];

fn csv_cell(v: &Value) -> String {
    let raw = match v {
        Value::Null => String::new(),
        Value::String(t) => t.clone(),
        other => other.to_string(),
    };
    let guarded = if raw.starts_with(['=', '+', '-', '@', '\t', '\r']) && v.is_string() {
        format!("'{raw}")
    } else {
        raw
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

pub fn csv(rows: &[Value]) -> String {
    let mut out = CSV_COLUMNS.join(",");
    out.push('\n');
    for r in rows {
        let line: Vec<String> = CSV_COLUMNS
            .iter()
            .map(|c| {
                if *c == "created_at" {
                    r[*c]
                        .as_i64()
                        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                        .map(|d| d.to_rfc3339())
                        .unwrap_or_default()
                } else {
                    csv_cell(&r[*c])
                }
            })
            .collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn post(id: &str, likes: u64, text: &str, ts: i64) -> Post {
        Post {
            id: id.into(),
            url: format!("https://x.com/ana/status/{id}"),
            text: text.into(),
            created_at: Some(ts),
            stats: Stats {
                likes: Some(likes),
                replies: Some(1),
                reposts: Some(0),
                views: None,
                saves: None,
            },
            ..Default::default()
        }
    }

    fn sample(likes: &[u64]) -> Vec<Post> {
        likes
            .iter()
            .enumerate()
            .map(|(i, l)| post(&i.to_string(), *l, "", 1_700_000_000 + i as i64 * 3600))
            .collect()
    }

    #[test]
    fn mean_and_median_of_odd_even_and_empty_lists() {
        assert_eq!(mean(&[1, 2, 6]), Some(3.0));
        assert_eq!(median(&[9, 1, 5]), Some(5.0));
        assert_eq!(median(&[4, 1, 2, 100]), Some(3.0));
        assert_eq!(mean(&[]), None);
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn a_post_survives_the_round_trip_through_its_json() {
        let mut p = post("7", 3, "olá", 10);
        p.reply_to = Some("6".into());
        p.reposted_by = Some(Author {
            handle: "bob".into(),
            ..Default::default()
        });
        p.media = vec![Media {
            kind: MediaKind::Video,
            url: "v.mp4".into(),
            thumb: None,
            width: Some(4),
            height: None,
        }];
        p.quoted = Some(Box::new(post("5", 1, "q", 9)));
        assert_eq!(post_from(&json!(p)), Some(p));
        assert_eq!(post_from(&json!({ "text": "no id" })), None);
    }

    #[test]
    fn summary_counts_types_and_leaves_reposts_out_of_the_averages() {
        let mut posts = sample(&[10, 20, 90]);
        posts[1].reply_to = Some("1".into());
        let mut repost = post("r", 5000, "", 1_600_000_000);
        repost.reposted_by = Some(Author::default());
        posts.push(repost);
        let v = summary(&posts, Platform::ShortPost);
        assert_eq!(v["sample"], 4);
        assert_eq!(v["analyzed"], 3);
        assert_eq!(v["types"], json!({ "post": 2, "reply": 1, "repost": 1 }));
        assert_eq!(v["metrics"]["likes"]["total"], 120);
        assert_eq!(v["metrics"]["likes"]["mean"], 40.0);
        assert_eq!(v["metrics"]["likes"]["median"], 20.0);
        assert_eq!(v["metrics"]["views"], json!({ "available": false }));
        assert_eq!(v["engagement"]["total"], 123);
        assert_eq!(v["engagement"]["median"], 21.0);
        assert_eq!(v["from"], 1_600_000_000);
        assert_eq!(v["to"], 1_700_007_200);
        assert_eq!(v["confidence"], "low");
        let names: Vec<String> = v["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| s(&x["metric"]))
            .collect();
        for expected in ["impressions", "reach", "demographics", "quotes"] {
            assert!(names.contains(&expected.to_string()), "{expected}");
        }
    }

    #[test]
    fn photofeed_says_saves_are_hidden_instead_of_guessing() {
        let v = summary(&sample(&[1]), Platform::PhotoFeed);
        assert_eq!(v["metrics"]["saves"], json!({ "available": false }));
        assert!(v["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["metric"] == "saves"));
    }

    #[test]
    fn classifies_post_and_media_types() {
        let mut p = post("1", 0, "", 0);
        assert_eq!(media_type(&p), "text");
        let img = Media {
            kind: MediaKind::Image,
            url: String::new(),
            thumb: None,
            width: None,
            height: None,
        };
        p.media = vec![img.clone()];
        assert_eq!(media_type(&p), "image");
        p.media = vec![img.clone(), img.clone()];
        assert_eq!(media_type(&p), "carousel");
        p.media = vec![Media {
            kind: MediaKind::Video,
            ..img
        }];
        assert_eq!(media_type(&p), "video");
        p.url = "https://www.instagram.com/reel/abc/".into();
        assert_eq!(media_type(&p), "reel");
        assert_eq!(post_type(&p, Platform::PhotoFeed), "reel");
        p.url = "https://www.tiktok.com/@a/photo/1".into();
        assert_eq!(post_type(&p, Platform::ClipFeed), "photo");
        p.quoted = Some(Box::new(post("2", 0, "", 0)));
        assert_eq!(post_type(&p, Platform::ShortPost), "quote");
    }

    #[test]
    fn outlier_labels_follow_the_median_rule() {
        assert_eq!(outlier_label(1.49), None);
        assert_eq!(outlier_label(1.5), Some("mild"));
        assert_eq!(outlier_label(2.0), Some("strong"));
        assert_eq!(outlier_label(4.99), Some("strong"));
        assert_eq!(outlier_label(5.0), Some("huge"));
        assert_eq!(confidence(9), "low");
        assert_eq!(confidence(10), "medium");
        assert_eq!(confidence(20), "high");
    }

    #[test]
    fn outliers_are_measured_against_the_median_of_the_sample() {
        let posts = sample(&[9, 9, 9, 9, 14, 19, 49]);
        let v = outliers(&posts, Platform::ShortPost);
        assert_eq!(v["median"], 10.0);
        assert_eq!(v["confidence"], "low");
        let got: Vec<(String, String)> = v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (s(&r["id"]), s(&r["outlier"])))
            .collect();
        assert_eq!(
            got,
            vec![
                ("6".to_string(), "huge".to_string()),
                ("5".to_string(), "strong".to_string()),
                ("4".to_string(), "mild".to_string())
            ]
        );
        let zero = outliers(&[post("1", 0, "", 0)], Platform::ClipFeed);
        assert_eq!(zero["total"], 0);
        assert!(zero["reason"].is_string());
    }

    #[test]
    fn clipfeed_outliers_use_views() {
        let mut posts = sample(&[1, 1, 1]);
        for (p, views) in posts.iter_mut().zip([100, 100, 1000]) {
            p.stats.views = Some(views);
        }
        let v = outliers(&posts, Platform::ClipFeed);
        assert_eq!(v["metric"], "views");
        assert_eq!(v["items"][0]["lift"], 10.0);
    }

    #[test]
    fn times_bucket_in_the_given_time_zone_and_show_thin_cells() {
        let monday_0130_utc = 1_700_443_800;
        let posts = vec![
            post("a", 9, "", monday_0130_utc),
            post("b", 19, "", monday_0130_utc + 60),
            post("c", 29, "", monday_0130_utc + 120),
            post("d", 4, "", monday_0130_utc + 3 * 3600),
        ];
        let utc = times(&posts, &FixedOffset::east_opt(0).unwrap());
        assert_eq!(utc["cells"][0]["weekday"], 0);
        assert_eq!(utc["cells"][0]["hour"], 1);
        assert_eq!(utc["cells"][0]["posts"], 3);
        assert_eq!(utc["cells"][0]["mean"], 20.0);
        assert_eq!(utc["cells"][0]["thin"], false);
        assert_eq!(utc["cells"][1]["thin"], true);
        assert_eq!(utc["best"]["hour"], 1);
        let sao_paulo = times(&posts, &FixedOffset::west_opt(3 * 3600).unwrap());
        assert_eq!(sao_paulo["cells"][0]["weekday"], 0);
        assert_eq!(sao_paulo["cells"][0]["hour"], 1);
        assert_eq!(sao_paulo["cells"][1]["weekday"], 6);
        assert_eq!(sao_paulo["cells"][1]["hour"], 22);
        assert_eq!(sao_paulo["cells"][1]["posts"], 3);
    }

    #[test]
    fn hashtags_are_read_with_unicode_and_once_per_post() {
        assert_eq!(
            hashtags_of("#Café com #東京 e #café, #2024 a#b https://x.com/#frag #a_b ＃全角"),
            vec!["café", "東京", "a_b", "全角"]
        );
        let posts = vec![
            post("1", 9, "#rust #dev", 0),
            post("2", 19, "mais #Rust", 0),
        ];
        let v = by_hashtag(&posts);
        assert_eq!(
            v[0],
            json!({ "hashtag": "rust", "uses": 2, "engagement_mean": 15.0, "engagement_median": 15.0 })
        );
        assert_eq!(v[1]["hashtag"], "dev");
    }

    #[test]
    fn words_skip_links_mentions_stop_words_and_keep_unicode() {
        assert_eq!(
            words_of(
                "The Árvore não é @ana #tag https://t.co/x árvore, don't 2024 東京 it's coração"
            ),
            vec!["árvore", "árvore", "東京", "coração"]
        );
        let v = by_word(&[post("1", 0, "rust rust tauri", 0), post("2", 0, "Rust", 0)]);
        assert_eq!(v[0], json!({ "word": "rust", "uses": 3, "posts": 2 }));
    }

    #[test]
    fn trend_groups_by_local_day_or_week() {
        let sunday_2330_utc = 1_700_436_600;
        let posts = vec![
            post("a", 1, "", sunday_2330_utc),
            post("b", 3, "", sunday_2330_utc + 3600),
        ];
        let tz = FixedOffset::east_opt(0).unwrap();
        let days = trend(&posts, &tz, false);
        assert_eq!(days.len(), 2);
        assert_eq!(days[0]["start"], "2023-11-19");
        assert_eq!(days[1]["start"], "2023-11-20");
        let weeks = trend(&posts, &tz, true);
        assert_eq!(weeks[0]["start"], "2023-11-13");
        assert_eq!(weeks[1]["start"], "2023-11-20");
        let west = trend(&posts, &FixedOffset::west_opt(3 * 3600).unwrap(), false);
        assert_eq!(west.len(), 1);
        assert_eq!(west[0]["posts"], 2);
        assert_eq!(west[0]["engagement"], 6);
    }

    #[test]
    fn compare_uses_the_same_number_of_newest_posts_for_everyone() {
        let mut big = sample(&[1, 2, 3, 100]);
        let mut repost = post("r", 7, "", 1_800_000_000);
        repost.reposted_by = Some(Author::default());
        big.push(repost);
        let small = sample(&[10, 30]);
        let v = compare(
            &[("big".into(), big), ("small".into(), small)],
            Platform::ShortPost,
        );
        assert_eq!(v["sample_each"], 2);
        assert_eq!(v["accounts"][0]["handle"], "big");
        assert_eq!(v["accounts"][0]["summary"]["analyzed"], 2);
        assert_eq!(
            v["accounts"][0]["summary"]["metrics"]["likes"]["total"],
            103
        );
        assert_eq!(
            v["accounts"][1]["summary"]["metrics"]["likes"]["mean"],
            20.0
        );
    }

    #[test]
    fn rows_sort_by_any_column_with_missing_values_last() {
        let mut posts = sample(&[5, 50, 20]);
        posts[1].stats.views = Some(10);
        posts[2].stats.views = Some(99);
        let mut r = rows(&posts, Platform::ShortPost);
        sort_rows(&mut r, "likes", false);
        assert_eq!(
            r.iter().map(|x| s(&x["id"])).collect::<Vec<_>>(),
            vec!["1", "2", "0"]
        );
        sort_rows(&mut r, "views", true);
        assert_eq!(
            r.iter().map(|x| s(&x["id"])).collect::<Vec<_>>(),
            vec!["1", "2", "0"]
        );
        assert_eq!(r[0]["engagement"], 51);
    }

    #[test]
    fn csv_escapes_text_and_defuses_formulas() {
        let mut p = post("1", 2, "=SUM(A1) \"hi\",\nthere", 0);
        p.stats.views = Some(7);
        let out = csv(&rows(&[p], Platform::ShortPost));
        let mut lines = out.lines();
        assert_eq!(lines.next(), Some(CSV_COLUMNS.join(",").as_str()));
        assert_eq!(
            lines.next(),
            Some("1,https://x.com/ana/status/1,1970-01-01T00:00:00+00:00,post,text,2,1,0,,7,3,1.0,,\"'=SUM(A1) \"\"hi\"\", there\"")
        );
    }
}
