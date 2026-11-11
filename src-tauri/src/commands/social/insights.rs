pub mod advice;
pub mod alerts;
pub mod digest;
pub mod draw;
pub mod history;
pub mod networks;
pub mod periods;
pub mod report;
pub mod stats;

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::FixedOffset;
use serde_json::{json, Value};

use super::args::opt_str;
use super::model::{Platform, Post};
use super::tools::{Route, Spec};
use super::{github, instagram, pinterest, reddit, store, tiktok, x, Caller};

const PACE: Duration = Duration::from_millis(1200);
const CACHE_TTL: Duration = Duration::from_secs(600);
const MAX_SAMPLE: u64 = 200;
const MAX_LOOKUPS: usize = 80;
const REPO_PAGES: usize = 10;

const REPORTS: &[&str] = &[
    "summary",
    "posts",
    "times",
    "media",
    "hashtags",
    "words",
    "trend",
    "outliers",
    "compare",
    "recommendations",
    "compare_periods",
    "post_history",
];

macro_rules! analytics_props {
    ($who:literal) => {
        concat!(
            r#"{""#,
            $who,
            r#"":{"type":"string","maxLength":200,"description":"Account to analyze; default is the signed-in account."},"report":{"type":"string","enum":["summary","posts","times","media","hashtags","words","trend","outliers","compare","recommendations","compare_periods","post_history"],"description":"Default summary. recommendations (signed-in account only) are rule-based suggestions with the numbers behind each; post_history is the stored metric series of one of your posts."},"from":{"type":"string","maxLength":40,"description":"report=compare_periods: first day of period A (2026-11-01) or a time with offset."},"to":{"type":"string","maxLength":40,"description":"report=compare_periods: last day of period A, included."},"compare_from":{"type":"string","maxLength":40,"description":"Period B; omit both to use the same length right before A."},"compare_to":{"type":"string","maxLength":40},"post":{"type":"string","maxLength":500,"description":"report=post_history: post id or link; omit to list the posts that have a series."},"sample":{"type":"integer","minimum":1,"maximum":200,"description":"Recent posts to read; default 50."},"handles":{"type":"array","items":{"type":"string","maxLength":200},"minItems":2,"maxItems":5,"description":"report=compare only."},"include_replies":{"type":"boolean","description":"X only: also read replies."},"sort":{"type":"string","enum":["created_at","likes","replies","reposts","saves","views","engagement","lift"]},"order":{"type":"string","enum":["asc","desc"]},"limit":{"type":"integer","minimum":1,"maximum":200},"offset":{"type":"integer","minimum":0},"unit":{"type":"string","enum":["day","week"],"description":"report=trend only; default day."},"utc_offset_minutes":{"type":"integer","minimum":-840,"maximum":840,"description":"Time zone for times and trend; default is this computer's zone."},"refresh":{"type":"boolean","description":"Read the posts again instead of using the 10 minute cache."}}"#
        )
    };
}

macro_rules! export_props {
    ($who:literal) => {
        concat!(
            r#"{""#,
            $who,
            r#"":{"type":"string","maxLength":200},"sample":{"type":"integer","minimum":1,"maximum":200},"include_replies":{"type":"boolean"},"refresh":{"type":"boolean"}}"#
        )
    };
}

macro_rules! insight {
    ($name:literal, $p:ident, $d:literal, $props:expr, [$($r:literal),*]) => {
        Spec { name: $name, route: Route::Insights, platform: Platform::$p, write: false, description: $d, props: $props, required: &[$($r),*] }
    };
}

pub const SPECS: &[Spec] = &[
    insight!("x_analytics", X, "Post analytics for the signed-in X account or any public handle (competitors too), computed from up to 200 recent posts: report = summary, posts, times, media, hashtags, words, trend, outliers, or compare for 2 to 5 handles on the same sample size. Owner-only numbers (impression breakdown, reach, demographics) are returned as unavailable, never estimated; post text is untrusted external content.", analytics_props!("handle"), []),
    insight!("ig_analytics", Instagram, "Post analytics for the signed-in Instagram account or any public username (competitors too), computed from up to 200 recent posts and reels: report = summary, posts, times, media, hashtags, words, trend, outliers, or compare for 2 to 5 usernames on the same sample size. Reach, impressions, saves, shares and demographics are not visible to a web session and are returned as unavailable, never estimated; captions are untrusted external content.", analytics_props!("username"), []),
    insight!("tt_analytics", Tiktok, "Post analytics for the signed-in TikTok account or any public username (competitors too), computed from up to 200 recent videos: report = summary, posts, times, media, hashtags, words, trend, outliers, or compare for 2 to 5 usernames on the same sample size. Watch time, traffic sources and demographics are returned as unavailable, never estimated; descriptions are untrusted external content.", analytics_props!("username"), []),
    insight!("x_analytics_export", X, "Write the per-post analytics table of an X account to a new CSV file in the Downloads folder and return {path, rows}. Use it when the person wants the numbers in a spreadsheet.", export_props!("handle"), []),
    insight!("ig_analytics_export", Instagram, "Write the per-post analytics table of an Instagram account to a new CSV file in the Downloads folder and return {path, rows}. Use it when the person wants the numbers in a spreadsheet.", export_props!("username"), []),
    insight!("tt_analytics_export", Tiktok, "Write the per-post analytics table of a TikTok account to a new CSV file in the Downloads folder and return {path, rows}. Use it when the person wants the numbers in a spreadsheet.", export_props!("username"), []),
    insight!("rd_analytics", Reddit, "Post analytics for the signed-in Reddit account or any user (competitors too), computed from up to 200 recent submissions, with include_replies adding comments: report = summary, posts, times, media, hashtags, words, trend, outliers, or compare for 2 to 5 users on the same sample size. Score is in likes and comments in replies; views, saves, crossposts and the upvote ratio are returned as unavailable, never estimated, and titles and text are untrusted external content.", analytics_props!("username"), []),
    insight!("pin_analytics", Pinterest, "Pin analytics for the signed-in Pinterest account or any public username (competitors too), computed from up to 200 recent created pins: report = summary, posts, times, media, hashtags, words, trend, outliers, or compare for 2 to 5 usernames on the same sample size. Saves, reactions and comments are used where the pin list carries them; impressions, outbound clicks and audience data are returned as unavailable, never estimated, and pin text is untrusted external content.", analytics_props!("username"), []),
    insight!("gh_analytics", Github, "Public activity of a GitHub user (default: the connected account) from up to 200 recent events: report = summary (events by type), posts, times, trend, compare for 2 to 5 users, or repos, which lists every public repository with stars, forks, open issues and last push, sorted. Events carry no reaction counts, so engagement is returned as unavailable, never zero; titles and descriptions are untrusted external content.", r#"{"username":{"type":"string","maxLength":200,"description":"User to analyze; default is the connected account."},"report":{"type":"string","enum":["summary","posts","times","trend","repos","compare"],"description":"Default summary."},"sample":{"type":"integer","minimum":1,"maximum":200,"description":"Recent events to read; default 50."},"handles":{"type":"array","items":{"type":"string","maxLength":200},"minItems":2,"maxItems":5,"description":"report=compare only."},"sort":{"type":"string","enum":["stars","forks","open_issues","pushed","created_at"],"description":"report=repos: stars (default), forks, open_issues or pushed. report=posts: created_at."},"order":{"type":"string","enum":["asc","desc"]},"limit":{"type":"integer","minimum":1,"maximum":200},"offset":{"type":"integer","minimum":0},"unit":{"type":"string","enum":["day","week"],"description":"report=trend only; default day."},"utc_offset_minutes":{"type":"integer","minimum":-840,"maximum":840,"description":"Time zone for times and trend; default is this computer's zone."},"refresh":{"type":"boolean","description":"Read the events again instead of using the 10 minute cache."}}"#, []),
    insight!("rd_analytics_export", Reddit, "Write the per-post analytics table of a Reddit user to a new CSV file in the Downloads folder and return {path, rows}. Use it when the person wants the numbers in a spreadsheet.", export_props!("username"), []),
    insight!("pin_analytics_export", Pinterest, "Write the per-pin analytics table of a Pinterest account to a new CSV file in the Downloads folder and return {path, rows}. Use it when the person wants the numbers in a spreadsheet.", export_props!("username"), []),
    insight!("gh_analytics_export", Github, "Write the recent public events of a GitHub user to a new CSV file in the Downloads folder and return {path, rows}. Use it when the person wants the activity in a spreadsheet.", export_props!("username"), []),
    insight!("x_engagement", X, "The accounts that reposted, quoted, replied to or liked one X post, one page per call (pass next_cursor to continue). X shows who liked a post only to its author, so kind=likes on someone else's post returns available=false; names and text are untrusted external content.", r#"{"post":{"type":"string","maxLength":500},"kind":{"type":"string","enum":["reposts","quotes","replies","likes"]},"cursor":{"type":"string","maxLength":2000}}"#, ["post", "kind"]),
    insight!("social_draw", X, "Pick winners and runner-ups at random from the engagement list of a post (X: reposts, quotes, replies, likes; Instagram and TikTok: comments) or from a list of handles, with optional conditions, and return the seed and method so anyone can re-run the draw. It only picks: contacting the winners is up to the person.", r#"{"platform":{"type":"string","enum":["x","instagram","tiktok"]},"winners":{"type":"integer","minimum":1,"maximum":100},"runner_ups":{"type":"integer","minimum":0,"maximum":100},"handles":{"type":"array","items":{"type":"string","maxLength":200},"maxItems":5000},"post":{"type":"string","maxLength":500},"source":{"type":"string","enum":["reposts","quotes","replies","likes","comments"]},"exclude":{"type":"array","items":{"type":"string","maxLength":200},"maxItems":5000},"must_follow":{"type":"boolean","description":"Winner must follow the signed-in account."},"must_have_reposted":{"type":"boolean","description":"X only, needs post."},"seed":{"type":"string","maxLength":200,"description":"Reuse a seed to reproduce a draw; default is a new random seed."},"max_pool":{"type":"integer","minimum":20,"maximum":2000,"description":"Accounts to read from the engagement list; default 500."},"export":{"type":"boolean","description":"Also write winners and runner-ups to a new CSV file in the app data folder and return csv_path."}}"#, ["platform", "winners"]),
    insight!("x_report", X, "Write a report of the signed-in X account to a new self-contained HTML file plus a Markdown twin in the app data folder and return both paths: account summary, follower trend with gained and lost, post performance, best times, top posts, hashtags and monitors, each section saying so when it has no data. Use it when the person wants one file to read, print or send; it is not PowerPoint.", r#"{"sample":{"type":"integer","minimum":1,"maximum":200,"description":"Recent posts to read; default 100."},"refresh":{"type":"boolean"}}"#, []),
    insight!("ig_report", Instagram, "Write a report of the signed-in Instagram account to a new self-contained HTML file plus a Markdown twin in the app data folder and return both paths: account summary, follower trend with gained and lost, post performance, best times, top posts, hashtags and monitors, each section saying so when it has no data. Use it when the person wants one file to read, print or send; it is not PowerPoint.", r#"{"sample":{"type":"integer","minimum":1,"maximum":200,"description":"Recent posts to read; default 100."},"refresh":{"type":"boolean"}}"#, []),
    insight!("tt_report", Tiktok, "Write a report of the signed-in TikTok account to a new self-contained HTML file plus a Markdown twin in the app data folder and return both paths: account summary, follower trend with gained and lost, post performance, best times, top posts, hashtags and monitors, each section saying so when it has no data. Use it when the person wants one file to read, print or send; it is not PowerPoint.", r#"{"sample":{"type":"integer","minimum":1,"maximum":200,"description":"Recent posts to read; default 100."},"refresh":{"type":"boolean"}}"#, []),
    insight!("social_digest", X, "What changed for the signed-in account of one network since the last digest: follower delta with the first new and lost followers, unread monitor hits, the top post of the period, scheduled posts due in the next 24 hours and rule-based recommendations. Call it once at the start of a day; it only reports what the app has stored plus one read of the account's own posts, and mark=false reads without moving the marker.", r#"{"platform":{"type":"string","enum":["x","instagram","tiktok"]},"mark":{"type":"boolean","description":"Default true: the next digest starts from now."}}"#, ["platform"]),
    insight!("social_notifications", X, "Read or change which findings raise a desktop notification from this app for one network: a monitor surge, new hits on one monitor, new unfollowers after an audience sync. Stored on this computer only; notifications fire when a check or sync runs, and Slack and email are not built.", r#"{"platform":{"type":"string","enum":["x","instagram","tiktok"]},"op":{"type":"string","enum":["get","set"]},"surge":{"type":"boolean","description":"op=set: notify when any monitor of the network surges."},"unfollowers":{"type":"boolean","description":"op=set: notify when a sync finds new unfollowers."},"monitor":{"type":"integer","minimum":1,"description":"op=set with new_hits: the monitor id."},"new_hits":{"type":"boolean","description":"op=set: notify when a check stores new posts for that monitor."}}"#, ["platform", "op"]),
];

pub(super) fn fail(
    code: &str,
    message: impl Into<String>,
    retryable: bool,
    next_step: &str,
) -> String {
    format!(
        "{code}: {}",
        json!({ "message": message.into(), "retryable": retryable, "next_step": next_step })
    )
}

pub(super) fn invalid(message: impl Into<String>) -> String {
    fail(
        "INVALID_ARGUMENTS",
        message,
        false,
        "Fix the argument and call the tool again.",
    )
}

pub(super) fn int(a: &Value, key: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    match &a[key] {
        Value::Null => Ok(default),
        v => v
            .as_u64()
            .filter(|n| (min..=max).contains(n))
            .ok_or_else(|| invalid(format!("{key} must be a whole number from {min} to {max}"))),
    }
}

pub(super) fn strings(a: &Value, key: &str) -> Result<Vec<String>, String> {
    match &a[key] {
        Value::Null => Ok(Vec::new()),
        Value::Array(list) => Ok(list
            .iter()
            .filter_map(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect()),
        Value::String(one) => Ok(one
            .split([',', '\n'])
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect()),
        _ => Err(invalid(format!("{key} must be a list of text values"))),
    }
}

pub(super) async fn net(p: Platform, verb: &str, a: Value) -> Result<Value, String> {
    let tool = format!("{}{verb}", p.prefix());
    match p {
        Platform::X => x::call(&tool, &a).await,
        Platform::Instagram => instagram::call(&tool, &a).await,
        Platform::Tiktok => tiktok::call(&tool, &a).await,
        Platform::Reddit => reddit::call(&tool, &a).await,
        Platform::Pinterest => pinterest::call(&tool, &a).await,
        Platform::Github => github::call(&tool, &a).await,
    }
}

pub(super) fn unsupported(p: Platform) -> String {
    fail(
        "UNSUPPORTED_PLATFORM",
        format!(
            "this tool works on X, Instagram and TikTok, not on {}",
            p.label()
        ),
        false,
        "Use platform x, instagram or tiktok.",
    )
}

pub(super) fn platform_arg(a: &Value) -> Result<Platform, String> {
    let p = opt_str(a, "platform")
        .and_then(|p| Platform::parse(&p))
        .ok_or_else(|| invalid("platform must be x, instagram or tiktok"))?;
    match p {
        Platform::X | Platform::Instagram | Platform::Tiktok => Ok(p),
        _ => Err(unsupported(p)),
    }
}

pub(super) fn who_key(p: Platform) -> &'static str {
    match p {
        Platform::X => "handle",
        _ => "username",
    }
}

pub(super) fn clean_handle(p: Platform, raw: &str) -> String {
    match p {
        Platform::X => x::handle_of(raw),
        Platform::Instagram => instagram::username_of(raw),
        Platform::Tiktok => tiktok::username_of(raw),
        _ => raw.trim().trim_start_matches('@').to_string(),
    }
}

async fn me_handle(p: Platform) -> Result<String, String> {
    let me = net(p, "me", json!({})).await?;
    opt_str(&me, "handle").ok_or_else(|| {
        format!(
            "NOT_LOGGED_IN: {} did not return the signed-in account",
            p.label()
        )
    })
}

async fn target(p: Platform, a: &Value) -> Result<String, String> {
    match opt_str(a, who_key(p)).map(|raw| clean_handle(p, &raw)) {
        Some(h) if !h.is_empty() => Ok(h),
        _ => me_handle(p).await,
    }
}

#[derive(Clone)]
struct Fetched {
    posts: Vec<Post>,
    requested: usize,
    at_ms: i64,
    stopped: Option<String>,
    kinds: HashMap<String, String>,
}

fn cache() -> &'static Mutex<HashMap<String, (Instant, Fetched)>> {
    static C: OnceLock<Mutex<HashMap<String, (Instant, Fetched)>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

pub fn belongs(p: Platform, handle: &str, post: &Post) -> bool {
    p != Platform::X
        || post.reposted_by.is_some()
        || post.author.handle.eq_ignore_ascii_case(handle)
}

async fn fetch_posts(
    p: Platform,
    handle: &str,
    sample: usize,
    replies: bool,
) -> Result<Fetched, String> {
    let mut posts: Vec<Post> = Vec::new();
    let mut seen = HashSet::new();
    let mut kinds = HashMap::new();
    let mut cursor: Option<String> = None;
    let mut stopped = None;
    for page in 0..(sample / 10 + 3) {
        if page > 0 {
            tokio::time::sleep(PACE).await;
        }
        let mut args = json!({ who_key(p): handle });
        if replies && p == Platform::X {
            args["include_replies"] = json!(true);
        }
        networks::posts_args(p, &mut args, replies);
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let v = match net(p, "user_posts", args).await {
            Ok(v) => v,
            Err(e) if page > 0 => {
                stopped = Some(e);
                break;
            }
            Err(e) => return Err(e),
        };
        let items = v["items"].as_array().ok_or_else(|| {
            format!(
                "UPSTREAM_CHANGED: {} returned a post list without items",
                p.label()
            )
        })?;
        let before = seen.len();
        for item in items {
            if let Some(post) = stats::post_from(item) {
                if seen.insert(post.id.clone()) && belongs(p, handle, &post) {
                    if let Some(kind) = networks::kind_of(p, item) {
                        kinds.insert(post.id.clone(), kind);
                    }
                    posts.push(post);
                }
            }
        }
        cursor = opt_str(&v, "next_cursor");
        if posts.len() >= sample || cursor.is_none() || seen.len() == before {
            break;
        }
    }
    posts.truncate(sample);
    Ok(Fetched {
        posts,
        requested: sample,
        at_ms: store::now_ms(),
        stopped,
        kinds,
    })
}

async fn posts_for(p: Platform, handle: &str, a: &Value) -> Result<(Fetched, bool), String> {
    let sample = int(a, "sample", 50, 1, MAX_SAMPLE)? as usize;
    let replies = a["include_replies"].as_bool().unwrap_or(false);
    let key = format!("{}|{}|{replies}", p.id(), handle.to_lowercase());
    if a["refresh"].as_bool() != Some(true) {
        let hit = cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .filter(|(at, f)| at.elapsed() < CACHE_TTL && f.requested >= sample)
            .map(|(_, f)| f.clone());
        if let Some(mut f) = hit {
            f.posts.truncate(sample);
            return Ok((f, true));
        }
    }
    let fetched = fetch_posts(p, handle, sample, replies).await?;
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if c.len() >= 24 {
        c.clear();
    }
    c.insert(key, (Instant::now(), fetched.clone()));
    Ok((fetched, false))
}

pub(super) fn listing(items: Vec<Value>, limit: usize, offset: usize) -> Value {
    let total = items.len();
    let page: Vec<Value> = items.into_iter().skip(offset).take(limit).collect();
    json!({
        "total": total,
        "returned": page.len(),
        "offset": offset,
        "truncated": offset + page.len() < total,
        "items": page,
    })
}

fn zone(a: &Value) -> Result<Option<FixedOffset>, String> {
    match &a["utc_offset_minutes"] {
        Value::Null => Ok(None),
        v => v
            .as_i64()
            .filter(|m| (-840..=840).contains(m))
            .and_then(|m| FixedOffset::east_opt(m as i32 * 60))
            .map(Some)
            .ok_or_else(|| invalid("utc_offset_minutes must be a whole number from -840 to 840")),
    }
}

fn zone_label(tz: Option<FixedOffset>) -> String {
    match tz {
        Some(tz) => tz.to_string(),
        None => chrono::Local::now().offset().to_string(),
    }
}

fn report(p: Platform, name: &str, posts: &[Post], a: &Value) -> Result<Value, String> {
    let limit = int(a, "limit", 50, 1, 200)? as usize;
    let offset = int(a, "offset", 0, 0, 100_000)? as usize;
    let tz = zone(a)?;
    Ok(match name {
        "summary" => stats::summary(posts, p),
        "posts" => {
            let key = opt_str(a, "sort").unwrap_or_else(|| "created_at".into());
            if !stats::SORT_KEYS.contains(&key.as_str()) {
                return Err(invalid(format!(
                    "sort must be one of {}",
                    stats::SORT_KEYS.join(", ")
                )));
            }
            let mut rows = stats::rows(posts, p);
            stats::sort_rows(
                &mut rows,
                &key,
                opt_str(a, "order").as_deref() == Some("asc"),
            );
            listing(rows, limit, offset)
        }
        "times" => {
            let mut v = match tz {
                Some(tz) => stats::times(posts, &tz),
                None => stats::times(posts, &chrono::Local),
            };
            v["timezone"] = json!(zone_label(tz));
            v
        }
        "media" => listing(stats::by_media(posts), limit, offset),
        "hashtags" => listing(stats::by_hashtag(posts), limit, offset),
        "words" => {
            let mut v = listing(stats::by_word(posts), limit, offset);
            v["note"] = json!("Words are split on spaces and punctuation, so languages written without spaces are counted by phrase.");
            v
        }
        "trend" => {
            let weekly = opt_str(a, "unit").as_deref() == Some("week");
            let buckets = match tz {
                Some(tz) => stats::trend(posts, &tz, weekly),
                None => stats::trend(posts, &chrono::Local, weekly),
            };
            let mut v = listing(buckets, limit, offset);
            v["unit"] = json!(if weekly { "week" } else { "day" });
            v["timezone"] = json!(zone_label(tz));
            v
        }
        "outliers" => stats::outliers(posts, p),
        _ => {
            return Err(invalid(format!(
                "report must be one of {}",
                REPORTS.join(", ")
            )))
        }
    })
}

fn remember(p: Platform, handle: &str, f: &Fetched, cached: bool) {
    if !cached {
        let _ = store::with(history::SCHEMA, |c| {
            history::record(c, p.id(), handle, &f.posts, f.at_ms)
        });
    }
}

pub(super) async fn own_sample(p: Platform, a: &Value) -> Result<(String, Vec<Post>), String> {
    let handle = me_handle(p).await?;
    let (f, cached) = posts_for(p, &handle, a).await?;
    remember(p, &handle, &f, cached);
    Ok((handle, f.posts))
}

fn post_history(p: Platform, a: &Value) -> Result<Value, String> {
    let limit = int(a, "limit", 50, 1, 200)? as usize;
    let offset = int(a, "offset", 0, 0, 100_000)? as usize;
    let result = store::with(history::SCHEMA, |c| match opt_str(a, "post") {
        Some(raw) => history::series(c, p.id(), &history::post_id(p, &raw)),
        None => history::tracked(c, p.id(), limit, offset),
    })?;
    Ok(json!({ "platform": p.id(), "report": "post_history", "result": result }))
}

async fn recommendations(p: Platform, a: &Value) -> Result<Value, String> {
    if opt_str(a, who_key(p)).is_some() {
        return Err(invalid(format!(
            "recommendations are for the signed-in account; call it without {}",
            who_key(p)
        )));
    }
    let own = own_sample(p, a).await;
    let result = match &own {
        Ok((_, posts)) => advice::gather(p, Ok(posts.as_slice())).await,
        Err(e) => advice::gather(p, Err(e.clone())).await,
    };
    Ok(json!({
        "platform": p.id(),
        "report": "recommendations",
        "handle": own.ok().map(|(handle, _)| handle),
        "result": result,
    }))
}

fn compare_periods(p: Platform, f: &Fetched, a: &Value) -> Result<Value, String> {
    let whole = f.stopped.is_none() && f.posts.len() < f.requested;
    Ok(match zone(a)? {
        Some(tz) => {
            let (first, second) = periods::ranges(a, &tz)?;
            periods::compare(&f.posts, p, first, second, whole)
        }
        None => {
            let (first, second) = periods::ranges(a, &chrono::Local)?;
            periods::compare(&f.posts, p, first, second, whole)
        }
    })
}

pub fn report_name(p: Platform, handle: &str, stamp: &str, ext: &str) -> String {
    export_name(p, handle, stamp)
        .replace("-posts-", "-report-")
        .replace(".csv", ext)
}

const NO_POWERPOINT: &str = "PowerPoint is not produced: the report is one HTML file that opens in any browser and prints to PDF, with a Markdown twin.";

async fn make_report(p: Platform, a: &Value) -> Result<Value, String> {
    let me = net(p, "me", json!({})).await?;
    let handle = opt_str(&me, "handle").ok_or_else(|| {
        format!(
            "NOT_LOGGED_IN: {} did not return the signed-in account",
            p.label()
        )
    })?;
    let mut args = a.clone();
    if args["sample"].is_null() {
        args["sample"] = json!(100);
    }
    let mut data = json!({
        "platform": p.label(),
        "handle": handle,
        "generated": chrono::Local::now().format("%Y-%m-%d %H:%M %:z").to_string(),
        "profile": { "name": me["name"], "followers": me["followers"], "following": me["following"], "posts": me["posts"] },
    });
    match posts_for(p, &handle, &args).await {
        Ok((f, cached)) => {
            remember(p, &handle, &f, cached);
            let mut rows = stats::rows(&f.posts, p);
            stats::sort_rows(&mut rows, "engagement", false);
            rows.truncate(report::TOP_POSTS);
            let mut tags = stats::by_hashtag(&f.posts);
            tags.truncate(report::TOP_HASHTAGS);
            let mut times = stats::times(&f.posts, &chrono::Local);
            times["timezone"] = json!(zone_label(None));
            data["summary"] = stats::summary(&f.posts, p);
            data["times"] = times;
            data["top"] = json!(rows);
            data["hashtags"] = json!(tags);
            if let Some(e) = &f.stopped {
                data["posts_note"] =
                    json!(format!("Stopped reading posts early: {}", advice::plain(e)));
            }
            if f.posts.is_empty() {
                data["posts_reason"] =
                    json!("the account has no posts a signed-in session can see.");
            }
        }
        Err(e) => data["posts_reason"] = json!(advice::plain(&e)),
    }
    let screen = Caller::Screen;
    match super::audience::call(
        &screen,
        &format!("{}audience", p.prefix()),
        &json!({ "view": "trend" }),
    )
    .await
    {
        Ok(trend) if trend["total"].as_u64().unwrap_or(0) > 0 => {
            data["audience"] = json!({ "days": trend["days"] })
        }
        Ok(_) => {}
        Err(e) => data["audience_reason"] = json!(advice::plain(&e)),
    }
    match super::monitor::call(&screen, "social_monitors", &json!({ "platform": p.id() })).await {
        Ok(v) => data["monitors"] = v["monitors"].clone(),
        Err(e) => data["monitors_reason"] = json!(advice::plain(&e)),
    }
    let dir = store::dir().map(|d| d.join("reports")).ok_or_else(|| {
        fail(
            "EXPORT_FAILED",
            "the app data folder is not available",
            false,
            "Read the analytics reports one by one instead.",
        )
    })?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let page = dir.join(report_name(p, &handle, &stamp, ".html"));
    let twin = dir.join(report_name(p, &handle, &stamp, ".md"));
    let files = [
        (&page, report::html(&data)),
        (&twin, report::markdown(&data)),
    ];
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        fail(
            "EXPORT_FAILED",
            format!("could not create {}: {e}", dir.display()),
            true,
            "Call the tool again.",
        )
    })?;
    for (path, body) in &files {
        let write = async {
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .await?;
            tokio::io::AsyncWriteExt::write_all(&mut file, body.as_bytes()).await
        };
        write.await.map_err(|e| {
            fail(
                "EXPORT_FAILED",
                format!("could not write {}: {e}", path.display()),
                true,
                "Call the tool again in a second; the file name carries the time.",
            )
        })?;
    }
    let has = |key: &str| data[key].as_array().is_some_and(|l| !l.is_empty());
    Ok(json!({
        "path": page.to_string_lossy(),
        "markdown_path": twin.to_string_lossy(),
        "platform": p.id(),
        "handle": handle,
        "sections": {
            "account": true,
            "followers": data["audience"].is_object(),
            "post_performance": data["summary"]["analyzed"].as_u64().unwrap_or(0) > 0,
            "best_times": data["times"]["cells"].as_array().is_some_and(|l| !l.is_empty()),
            "top_posts": has("top"),
            "hashtags": has("hashtags"),
            "monitors": has("monitors"),
        },
        "sections_note": "A section marked false is in the file with the reason it has no data.",
        "language": "en",
        "powerpoint": false,
        "powerpoint_note": NO_POWERPOINT,
    }))
}

fn envelope(
    p: Platform,
    handle: &str,
    name: &str,
    f: &Fetched,
    cached: bool,
    result: Value,
) -> Value {
    let mut out = json!({
        "platform": p.id(),
        "handle": handle,
        "report": name,
        "sample_requested": f.requested,
        "sample": f.posts.len(),
        "fetched_at": f.at_ms,
        "cached": cached,
        "result": result,
    });
    if f.posts.is_empty() {
        out["empty_reason"] = json!(
            "The account has no posts a signed-in session can see (new, private or empty profile)."
        );
    }
    if let Some(e) = &f.stopped {
        out["partial_reason"] = json!(format!("Stopped reading early: {e}"));
    }
    out
}

async fn analytics(p: Platform, a: &Value) -> Result<Value, String> {
    let name = opt_str(a, "report").unwrap_or_else(|| "summary".into());
    if let Some(list) = networks::reports(p) {
        if !list.contains(&name.as_str()) {
            return Err(invalid(format!(
                "report for {} must be one of {}",
                p.label(),
                list.join(", ")
            )));
        }
        if name == "repos" {
            return repos(p, a).await;
        }
    }
    if !REPORTS.contains(&name.as_str()) {
        return Err(invalid(format!(
            "report must be one of {}",
            REPORTS.join(", ")
        )));
    }
    if name == "post_history" {
        return post_history(p, a);
    }
    if name == "recommendations" {
        return recommendations(p, a).await;
    }
    if name == "compare" {
        let mut handles: Vec<String> = Vec::new();
        for h in strings(a, "handles")? {
            let h = clean_handle(p, &h);
            if !h.is_empty() && !handles.iter().any(|x| x.eq_ignore_ascii_case(&h)) {
                handles.push(h);
            }
        }
        if !(2..=5).contains(&handles.len()) {
            return Err(invalid(
                "compare needs handles with 2 to 5 different accounts",
            ));
        }
        let mut sets = Vec::new();
        let mut notes = Vec::new();
        for (i, h) in handles.iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(PACE).await;
            }
            let (f, _) = posts_for(p, h, a).await?;
            if let Some(e) = &f.stopped {
                notes.push(format!("@{h}: stopped reading early: {e}"));
            }
            sets.push((h.clone(), f.posts));
        }
        let mut compared = stats::compare(&sets, p);
        networks::adjust_compare(p, &mut compared);
        let mut out = json!({
            "platform": p.id(),
            "report": "compare",
            "result": compared,
        });
        if !notes.is_empty() {
            out["partial_reason"] = json!(notes.join("; "));
        }
        return Ok(out);
    }
    let handle = target(p, a).await?;
    let (f, cached) = posts_for(p, &handle, a).await?;
    if opt_str(a, who_key(p)).is_none() {
        remember(p, &handle, &f, cached);
    }
    let mut result = if name == "compare_periods" {
        compare_periods(p, &f, a)?
    } else {
        report(p, &name, &f.posts, a)?
    };
    let kinds: HashMap<String, String> = f
        .posts
        .iter()
        .filter_map(|x| f.kinds.get(&x.id).map(|k| (x.id.clone(), k.clone())))
        .collect();
    networks::adjust(p, &name, &mut result, &kinds);
    Ok(envelope(p, &handle, &name, &f, cached, result))
}

async fn repos(p: Platform, a: &Value) -> Result<Value, String> {
    let limit = int(a, "limit", 50, 1, 200)? as usize;
    let offset = int(a, "offset", 0, 0, 100_000)? as usize;
    let key = opt_str(a, "sort").unwrap_or_else(|| "stars".into());
    if !networks::REPO_SORTS.contains(&key.as_str()) {
        return Err(invalid(format!(
            "sort for report=repos must be one of {}",
            networks::REPO_SORTS.join(", ")
        )));
    }
    let ascending = opt_str(a, "order").as_deref() == Some("asc");
    let handle = target(p, a).await?;
    let mut rows: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut stopped = None;
    let mut capped = false;
    for page in 0..REPO_PAGES {
        if page > 0 {
            tokio::time::sleep(PACE).await;
        }
        let mut args = json!({ who_key(p): handle, "sort": "pushed" });
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let v = match net(p, "repos", args).await {
            Ok(v) => v,
            Err(e) if page > 0 => {
                stopped = Some(e);
                break;
            }
            Err(e) => return Err(e),
        };
        let items = v["items"].as_array().ok_or_else(|| {
            format!(
                "UPSTREAM_CHANGED: {} returned a repository list without items",
                p.label()
            )
        })?;
        rows.extend(items.iter().filter_map(networks::repo_row));
        cursor = opt_str(&v, "next_cursor");
        if cursor.is_none() {
            break;
        }
        capped = page + 1 == REPO_PAGES;
    }
    networks::sort_repos(&mut rows, &key, ascending);
    let totals = networks::repo_totals(&rows);
    let read = rows.len();
    let mut result = listing(rows, limit, offset);
    result["totals"] = totals;
    result["sort"] = json!(key);
    result["order"] = json!(if ascending { "asc" } else { "desc" });
    result["note"] = json!("open_issues is GitHub's open_issues_count, which also counts open pull requests; pushed is the last push in unix seconds.");
    let mut out = json!({
        "platform": p.id(),
        "handle": handle,
        "report": "repos",
        "sample_requested": read,
        "sample": read,
        "fetched_at": store::now_ms(),
        "cached": false,
        "result": result,
    });
    if read == 0 {
        out["empty_reason"] = json!("This user has no public repositories.");
    }
    if let Some(e) = stopped {
        out["partial_reason"] = json!(format!("Stopped reading early: {e}"));
    } else if capped {
        out["partial_reason"] = json!(format!(
            "Only the {read} most recently pushed repositories were read."
        ));
    }
    Ok(out)
}

pub fn export_name(p: Platform, handle: &str, stamp: &str) -> String {
    let safe: String = handle
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    format!("omniget-{}-{safe}-posts-{stamp}.csv", p.id())
}

async fn export(p: Platform, a: &Value) -> Result<Value, String> {
    let handle = target(p, a).await?;
    let (f, _) = posts_for(p, &handle, a).await?;
    let rows = stats::rows(&f.posts, p);
    if rows.is_empty() {
        return Err(fail(
            "NOTHING_TO_EXPORT",
            format!("@{handle} has no posts a signed-in session can see"),
            false,
            "Check the handle, or run the analytics tool with refresh=true.",
        ));
    }
    let dir = dirs::download_dir()
        .or_else(|| store::dir().map(|d| d.join("exports")))
        .ok_or_else(|| {
            fail(
                "EXPORT_FAILED",
                "no Downloads folder on this computer",
                false,
                "Use report=posts and save the rows yourself.",
            )
        })?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let path = dir.join(export_name(p, &handle, &stamp));
    let body = format!("\u{feff}{}", stats::csv(&rows));
    let write = async {
        tokio::fs::create_dir_all(&dir).await?;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await?;
        tokio::io::AsyncWriteExt::write_all(&mut file, body.as_bytes()).await
    };
    write.await.map_err(|e| {
        fail(
            "EXPORT_FAILED",
            format!("could not write {}: {e}", path.display()),
            true,
            "Call the export again in a second; the file name carries the time.",
        )
    })?;
    Ok(json!({
        "path": path.to_string_lossy(),
        "rows": rows.len(),
        "columns": stats::CSV_COLUMNS,
        "handle": handle,
        "platform": p.id(),
    }))
}

pub fn owner_in_link(post: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?:x|twitter)\.com/([A-Za-z0-9_]{1,15})/status(?:es)?/\d+")
        .expect("static regex");
    re.captures(post)
        .map(|c| c[1].to_string())
        .filter(|h| h != "i")
}

fn entry_of_author(author: &Value) -> Value {
    json!({
        "id": author["id"],
        "handle": author["handle"],
        "name": author["name"],
        "avatar": author["avatar"],
        "verified": author["verified"],
    })
}

pub fn entries_from_posts(items: &[Value], reply_to: Option<&str>) -> Vec<Value> {
    items
        .iter()
        .filter(|p| reply_to.is_none_or(|id| p["reply_to"].as_str() == Some(id)))
        .filter(|p| p["reposted_by"].is_null())
        .map(|p| {
            let mut e = entry_of_author(&p["author"]);
            e["post"] = json!({ "id": p["id"], "url": p["url"], "text": p["text"], "created_at": p["created_at"] });
            e
        })
        .collect()
}

pub fn entries_from_comments(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .map(|c| {
            let mut e = entry_of_author(&c["author"]);
            e["comment"] =
                json!({ "id": c["id"], "text": c["text"], "created_at": c["created_at"] });
            e
        })
        .collect()
}

const X_KINDS: &[&str] = &["reposts", "quotes", "replies", "likes"];

async fn engagement_page(
    p: Platform,
    post: &str,
    kind: &str,
    cursor: Option<String>,
) -> Result<(Vec<Value>, Option<String>), String> {
    let items_of = |v: &Value| -> Result<Vec<Value>, String> {
        v["items"].as_array().cloned().ok_or_else(|| {
            format!(
                "UPSTREAM_CHANGED: {} returned an engagement list without items",
                p.label()
            )
        })
    };
    if p != Platform::X {
        let mut args = json!({ "post": post });
        let first = cursor.is_none();
        if let Some(c) = cursor {
            args["cursor"] = json!(c);
        }
        let v = net(p, if first { "post" } else { "comments" }, args).await?;
        let page = if first { &v["comments"] } else { &v };
        return Ok((
            entries_from_comments(&items_of(page)?),
            opt_str(page, "next_cursor"),
        ));
    }
    let id = x::tweet_id(post)
        .ok_or_else(|| invalid("post must be a post id or an x.com/.../status/... link"))?;
    match kind {
        "likes" | "reposts" => {
            let (users, next) = x::engagement_users(kind == "likes", &id, cursor).await?;
            Ok((users.into_iter().map(|u| json!(u)).collect(), next))
        }
        "quotes" => {
            let mut args = json!({ "query": x::quotes_query(&id), "mode": "latest" });
            if let Some(c) = cursor {
                args["cursor"] = json!(c);
            }
            let v = net(p, "search", args).await?;
            Ok((
                entries_from_posts(&items_of(&v)?, None),
                opt_str(&v, "next_cursor"),
            ))
        }
        _ => {
            let mut args = json!({ "post": id });
            if let Some(c) = cursor {
                args["cursor"] = json!(c);
            }
            let v = net(p, "post", args).await?;
            let page = &v["replies"];
            let entries = entries_from_posts(&items_of(page)?, Some(&id));
            let next = opt_str(page, "next_cursor").filter(|_| !entries.is_empty());
            Ok((entries, next))
        }
    }
}

const LIKES_HIDDEN: &str = "X shows who liked a post only to the post's author (likes became private in 2024). The like count is still public in the post itself.";

async fn x_engagement(a: &Value) -> Result<Value, String> {
    let post = super::args::req_str(a, "post").map_err(|_| invalid("post is required"))?;
    let kind = opt_str(a, "kind").unwrap_or_default();
    if !X_KINDS.contains(&kind.as_str()) {
        return Err(invalid(format!(
            "kind must be one of {}",
            X_KINDS.join(", ")
        )));
    }
    let hidden = |reason: &str| json!({ "post": post, "kind": kind, "available": false, "count": 0, "items": [], "next_cursor": Value::Null, "reason": reason });
    if kind == "likes" {
        if let Some(owner) = owner_in_link(&post) {
            if !owner.eq_ignore_ascii_case(&x::me_handle().await?) {
                return Ok(hidden(LIKES_HIDDEN));
            }
        }
    }
    let (items, next) = engagement_page(Platform::X, &post, &kind, opt_str(a, "cursor")).await?;
    let mut out = json!({ "post": post, "kind": kind, "available": true, "count": items.len(), "items": items, "next_cursor": next });
    if items.is_empty() && opt_str(a, "cursor").is_none() {
        out["empty_reason"] = json!(if kind == "likes" {
            LIKES_HIDDEN
        } else {
            "Nobody did this to the post yet, or X no longer lists it."
        });
    }
    Ok(out)
}

async fn collect(
    p: Platform,
    post: &str,
    kind: &str,
    cap: usize,
) -> Result<(Vec<Value>, bool), String> {
    let mut out: Vec<Value> = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor: Option<String> = None;
    for page in 0..(cap / 10 + 3) {
        if page > 0 {
            tokio::time::sleep(PACE).await;
        }
        let (items, next) = engagement_page(p, post, kind, cursor).await?;
        let before = seen.len();
        for item in items {
            let handle = draw::normalize(item["handle"].as_str().unwrap_or_default());
            if !handle.is_empty() && seen.insert(handle) {
                out.push(item);
            }
        }
        cursor = next;
        if cursor.is_none() || seen.len() == before {
            return Ok((out, false));
        }
        if out.len() >= cap {
            break;
        }
    }
    Ok((out, true))
}

async fn social_draw(p: Platform, a: &Value) -> Result<Value, String> {
    if a["winners"].is_null() {
        return Err(invalid("winners is required"));
    }
    let winners = int(a, "winners", 1, 1, 100)? as usize;
    let runner_ups = int(a, "runner_ups", 0, 0, 100)? as usize;
    let cap = int(a, "max_pool", 500, 20, 2000)? as usize;
    let must_follow = a["must_follow"].as_bool().unwrap_or(false);
    let must_repost = a["must_have_reposted"].as_bool().unwrap_or(false);
    let post = opt_str(a, "post");
    let given = strings(a, "handles")?;
    let exclude = strings(a, "exclude")?;
    if must_follow && p == Platform::Tiktok {
        return Err(fail(
            "CONDITION_UNAVAILABLE",
            "TikTok does not tell a web session whether another account follows you",
            false,
            "Run the draw without must_follow, or check the winners' follow by hand.",
        ));
    }
    if must_repost && (p != Platform::X || post.is_none()) {
        return Err(invalid("must_have_reposted works on X only and needs post"));
    }
    let mut follows: HashMap<String, bool> = HashMap::new();
    let mut note_follow = |items: &[Value]| {
        for u in items {
            if let Some(f) = u["follows_me"].as_bool() {
                follows.insert(draw::normalize(u["handle"].as_str().unwrap_or_default()), f);
            }
        }
    };
    let (candidates, truncated, source) = if !given.is_empty() {
        (
            given.iter().map(|h| clean_handle(p, h)).collect::<Vec<_>>(),
            false,
            "handles".to_string(),
        )
    } else {
        let post = post
            .as_deref()
            .ok_or_else(|| invalid("pass handles, or post with source"))?;
        let default = if p == Platform::X {
            "reposts"
        } else {
            "comments"
        };
        let source = opt_str(a, "source").unwrap_or_else(|| default.into());
        let allowed = if p == Platform::X {
            X_KINDS.contains(&source.as_str())
        } else {
            source == "comments"
        };
        if !allowed {
            return Err(invalid(if p == Platform::X {
                "on X source must be reposts, quotes, replies or likes"
            } else {
                "on Instagram and TikTok the only source is comments"
            }));
        }
        let (items, truncated) = collect(p, post, &source, cap).await?;
        note_follow(&items);
        (
            items
                .iter()
                .map(|u| u["handle"].as_str().unwrap_or_default().to_string())
                .collect(),
            truncated,
            source,
        )
    };
    let mut reposters: Option<HashSet<String>> = None;
    let mut reposters_truncated = false;
    if must_repost && source != "reposts" {
        let (items, cut) = collect(p, post.as_deref().unwrap_or_default(), "reposts", 2000).await?;
        note_follow(&items);
        reposters_truncated = cut;
        reposters = Some(
            items
                .iter()
                .map(|u| draw::normalize(u["handle"].as_str().unwrap_or_default()))
                .collect(),
        );
    }
    let pool = draw::pool(&candidates, &exclude);
    if pool.is_empty() {
        return Err(fail(
            "EMPTY_POOL",
            "nobody is left to draw from after removing duplicates and the exclude list",
            false,
            "Check the post, the source or the handles list.",
        ));
    }
    let seed = opt_str(a, "seed").unwrap_or_else(|| format!("{:032x}", rand::random::<u128>()));
    let ordered = draw::order(&seed, &pool);
    let wanted = winners + runner_ups;
    let mut eligible: Vec<String> = Vec::new();
    let mut disqualified: Vec<Value> = Vec::new();
    let mut checked = 0usize;
    let mut lookups = 0usize;
    let mut stopped: Option<&str> = None;
    for handle in &ordered {
        if eligible.len() >= wanted {
            break;
        }
        checked += 1;
        if reposters.as_ref().is_some_and(|r| !r.contains(handle)) {
            disqualified.push(json!({ "handle": handle, "reason": "did not repost the post" }));
            continue;
        }
        if must_follow {
            let known = match follows.get(handle) {
                Some(f) => Some(*f),
                None => {
                    if lookups >= MAX_LOOKUPS {
                        stopped = Some("Stopped after 80 profile lookups to keep a human pace; run again with the same seed and the disqualified handles in exclude to continue.");
                        checked -= 1;
                        break;
                    }
                    if lookups > 0 {
                        tokio::time::sleep(PACE).await;
                    }
                    lookups += 1;
                    match net(p, "profile", json!({ who_key(p): handle })).await {
                        Ok(profile) => profile["follows_me"].as_bool(),
                        Err(e) if e.starts_with("NOT_FOUND") => {
                            disqualified
                                .push(json!({ "handle": handle, "reason": "account not found" }));
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                }
            };
            match known {
                Some(true) => {}
                Some(false) => {
                    disqualified
                        .push(json!({ "handle": handle, "reason": "does not follow the account" }));
                    continue;
                }
                None => {
                    return Err(fail(
                        "CONDITION_UNAVAILABLE",
                        format!(
                            "{} did not say whether @{handle} follows the signed-in account",
                            p.label()
                        ),
                        false,
                        "Run the draw without must_follow and check the winners by hand.",
                    ))
                }
            }
        }
        eligible.push(handle.clone());
    }
    let placed = |list: Vec<String>| -> Vec<Value> {
        list.into_iter()
            .map(|h| json!({ "rank": draw::rank(&seed, &h), "handle": h }))
            .collect()
    };
    let filled = eligible.len() >= wanted;
    let (first, rest) = draw::split(eligible, winners);
    let mut out = json!({
        "platform": p.id(),
        "seed": seed,
        "method": draw::METHOD,
        "source": source,
        "post": post,
        "pool": pool.len(),
        "pool_truncated": truncated,
        "checked": checked,
        "requested": { "winners": winners, "runner_ups": runner_ups },
        "filled": filled,
        "winners": placed(first),
        "runner_ups": placed(rest),
        "disqualified": disqualified,
        "conditions": { "must_follow": must_follow, "must_have_reposted": must_repost, "excluded": exclude.len() },
    });
    let mut notes: Vec<&str> = Vec::new();
    if truncated {
        notes.push("The engagement list was longer than max_pool, so the draw covers only the accounts read; raise max_pool to cover everyone.");
    }
    if reposters_truncated {
        notes.push("The repost list was longer than 2000 accounts; someone who reposted may be marked as not having reposted.");
    }
    if let Some(s) = stopped {
        notes.push(s);
    } else if !filled {
        notes.push("Fewer eligible accounts than places asked for.");
    }
    if source == "likes" {
        notes.push(LIKES_HIDDEN);
    }
    if !notes.is_empty() {
        out["notes"] = json!(notes);
    }
    if a["export"].as_bool() == Some(true) {
        let dir = store::dir().map(|d| d.join("exports")).ok_or_else(|| {
            fail(
                "EXPORT_FAILED",
                "the app data folder is not available",
                false,
                "Run the draw without export and copy the winners from the result.",
            )
        })?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let path = dir.join(format!("{}-draw-{stamp}.csv", p.id()));
        let body = format!("\u{feff}{}", draw::csv(&out));
        let write = async {
            tokio::fs::create_dir_all(&dir).await?;
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .await?;
            tokio::io::AsyncWriteExt::write_all(&mut file, body.as_bytes()).await
        };
        write.await.map_err(|e| {
            fail(
                "EXPORT_FAILED",
                format!("could not write {}: {e}", path.display()),
                true,
                "Run the draw again with the same seed in a second; the file name carries the time.",
            )
        })?;
        out["csv_path"] = json!(path.to_string_lossy());
    }
    Ok(out)
}

pub async fn call(_caller: &Caller, tool: &str, a: &Value) -> Result<Value, String> {
    let spec_platform = super::tools::find(tool).map(|s| s.platform);
    match (tool, spec_platform) {
        ("x_engagement", _) => x_engagement(a).await,
        ("social_draw", _) => social_draw(platform_arg(a)?, a).await,
        ("social_digest", _) => digest::tool(platform_arg(a)?, a).await,
        ("social_notifications", _) => alerts::tool(platform_arg(a)?, a),
        (name, Some(p)) if name.ends_with("_report") => make_report(p, a).await,
        (name, Some(p)) if name.ends_with("_analytics") => analytics(p, a).await,
        (name, Some(p)) if name.ends_with("_analytics_export") => export(p, a).await,
        _ => Err(format!("UNKNOWN_TOOL: {tool}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::social::model::Author;
    use crate::commands::social::tools;

    fn post(id: &str, likes: u64, ts: i64) -> Post {
        Post {
            id: id.into(),
            created_at: Some(ts),
            stats: crate::commands::social::model::Stats {
                likes: Some(likes),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn specs_are_read_only_valid_and_named_after_their_network() {
        for s in SPECS {
            assert!(!s.write, "{}", s.name);
            assert_eq!(s.route, Route::Insights);
            let schema = tools::input_schema(s);
            for r in s.required {
                assert!(
                    schema["properties"][*r].is_object(),
                    "{} requires {r}",
                    s.name
                );
            }
            assert!(
                s.name.starts_with("social_") || s.name.starts_with(s.platform.prefix()),
                "{}",
                s.name
            );
            assert!(
                s.description.matches(". ").count() <= 1,
                "{} is longer than two sentences",
                s.name
            );
        }
        let x = tools::input_schema(&SPECS[0]);
        assert!(x["properties"]["handle"].is_object());
        assert_eq!(
            x["properties"]["report"]["enum"].as_array().map(Vec::len),
            Some(REPORTS.len())
        );
        let ig = tools::input_schema(&SPECS[1]);
        assert!(ig["properties"]["username"].is_object());
    }

    #[test]
    fn every_network_has_analytics_and_export_and_github_has_repos() {
        for prefix in ["x_", "ig_", "tt_", "rd_", "pin_", "gh_"] {
            for name in ["analytics", "analytics_export"] {
                let spec = tools::find(&format!("{prefix}{name}")).expect(name);
                assert_eq!(spec.route, Route::Insights);
                assert_eq!(spec.platform.prefix(), prefix);
            }
        }
        let gh = tools::input_schema(tools::find("gh_analytics").unwrap());
        let reports: Vec<&str> = gh["properties"]["report"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(reports, networks::reports(Platform::Github).unwrap());
        for key in networks::REPO_SORTS {
            assert!(gh["properties"]["sort"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!(key)));
        }
        assert!(gh["properties"]["username"].is_object());
        let rd = tools::input_schema(tools::find("rd_analytics").unwrap());
        assert!(rd["properties"]["username"].is_object());
        assert!(!rd["properties"]["report"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("repos")));
    }

    #[tokio::test]
    async fn github_rejects_reports_it_cannot_answer_before_the_network() {
        let caller = Caller::Screen;
        for bad in [
            json!({ "report": "outliers" }),
            json!({ "report": "hashtags" }),
            json!({ "report": "repos", "sort": "likes" }),
            json!({ "report": "repos", "limit": 0 }),
        ] {
            let e = call(&caller, "gh_analytics", &bad).await.unwrap_err();
            assert!(e.starts_with("INVALID_ARGUMENTS"), "{e}");
        }
        let e = call(&caller, "rd_analytics", &json!({ "report": "repos" }))
            .await
            .unwrap_err();
        assert!(e.starts_with("INVALID_ARGUMENTS"), "{e}");
    }

    #[test]
    fn arguments_are_validated_with_a_next_step() {
        assert_eq!(int(&json!({}), "sample", 50, 1, 200), Ok(50));
        assert_eq!(
            int(&json!({ "sample": 200 }), "sample", 50, 1, 200),
            Ok(200)
        );
        let e = int(&json!({ "sample": 201 }), "sample", 50, 1, 200).unwrap_err();
        let (code, body) = e.split_once(": ").unwrap();
        assert_eq!(code, "INVALID_ARGUMENTS");
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["retryable"], false);
        assert!(body["next_step"].is_string());
        assert_eq!(
            strings(&json!({ "h": ["a", " ", "b "] }), "h"),
            Ok(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(
            strings(&json!({ "h": "a, b\nc" }), "h").map(|v| v.len()),
            Ok(3)
        );
        assert!(strings(&json!({ "h": 3 }), "h").is_err());
    }

    #[test]
    fn reports_page_their_rows_and_reject_unknown_names() {
        let posts: Vec<Post> = (0..5)
            .map(|i| post(&i.to_string(), i * 10, 1_700_000_000 + i as i64))
            .collect();
        let v = report(
            Platform::X,
            "posts",
            &posts,
            &json!({ "sort": "likes", "limit": 2, "offset": 1 }),
        )
        .unwrap();
        assert_eq!(v["total"], 5);
        assert_eq!(v["returned"], 2);
        assert_eq!(v["truncated"], true);
        assert_eq!(v["items"][0]["id"], "3");
        let last = report(
            Platform::X,
            "posts",
            &posts,
            &json!({ "sort": "likes", "order": "asc", "offset": 3 }),
        )
        .unwrap();
        assert_eq!(last["truncated"], false);
        assert_eq!(last["items"][1]["id"], "4");
        assert!(
            report(Platform::X, "posts", &posts, &json!({ "sort": "nope" }))
                .unwrap_err()
                .starts_with("INVALID_ARGUMENTS")
        );
        assert!(report(Platform::X, "nope", &posts, &json!({})).is_err());
        let t = report(
            Platform::X,
            "times",
            &posts,
            &json!({ "utc_offset_minutes": -180 }),
        )
        .unwrap();
        assert_eq!(t["timezone"], "-03:00");
        assert_eq!(
            report(
                Platform::X,
                "trend",
                &posts,
                &json!({ "unit": "week", "utc_offset_minutes": 0 })
            )
            .unwrap()["unit"],
            "week"
        );
        assert!(report(
            Platform::X,
            "times",
            &posts,
            &json!({ "utc_offset_minutes": 9000 })
        )
        .is_err());
    }

    #[test]
    fn an_empty_sample_explains_itself() {
        let f = Fetched {
            posts: Vec::new(),
            requested: 50,
            at_ms: 1,
            stopped: Some("RATE_LIMITED: wait".into()),
            kinds: Default::default(),
        };
        let v = envelope(Platform::Instagram, "ana", "summary", &f, false, json!({}));
        assert_eq!(v["sample"], 0);
        assert!(v["empty_reason"].is_string());
        assert!(v["partial_reason"]
            .as_str()
            .unwrap()
            .contains("RATE_LIMITED"));
    }

    #[test]
    fn on_x_other_peoples_posts_in_a_conversation_are_left_out() {
        let mut theirs = post("1", 1, 0);
        theirs.author = Author {
            handle: "Bob".into(),
            ..Default::default()
        };
        assert!(!belongs(Platform::X, "ana", &theirs));
        assert!(belongs(Platform::X, "bob", &theirs));
        theirs.reposted_by = Some(Author {
            handle: "ana".into(),
            ..Default::default()
        });
        assert!(belongs(Platform::X, "ana", &theirs));
        assert!(belongs(Platform::Instagram, "ana", &post("2", 1, 0)));
    }

    #[test]
    fn the_post_owner_is_read_from_a_link_only() {
        assert_eq!(
            owner_in_link("https://x.com/Ana_1/status/123?s=20").as_deref(),
            Some("Ana_1")
        );
        assert_eq!(
            owner_in_link("https://twitter.com/bob/statuses/9").as_deref(),
            Some("bob")
        );
        assert_eq!(owner_in_link("https://x.com/i/status/123"), None);
        assert_eq!(owner_in_link("123"), None);
    }

    #[test]
    fn engagement_entries_keep_the_author_and_only_direct_replies() {
        let items = vec![
            json!({ "id": "2", "url": "u2", "text": "direct", "reply_to": "1", "author": { "id": "7", "handle": "bob", "name": "Bob", "verified": false } }),
            json!({ "id": "3", "url": "u3", "text": "nested", "reply_to": "2", "author": { "id": "8", "handle": "eve", "name": "Eve", "verified": true } }),
        ];
        let replies = entries_from_posts(&items, Some("1"));
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["handle"], "bob");
        assert_eq!(replies[0]["post"]["text"], "direct");
        assert_eq!(entries_from_posts(&items, None).len(), 2);
        let comments = entries_from_comments(&[
            json!({ "id": "c1", "text": "me!", "author": { "id": "9", "handle": "dora", "name": "Dora", "verified": false } }),
        ]);
        assert_eq!(comments[0]["handle"], "dora");
        assert_eq!(comments[0]["comment"]["id"], "c1");
    }

    #[test]
    fn export_file_names_are_safe() {
        assert_eq!(
            export_name(Platform::Instagram, "ana.dev/../x", "20261111-101500"),
            "omniget-instagram-ana.dev_.._x-posts-20261111-101500.csv"
        );
    }

    #[tokio::test]
    async fn the_tool_rejects_bad_arguments_before_touching_the_network() {
        let caller = Caller::Screen;
        let e = call(&caller, "x_analytics", &json!({ "report": "nope" }))
            .await
            .unwrap_err();
        assert!(e.starts_with("INVALID_ARGUMENTS"));
        let e = call(
            &caller,
            "ig_analytics",
            &json!({ "report": "compare", "handles": ["a", "A"] }),
        )
        .await
        .unwrap_err();
        assert!(e.contains("2 to 5"));
        let e = call(
            &caller,
            "x_engagement",
            &json!({ "post": "1", "kind": "shares" }),
        )
        .await
        .unwrap_err();
        assert!(e.starts_with("INVALID_ARGUMENTS"));
        let e = call(
            &caller,
            "social_draw",
            &json!({ "platform": "tiktok", "winners": 1, "must_follow": true, "handles": ["a"] }),
        )
        .await
        .unwrap_err();
        assert!(e.starts_with("CONDITION_UNAVAILABLE"));
        let e = call(
            &caller,
            "social_draw",
            &json!({ "platform": "x", "winners": 0, "handles": ["a"] }),
        )
        .await
        .unwrap_err();
        assert!(e.starts_with("INVALID_ARGUMENTS"));
        let e = call(&caller, "social_draw", &json!({ "platform": "instagram", "winners": 1, "handles": ["a"], "must_have_reposted": true })).await.unwrap_err();
        assert!(e.starts_with("INVALID_ARGUMENTS"));
        assert!(call(&caller, "x_nope", &json!({}))
            .await
            .unwrap_err()
            .starts_with("UNKNOWN_TOOL"));
    }

    #[tokio::test]
    async fn the_second_wave_tools_reject_bad_arguments_before_any_read() {
        let caller = Caller::Screen;
        let e = call(
            &caller,
            "x_analytics",
            &json!({ "report": "recommendations", "handle": "bob" }),
        )
        .await
        .unwrap_err();
        assert!(e.starts_with("INVALID_ARGUMENTS") && e.contains("signed-in"));
        let e = call(
            &caller,
            "social_digest",
            &json!({ "platform": "x", "mark": "yes" }),
        )
        .await
        .unwrap_err();
        assert!(e.starts_with("INVALID_ARGUMENTS"));
        let e = call(
            &caller,
            "social_notifications",
            &json!({ "platform": "x", "op": "toggle" }),
        )
        .await
        .unwrap_err();
        assert!(e.starts_with("INVALID_ARGUMENTS"));
        assert!(call(&caller, "social_digest", &json!({}))
            .await
            .unwrap_err()
            .starts_with("INVALID_ARGUMENTS"));
        assert_eq!(
            report_name(Platform::Tiktok, "ana/../x", "20261111-101500", ".html"),
            "omniget-tiktok-ana_.._x-report-20261111-101500.html"
        );
        for name in [
            "x_report",
            "ig_report",
            "tt_report",
            "social_digest",
            "social_notifications",
        ] {
            assert!(tools::find(name).is_some_and(|s| !s.write), "{name}");
        }
    }

    #[tokio::test]
    async fn a_draw_from_a_list_of_handles_is_reproducible_end_to_end() {
        let caller = Caller::Screen;
        let args = json!({ "platform": "x", "winners": 2, "runner_ups": 1, "seed": "launch-2026", "handles": ["@Ana", "bob", "carl", "dora", "ana", "eve"], "exclude": ["eve"] });
        let a = call(&caller, "social_draw", &args).await.unwrap();
        let b = call(&caller, "social_draw", &args).await.unwrap();
        assert_eq!(a, b);
        assert_eq!(a["pool"], 4);
        assert_eq!(a["winners"].as_array().map(Vec::len), Some(2));
        assert_eq!(a["runner_ups"].as_array().map(Vec::len), Some(1));
        assert_eq!(a["filled"], true);
        assert_eq!(a["seed"], "launch-2026");
        let first = a["winners"][0]["handle"].as_str().unwrap();
        assert_eq!(a["winners"][0]["rank"], draw::rank("launch-2026", first));
        assert!(a["winners"][0]["rank"].as_str() < a["winners"][1]["rank"].as_str());
        let fresh = call(
            &caller,
            "social_draw",
            &json!({ "platform": "x", "winners": 5, "handles": ["a", "b"] }),
        )
        .await
        .unwrap();
        assert_eq!(fresh["filled"], false);
        assert_eq!(fresh["seed"].as_str().map(str::len), Some(32));
        let empty = call(
            &caller,
            "social_draw",
            &json!({ "platform": "x", "winners": 1, "handles": ["a"], "exclude": ["A"] }),
        )
        .await
        .unwrap_err();
        assert!(empty.starts_with("EMPTY_POOL"));
    }
}
