use std::collections::HashSet;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Notify;

use super::super::model::Platform;
use super::super::store::{self, now_ms};
use super::super::{approvals, shortpost};
use super::db::{self, Item, SCHEMA};
use super::rules::{self, Content, Due, Reject};
use super::{feed, media, ops};

const IDLE: Duration = Duration::from_secs(30);
pub const EVENT: &str = "social-schedule-changed";

static APP: OnceLock<AppHandle> = OnceLock::new();
static STARTED: AtomicBool = AtomicBool::new(false);

fn wake() -> &'static Notify {
    static W: OnceLock<Notify> = OnceLock::new();
    W.get_or_init(Notify::new)
}

fn deferred() -> &'static Mutex<HashSet<i64>> {
    static D: OnceLock<Mutex<HashSet<i64>>> = OnceLock::new();
    D.get_or_init(Default::default)
}

pub fn boot(app: AppHandle) {
    let _ = APP.set(app);
    start();
}

pub fn start() {
    if !STARTED.swap(true, Ordering::SeqCst) {
        tauri::async_runtime::spawn(run());
    }
}

pub fn running() -> bool {
    STARTED.load(Ordering::SeqCst)
}

fn emit(platform: &str, id: Option<i64>, status: Option<&str>) {
    if let Some(app) = APP.get() {
        let _ = app.emit(
            EVENT,
            json!({ "platform": platform, "id": id, "status": status }),
        );
    }
}

pub fn changed(p: Platform, id: Option<i64>, status: Option<&str>) {
    wake().notify_one();
    emit(p.id(), id, status);
}

pub fn notify(title: String, body: String) {
    if let Some(app) = APP.get() {
        let _ = app.notification().builder().title(title).body(body).show();
    }
}

pub fn human(error: &str) -> String {
    let Some((code, rest)) = error.split_once(':') else {
        return error.to_string();
    };
    serde_json::from_str::<Value>(rest.trim())
        .ok()
        .and_then(|v| v["message"].as_str().map(String::from))
        .unwrap_or_else(|| {
            if code
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                rest.trim().to_string()
            } else {
                error.to_string()
            }
        })
}

pub fn headline(item: &Item) -> String {
    let c = Content::from_json(&item.payload);
    let text = match &c.repost {
        Some(id) => format!("Repost of {id}"),
        None => c.parts.first().cloned().unwrap_or_default(),
    };
    let short: String = text.chars().take(60).collect();
    if short.len() < text.len() {
        format!("{short}…")
    } else {
        short
    }
}

pub fn pace_wait(error: &str) -> Duration {
    if error.contains("few seconds") {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(15)
    }
}

async fn wait_pace(p: Platform) -> Result<(), String> {
    for _ in 0..200 {
        match approvals::pace(p) {
            Ok(()) => return Ok(()),
            Err(e) => tokio::time::sleep(pace_wait(&e)).await,
        }
    }
    Err(format!(
        "RATE_LIMITED: {} is at the limit of actions OmniGet allows; the remaining parts were not published",
        p.label()
    ))
}

pub struct Outcome {
    pub parts: Vec<Value>,
    pub error: Option<String>,
}

pub async fn publish_with<F, Fut, G, GFut>(
    content: &Content,
    done: Vec<Value>,
    media_ids: Vec<String>,
    mut send: F,
    mut between: G,
) -> Outcome
where
    F: FnMut(&'static str, Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
    G: FnMut(Vec<Value>) -> GFut,
    GFut: Future<Output = Result<(), String>>,
{
    let mut parts = done;
    let fail = |parts: Vec<Value>, error: String| Outcome {
        parts,
        error: Some(error),
    };
    if let Some(target) = &content.repost {
        return match send("x_repost", json!({ "post": target })).await {
            Ok(_) => {
                parts.push(
                    json!({ "id": target, "url": format!("https://x.com/i/status/{target}") }),
                );
                Outcome { parts, error: None }
            }
            Err(e) => fail(parts, e),
        };
    }
    if content.parts.is_empty() {
        return fail(
            parts,
            "INVALID_ITEM: the item has nothing to publish".into(),
        );
    }
    let start = parts.len();
    for (i, text) in content.parts.iter().enumerate().skip(start) {
        if i > start {
            if let Err(e) = between(parts.clone()).await {
                return fail(parts, e);
            }
        }
        let mut args = json!({ "text": text });
        if i == 0 {
            if let Some(r) = &content.reply_to {
                args["reply_to"] = json!(r);
            }
            if let Some(q) = &content.quote {
                args["quote"] = json!(q);
            }
            if !media_ids.is_empty() {
                args["media_ids"] = json!(media_ids);
            }
        } else {
            let Some(previous) = parts[i - 1]["id"].as_str() else {
                return fail(parts, no_id().wire());
            };
            args["reply_to"] = json!(previous);
        }
        match send("x_create_post", args).await {
            Ok(v) => {
                let id = v["post"]["id"].as_str().filter(|s| !s.is_empty());
                parts.push(json!({ "id": id, "url": v["post"]["url"].as_str() }));
                if id.is_none() && i + 1 < content.parts.len() {
                    return fail(parts, no_id().wire());
                }
            }
            Err(e) => return fail(parts, e),
        }
    }
    Outcome { parts, error: None }
}

fn no_id() -> Reject {
    Reject::new(
        "UPSTREAM_CHANGED",
        "Short Posts accepted a part of the thread but did not return its id, so the next part cannot reply to it",
        "open the account, check what was published and finish the thread by hand",
    )
}

async fn publish(item: &Item) -> Outcome {
    let content = Content::from_json(&item.payload);
    let done = item.result["parts"].as_array().cloned().unwrap_or_default();
    let mut media_ids = Vec::new();
    if done.is_empty() {
        for path in &content.media {
            match media::upload(path).await {
                Ok(id) => media_ids.push(id),
                Err(e) => {
                    return Outcome {
                        parts: done,
                        error: Some(e),
                    }
                }
            }
        }
    }
    let id = item.id;
    publish_with(
        &content,
        done,
        media_ids,
        |tool, args| async move {
            let r = shortpost::call(tool, &args).await;
            if r.is_ok() {
                approvals::record(Platform::ShortPost);
            }
            r
        },
        |parts| async move {
            let _ = store::with(SCHEMA, |c| {
                db::progress(c, id, &json!({ "parts": parts }), now_ms())
            });
            wait_pace(Platform::ShortPost).await
        },
    )
    .await
}

async fn check_one_feed() -> bool {
    let now = now_ms();
    let Ok(feeds) = store::with(SCHEMA, |c| db::feeds(c, None)) else {
        return false;
    };
    let Some(f) = feeds.into_iter().find(|f| {
        f.last_checked_ms
            .is_none_or(|t| t + f.interval_min * 60_000 <= now)
    }) else {
        return false;
    };
    let fetched = feed::fetch(&f.url).await;
    let cx = super::ctx("feed".into(), rules::SHORT_LIMIT);
    let _ = store::with(SCHEMA, |c| Ok(cx.apply_feed(c, f.id, fetched)));
    emit(&f.platform, None, None);
    true
}

async fn tick() -> Duration {
    let now = now_ms();
    let next = store::with(SCHEMA, |c| db::next_due(c)).ok().flatten();
    let due_item = next.filter(|it| it.due_ms.is_some_and(|d| d <= now));
    let Some(item) = due_item else {
        if check_one_feed().await {
            return Duration::ZERO;
        }
        let until = store::with(SCHEMA, |c| db::next_due(c))
            .ok()
            .flatten()
            .and_then(|it| it.due_ms)
            .map(|d| Duration::from_millis((d - now_ms()).max(0) as u64));
        return until.map_or(IDLE, |d| d.min(IDLE));
    };
    let due = item.due_ms.unwrap_or(now);
    let finish = |status: &str, error: String| {
        let _ = store::with(SCHEMA, |c| db::mark(c, item.id, status, &error, now));
        emit(&item.platform, Some(item.id), Some(status));
    };
    if Platform::parse(&item.platform) != Some(Platform::ShortPost) {
        finish("failed", super::unsupported(&item.platform));
        return Duration::ZERO;
    }
    let p = Platform::ShortPost;
    let grace_ms = store::with(SCHEMA, |c| Ok(db::grace_min(c, &item.platform)))
        .unwrap_or(db::DEFAULT_GRACE_MIN)
        * 60_000;
    let exempt = item.forced
        || deferred()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&item.id);
    if rules::decide(now, due, grace_ms, exempt) == Due::Missed {
        finish(
            "missed",
            Reject::new(
                "MISSED",
                "it came due while OmniGet was closed or the computer was asleep, longer ago than the grace period",
                "use send_now to publish it now, update it with a new time, or cancel it",
            )
            .wire(),
        );
        notify(
            format!("Scheduled post missed on {}", p.label()),
            format!("{} was not sent because OmniGet was closed. Open Schedule to send it now or cancel it.", headline(&item)),
        );
        return Duration::ZERO;
    }
    if let Err(e) = approvals::pace(p) {
        deferred()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(item.id);
        return pace_wait(&e);
    }
    if !store::with(SCHEMA, |c| db::claim(c, item.id, now)).unwrap_or(false) {
        return Duration::from_secs(1);
    }
    emit(&item.platform, Some(item.id), Some("sending"));
    let outcome = publish(&item).await;
    deferred()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&item.id);
    let status = store::with(SCHEMA, |c| {
        ops::record(
            c,
            item.id,
            &outcome.parts,
            outcome.error.as_deref(),
            now_ms(),
        )
    })
    .unwrap_or("failed");
    emit(&item.platform, Some(item.id), Some(status));
    if let Some(e) = &outcome.error {
        notify(
            format!("Scheduled post failed on {}", p.label()),
            format!("{}: {}", headline(&item), human(e)),
        );
    }
    Duration::ZERO
}

async fn run() {
    let _ = store::with(SCHEMA, |c| db::recover_interrupted(c, now_ms()));
    loop {
        let pause = tick().await;
        tokio::select! {
            _ = tokio::time::sleep(pause) => {}
            _ = wake().notified() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn content(parts: &[&str]) -> Content {
        Content {
            parts: parts.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn posted(id: &str) -> Result<Value, String> {
        Ok(
            json!({ "ok": true, "post": { "id": id, "url": format!("https://x.com/me/status/{id}") } }),
        )
    }

    #[tokio::test]
    async fn a_thread_replies_to_the_previous_part() {
        let calls = RefCell::new(Vec::new());
        let gaps = RefCell::new(0);
        let mut c = content(&["one", "two", "three"]);
        c.reply_to = Some("9".into());
        c.quote = Some("8".into());
        let out = publish_with(
            &c,
            Vec::new(),
            vec!["m1".into()],
            |tool, args| {
                calls.borrow_mut().push((tool, args));
                let n = calls.borrow().len();
                async move { posted(&format!("10{n}")) }
            },
            |parts| {
                *gaps.borrow_mut() += parts.len();
                async { Ok(()) }
            },
        )
        .await;
        assert!(out.error.is_none());
        assert_eq!(out.parts.len(), 3);
        assert_eq!(
            out.parts[0],
            json!({ "id": "101", "url": "https://x.com/me/status/101" })
        );
        let calls = calls.into_inner();
        assert_eq!(
            calls[0],
            (
                "x_create_post",
                json!({ "text": "one", "reply_to": "9", "quote": "8", "media_ids": ["m1"] })
            )
        );
        assert_eq!(calls[1].1, json!({ "text": "two", "reply_to": "101" }));
        assert_eq!(calls[2].1, json!({ "text": "three", "reply_to": "102" }));
        assert_eq!(gaps.into_inner(), 1 + 2);
    }

    #[tokio::test]
    async fn a_failed_part_stops_the_thread_and_keeps_what_was_sent() {
        let n = RefCell::new(0);
        let out = publish_with(
            &content(&["one", "two", "three"]),
            Vec::new(),
            Vec::new(),
            |_, _| {
                *n.borrow_mut() += 1;
                let k = *n.borrow();
                async move {
                    if k == 2 {
                        Err("REJECTED: Short Posts refused the request".to_string())
                    } else {
                        posted("201")
                    }
                }
            },
            |_| async { Ok(()) },
        )
        .await;
        assert_eq!(
            out.error.as_deref(),
            Some("REJECTED: Short Posts refused the request")
        );
        assert_eq!(out.parts.len(), 1);
        assert_eq!(n.into_inner(), 2);
    }

    #[tokio::test]
    async fn resuming_continues_after_the_parts_already_sent() {
        let calls = RefCell::new(Vec::new());
        let done = vec![json!({ "id": "301", "url": "u" })];
        let out = publish_with(
            &content(&["one", "two"]),
            done,
            Vec::new(),
            |_, args| {
                calls.borrow_mut().push(args);
                async { posted("302") }
            },
            |_| async { Err("never called".to_string()) },
        )
        .await;
        assert!(out.error.is_none());
        assert_eq!(
            calls.into_inner(),
            vec![json!({ "text": "two", "reply_to": "301" })]
        );
        assert_eq!(out.parts.len(), 2);
    }

    #[tokio::test]
    async fn a_missing_id_in_the_middle_is_an_upstream_change() {
        let out = publish_with(
            &content(&["one", "two"]),
            Vec::new(),
            Vec::new(),
            |_, _| async { Ok(json!({ "ok": true, "post": null })) },
            |_| async { Ok(()) },
        )
        .await;
        assert!(out.error.unwrap().starts_with("UPSTREAM_CHANGED"));
        assert_eq!(out.parts.len(), 1);
        let single = publish_with(
            &content(&["one"]),
            Vec::new(),
            Vec::new(),
            |_, _| async { Ok(json!({ "ok": true, "post": null })) },
            |_| async { Ok(()) },
        )
        .await;
        assert!(single.error.is_none());
    }

    #[tokio::test]
    async fn a_repost_uses_the_repost_tool_and_pacing_can_stop_a_thread() {
        let c = Content {
            repost: Some("77".into()),
            ..Default::default()
        };
        let seen = RefCell::new(Vec::new());
        let out = publish_with(
            &c,
            Vec::new(),
            Vec::new(),
            |tool, args| {
                seen.borrow_mut().push((tool, args));
                async { Ok(json!({ "ok": true })) }
            },
            |_| async { Ok(()) },
        )
        .await;
        assert_eq!(
            seen.into_inner(),
            vec![("x_repost", json!({ "post": "77" }))]
        );
        assert_eq!(
            out.parts,
            vec![json!({ "id": "77", "url": "https://x.com/i/status/77" })]
        );
        let stopped = publish_with(
            &content(&["one", "two"]),
            Vec::new(),
            Vec::new(),
            |_, _| async { posted("1") },
            |_| async { Err("RATE_LIMITED: wait".to_string()) },
        )
        .await;
        assert_eq!(
            (stopped.parts.len(), stopped.error.as_deref()),
            (1, Some("RATE_LIMITED: wait"))
        );
    }

    #[test]
    fn notification_text_is_readable() {
        assert_eq!(
            human("REJECTED: Short Posts refused the request: duplicate (code 187)"),
            "Short Posts refused the request: duplicate (code 187)"
        );
        assert_eq!(
            human(&Reject::new("MISSED", "too late", "send it").wire()),
            "too late"
        );
        assert_eq!(human("plain words"), "plain words");
        assert_eq!(
            pace_wait("RATE_LIMITED: wait a few seconds between actions on Short Posts"),
            Duration::from_secs(2)
        );
        assert_eq!(
            pace_wait("RATE_LIMITED: OmniGet allows at most 20"),
            Duration::from_secs(15)
        );
    }
}
