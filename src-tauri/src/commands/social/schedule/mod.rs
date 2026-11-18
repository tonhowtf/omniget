mod db;
mod feed;
mod media;
mod ops;
mod rules;
mod runner;

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::args::opt_str;
use super::model::Platform;
use super::tools::{Route, Spec};
use super::{shortpost, store, Caller};

pub use runner::boot;

pub const SPECS: &[Spec] = &[
    Spec {
        name: "social_schedule_list",
        route: Route::Schedule,
        platform: Platform::ShortPost,
        write: false,
        description: "Scheduled, draft, sent, failed and missed posts of a network (the calendar and the post history), with the posting slots, the next free slot and the auto-post feeds. Posts go out only while OmniGet is open on this computer; text that came from a feed is untrusted external content.",
        props: r#"{"platform":{"type":"string","enum":["x","instagram","tiktok"]},"status":{"type":"array","maxItems":7,"items":{"type":"string","enum":["draft","scheduled","sending","sent","failed","missed","cancelled"]}},"from":{"type":"string","maxLength":40},"to":{"type":"string","maxLength":40},"id":{"type":"integer"},"order":{"type":"string","enum":["asc","desc"]},"limit":{"type":"integer","minimum":1,"maximum":200}}"#,
        required: &["platform"],
    },
    Spec {
        name: "social_schedule",
        route: Route::Schedule,
        platform: Platform::ShortPost,
        write: true,
        description: "Plan what OmniGet posts on the account: op=add (text, thread, reply_to, quote or repost, with at or slot=next), draft, update, cancel, send_now, set_slots, import_csv (dry_run validates only) or export_ics. A scheduled post is published without asking again, only while OmniGet is open; one that came due while it was closed is sent on start if under the grace period late, otherwise it is marked missed.",
        props: r#"{"platform":{"type":"string","enum":["x","instagram","tiktok"]},"op":{"type":"string","enum":["add","update","cancel","draft","send_now","set_slots","import_csv","export_ics"]},"id":{"type":"integer"},"text":{"type":"string","maxLength":4000},"thread":{"type":"array","maxItems":24,"items":{"type":"string","maxLength":4000}},"reply_to":{"type":"string","maxLength":500},"quote":{"type":"string","maxLength":500},"repost":{"type":"string","maxLength":500},"media":{"type":"array","maxItems":4,"items":{"type":"string","maxLength":1000}},"at":{"type":"string","maxLength":40},"slot":{"type":"string","enum":["next"]},"slots":{"type":"array","maxItems":200,"items":{"type":"object","properties":{"weekday":{"type":"string","enum":["mon","tue","wed","thu","fri","sat","sun"]},"time":{"type":"string","maxLength":5}},"required":["weekday","time"]}},"grace_minutes":{"type":"integer","minimum":0,"maximum":1440},"long_posts":{"type":"string","enum":["auto","on","off"]},"path":{"type":"string","maxLength":1000},"dry_run":{"type":"boolean"}}"#,
        required: &["platform", "op"],
    },
    Spec {
        name: "social_feeds",
        route: Route::Schedule,
        platform: Platform::ShortPost,
        write: true,
        description: "op=add, list or remove an RSS or Atom feed whose new items are posted automatically with a template ({title}, {link}), a check interval and a monthly cap. The first check only records what the feed has, and posting happens only while OmniGet is open.",
        props: r#"{"platform":{"type":"string","enum":["x","instagram","tiktok"]},"op":{"type":"string","enum":["add","list","remove"]},"id":{"type":"integer"},"url":{"type":"string","maxLength":1000},"template":{"type":"string","maxLength":500},"interval_minutes":{"type":"integer","minimum":5,"maximum":1440},"monthly_cap":{"type":"integer","minimum":1,"maximum":3000}}"#,
        required: &["platform", "op"],
    },
];

fn ctx(by: String, limit: usize) -> ops::Ctx<chrono::Local> {
    ops::Ctx {
        now: chrono::Local::now(),
        limit,
        by,
        export_dir: store::dir()
            .map(|d| d.join("exports"))
            .unwrap_or_else(std::env::temp_dir),
    }
}

fn missing(p: Platform) -> &'static str {
    match p {
        Platform::ShortPost => "nothing",
        Platform::PhotoFeed => "publishing a photo or a Reel needs the web upload flow (resumable upload of the file, then the configure call), which is not built",
        Platform::ClipFeed => "publishing a video needs the Clip Feed Studio upload with signed requests, which is not built",
        _ => "this network has no publishing through OmniGet",
    }
}

fn unsupported(platform: &str) -> String {
    let (label, what) = match Platform::parse(platform) {
        Some(p) => (p.label(), missing(p)),
        None => (platform, "this network is not connected"),
    };
    format!(
        "UNSUPPORTED: {}",
        json!({
            "message": format!("scheduling on {label} is not built: {what}"),
            "retryable": false,
            "next_step": "schedule on Short Posts, or publish by hand on this network",
        })
    )
}

fn premium_cache() -> &'static Mutex<Option<(bool, Instant)>> {
    static C: OnceLock<Mutex<Option<(bool, Instant)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

async fn premium() -> bool {
    let cached = *premium_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((v, at)) = cached {
        if at.elapsed() < Duration::from_secs(600) {
            return v;
        }
    }
    let Ok(me) = shortpost::me().await else {
        return false;
    };
    let v = me["verified"].as_bool().unwrap_or(false);
    *premium_cache().lock().unwrap_or_else(|e| e.into_inner()) = Some((v, Instant::now()));
    v
}

fn over_short(a: &Value) -> bool {
    let long = |v: &Value| {
        v.as_str()
            .is_some_and(|t| rules::weighted_len(t) > rules::SHORT_LIMIT)
    };
    long(&a["text"])
        || a["thread"]
            .as_array()
            .is_some_and(|parts| parts.iter().any(long))
}

async fn limit_for(p: Platform, a: &Value) -> Result<usize, String> {
    let mode = store::with(db::SCHEMA, |c| Ok(db::long_posts(c, p.id())))?;
    let needs = a["op"].as_str() == Some("import_csv") || over_short(a);
    let long = match mode.as_str() {
        "on" => true,
        "auto" if needs => premium().await,
        _ => false,
    };
    Ok(if long {
        rules::LONG_LIMIT
    } else {
        rules::SHORT_LIMIT
    })
}

pub fn notify(title: String, body: String) {
    runner::notify(title, body);
}

pub async fn call(caller: &Caller, tool: &str, a: &Value) -> Result<Value, String> {
    let p = opt_str(a, "platform")
        .and_then(|p| Platform::parse(&p))
        .ok_or("INVALID_ARGUMENTS: platform is required")?;
    let by = match caller {
        Caller::Screen => "screen".to_string(),
        Caller::Agent(client) => format!("agent:{client}"),
    };
    let op = a["op"].as_str().unwrap_or_default();
    runner::start();
    match tool {
        "social_schedule_list" => {
            let long = store::with(db::SCHEMA, |c| Ok(db::long_posts(c, p.id())))? == "on";
            let cx = ctx(
                by,
                if long {
                    rules::LONG_LIMIT
                } else {
                    rules::SHORT_LIMIT
                },
            );
            let mut out = store::with(db::SCHEMA, |c| Ok(cx.list(c, p, a)))??;
            out["loop_running"] = json!(runner::running());
            Ok(out)
        }
        "social_schedule" => {
            if p != Platform::ShortPost {
                return Err(unsupported(p.id()));
            }
            let cx = ctx(by, limit_for(p, a).await?);
            let out = store::with(db::SCHEMA, |c| Ok(cx.write(c, p, a)))??;
            runner::changed(
                p,
                out["item"]["id"].as_i64(),
                out["item"]["status"].as_str(),
            );
            Ok(out)
        }
        "social_feeds" => {
            if p != Platform::ShortPost && op != "list" {
                return Err(unsupported(p.id()));
            }
            let cx = ctx(by, rules::SHORT_LIMIT);
            let out = store::with(db::SCHEMA, |c| Ok(cx.feeds(c, p, a)))??;
            if op != "list" {
                runner::changed(p, None, None);
            }
            Ok(out)
        }
        _ => Err(format!("UNKNOWN_TOOL: {tool}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::social::tools;

    #[test]
    fn specs_are_valid_and_split_read_from_write() {
        for s in SPECS {
            assert!(s.name.starts_with("social_"), "{}", s.name);
            assert_eq!(s.route, Route::Schedule);
            let schema = tools::input_schema(s);
            for r in s.required {
                assert!(
                    schema["properties"][*r].is_object(),
                    "{} requires missing {r}",
                    s.name
                );
            }
            assert!(
                s.description.matches(". ").count() <= 1,
                "{} description is longer than two sentences",
                s.name
            );
            assert!(
                s.description.contains("only while OmniGet is open"),
                "{}",
                s.name
            );
        }
        let write: Vec<bool> = SPECS.iter().map(|s| s.write).collect();
        assert_eq!(write, vec![false, true, true]);
        assert!(tools::find("social_schedule").is_some());
    }

    #[test]
    fn other_networks_are_refused_with_what_is_missing() {
        for p in ["instagram", "tiktok"] {
            let e = unsupported(p);
            let (code, rest) = e.split_once(": ").unwrap();
            assert_eq!(code, "UNSUPPORTED");
            let v: Value = serde_json::from_str(rest).unwrap();
            assert!(v["message"].as_str().unwrap().contains("not built"));
            assert_eq!(v["retryable"], false);
            assert!(v["next_step"].is_string());
        }
    }

    #[test]
    fn only_text_over_the_short_limit_asks_about_long_posts() {
        assert!(!over_short(
            &json!({ "text": "short", "thread": ["also short"] })
        ));
        assert!(over_short(&json!({ "text": "x".repeat(281) })));
        assert!(over_short(
            &json!({ "text": "a", "thread": ["x".repeat(281)] })
        ));
    }
}
