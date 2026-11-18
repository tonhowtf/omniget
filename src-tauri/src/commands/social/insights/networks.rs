use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Value};

use crate::commands::social::model::Platform;

pub const REPO_SORTS: &[&str] = &["stars", "forks", "open_issues", "pushed"];

pub fn reports(p: Platform) -> Option<&'static [&'static str]> {
    match p {
        Platform::CodeHub => Some(&["summary", "posts", "times", "trend", "repos", "compare"]),
        _ => None,
    }
}

pub fn kind_of(p: Platform, item: &Value) -> Option<String> {
    let extra = &item["extra"];
    match p {
        Platform::CodeHub => extra["event"].as_str().map(event_name),
        Platform::Forums => match extra["kind"].as_str()? {
            "comment" => Some("comment".into()),
            _ => Some("post".into()),
        },
        Platform::PinBoard => Some("pin".into()),
        _ => None,
    }
}

fn event_name(event: &str) -> String {
    let base = event.trim_end_matches("Event");
    let mut out = String::new();
    for (i, c) in base.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

pub fn posts_args(p: Platform, args: &mut Value, replies: bool) {
    if replies && p == Platform::Forums {
        args["kind"] = json!("all");
    }
}

fn no_counts() -> Value {
    json!({ "available": false, "reason": "Code Hub activity events carry no reaction or reply counts." })
}

pub fn adjust(p: Platform, report: &str, result: &mut Value, kinds: &HashMap<String, String>) {
    if kinds.is_empty() {
        return;
    }
    match report {
        "summary" => {
            let mut types: BTreeMap<&str, u64> = BTreeMap::new();
            for k in kinds.values() {
                *types.entry(k.as_str()).or_default() += 1;
            }
            result["types"] = json!(types);
            if p == Platform::CodeHub {
                result["engagement"] = no_counts();
            }
        }
        "posts" => {
            if let Some(items) = result["items"].as_array_mut() {
                for row in items {
                    if let Some(k) = row["id"].as_str().and_then(|id| kinds.get(id)) {
                        row["type"] = json!(k);
                    }
                    if p == Platform::CodeHub {
                        row["engagement"] = Value::Null;
                    }
                }
            }
        }
        "times" | "trend" if p == Platform::CodeHub => {
            let key = if report == "times" { "cells" } else { "items" };
            if let Some(cells) = result[key].as_array_mut() {
                for c in cells {
                    c["engagement"] = Value::Null;
                    c["mean"] = Value::Null;
                }
            }
            if report == "times" {
                result["best"] = Value::Null;
                result["note"] = json!(
                    "Code Hub events have no engagement; the grid shows when the account is active."
                );
            }
        }
        _ => {}
    }
}

pub fn adjust_compare(p: Platform, result: &mut Value) {
    if p != Platform::CodeHub {
        return;
    }
    if let Some(accounts) = result["accounts"].as_array_mut() {
        for a in accounts {
            a["summary"]["engagement"] = no_counts();
        }
    }
}

pub fn repo_row(item: &Value) -> Option<Value> {
    let name = item["id"].as_str().filter(|x| !x.is_empty())?;
    let stats = &item["stats"];
    let extra = &item["extra"];
    Some(json!({
        "repo": name,
        "url": item["url"],
        "description": item["text"],
        "stars": stats["likes"].as_u64(),
        "forks": stats["reposts"].as_u64(),
        "open_issues": stats["replies"].as_u64(),
        "pushed": item["created_at"].as_i64(),
        "language": extra["language"].as_str(),
        "fork": extra["fork"].as_bool().unwrap_or(false),
        "archived": extra["archived"].as_bool().unwrap_or(false),
    }))
}

pub fn sort_repos(rows: &mut [Value], key: &str, ascending: bool) {
    rows.sort_by(|a, b| {
        let ord = match (a[key].as_f64(), b[key].as_f64()) {
            (Some(x), Some(y)) => {
                let o = x.partial_cmp(&y).unwrap_or(Ordering::Equal);
                if ascending {
                    o
                } else {
                    o.reverse()
                }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        ord.then_with(|| a["repo"].as_str().cmp(&b["repo"].as_str()))
    });
}

pub fn repo_totals(rows: &[Value]) -> Value {
    let sum = |key: &str| rows.iter().filter_map(|r| r[key].as_u64()).sum::<u64>();
    json!({
        "repos": rows.len(),
        "stars": sum("stars"),
        "forks": sum("forks"),
        "open_issues": sum("open_issues"),
        "forked_from_others": rows.iter().filter(|r| r["fork"] == true).count(),
        "archived": rows.iter().filter(|r| r["archived"] == true).count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codehub_events_are_typed_by_their_event_name() {
        let item =
            json!({ "id": "1", "extra": { "kind": "event", "event": "PullRequestReviewEvent" } });
        assert_eq!(
            kind_of(Platform::CodeHub, &item).as_deref(),
            Some("pull_request_review")
        );
        let push = json!({ "id": "2", "extra": { "event": "PushEvent" } });
        assert_eq!(kind_of(Platform::CodeHub, &push).as_deref(), Some("push"));
        assert_eq!(kind_of(Platform::CodeHub, &json!({ "id": "3" })), None);
    }

    #[test]
    fn forums_tells_posts_from_comments_and_pinboard_has_pins() {
        let link = json!({ "id": "t3_1", "extra": { "kind": "link" } });
        let comment = json!({ "id": "t1_1", "extra": { "kind": "comment" } });
        assert_eq!(kind_of(Platform::Forums, &link).as_deref(), Some("post"));
        assert_eq!(
            kind_of(Platform::Forums, &comment).as_deref(),
            Some("comment")
        );
        assert_eq!(
            kind_of(Platform::PinBoard, &json!({ "id": "9" })).as_deref(),
            Some("pin")
        );
        assert_eq!(kind_of(Platform::ShortPost, &link), None);
        let mut args = json!({ "username": "ana" });
        posts_args(Platform::Forums, &mut args, true);
        assert_eq!(args["kind"], "all");
        let mut args = json!({ "username": "ana" });
        posts_args(Platform::PinBoard, &mut args, true);
        assert!(args.get("kind").is_none());
    }

    #[test]
    fn codehub_reports_say_engagement_is_unavailable_instead_of_zero() {
        let kinds: HashMap<String, String> = [("1", "push"), ("2", "push"), ("3", "watch")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let mut summary = json!({ "types": { "post": 3 }, "engagement": { "available": true, "total": 0, "mean": 0.0 } });
        adjust(Platform::CodeHub, "summary", &mut summary, &kinds);
        assert_eq!(summary["types"], json!({ "push": 2, "watch": 1 }));
        assert_eq!(summary["engagement"]["available"], false);
        let mut posts = json!({ "items": [{ "id": "3", "type": "post", "engagement": 0 }] });
        adjust(Platform::CodeHub, "posts", &mut posts, &kinds);
        assert_eq!(posts["items"][0]["type"], "watch");
        assert!(posts["items"][0]["engagement"].is_null());
        let mut times = json!({ "cells": [{ "weekday": 0, "hour": 9, "posts": 2, "engagement": 0, "mean": 0.0 }], "best": { "weekday": 0 } });
        adjust(Platform::CodeHub, "times", &mut times, &kinds);
        assert!(times["cells"][0]["mean"].is_null());
        assert_eq!(times["cells"][0]["posts"], 2);
        assert!(times["best"].is_null());
        let mut compare = json!({ "accounts": [{ "handle": "ana", "summary": { "engagement": { "available": true, "mean": 0.0 } } }] });
        adjust_compare(Platform::CodeHub, &mut compare);
        assert_eq!(
            compare["accounts"][0]["summary"]["engagement"]["available"],
            false
        );
    }

    #[test]
    fn forums_keeps_its_engagement_and_gets_its_types() {
        let kinds: HashMap<String, String> = [("a", "post"), ("b", "comment")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let mut summary =
            json!({ "types": { "post": 2 }, "engagement": { "available": true, "mean": 4.0 } });
        adjust(Platform::Forums, "summary", &mut summary, &kinds);
        assert_eq!(summary["types"], json!({ "post": 1, "comment": 1 }));
        assert_eq!(summary["engagement"]["available"], true);
        let mut untouched = json!({ "types": { "post": 2 } });
        adjust(
            Platform::ShortPost,
            "summary",
            &mut untouched,
            &HashMap::new(),
        );
        assert_eq!(untouched["types"], json!({ "post": 2 }));
    }

    fn repo(name: &str, stars: Option<u64>, forks: u64, issues: u64, pushed: i64) -> Value {
        json!({
            "id": name, "url": format!("https://github.com/{name}"), "text": "about",
            "created_at": pushed,
            "stats": { "likes": stars, "reposts": forks, "replies": issues, "views": 4 },
            "extra": { "kind": "repo", "language": "Rust", "fork": name.ends_with("fork").then_some(true) },
        })
    }

    #[test]
    fn the_repos_report_maps_sorts_and_totals() {
        let items = [
            repo("ana/small", Some(2), 9, 1, 300),
            repo("ana/big", Some(50), 3, 7, 100),
            repo("ana/unknown", None, 0, 0, 200),
            repo("ana/a-fork", Some(2), 0, 0, 50),
            json!({ "text": "no id" }),
        ];
        let mut rows: Vec<Value> = items.iter().filter_map(repo_row).collect();
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[1],
            json!({
                "repo": "ana/big", "url": "https://github.com/ana/big", "description": "about",
                "stars": 50, "forks": 3, "open_issues": 7, "pushed": 100, "language": "Rust",
                "fork": false, "archived": false,
            })
        );
        let names = |rows: &[Value]| -> Vec<String> {
            rows.iter()
                .map(|r| r["repo"].as_str().unwrap().to_string())
                .collect()
        };
        sort_repos(&mut rows, "stars", false);
        assert_eq!(
            names(&rows),
            vec!["ana/big", "ana/a-fork", "ana/small", "ana/unknown"]
        );
        sort_repos(&mut rows, "stars", true);
        assert_eq!(names(&rows)[3], "ana/unknown");
        sort_repos(&mut rows, "pushed", false);
        assert_eq!(names(&rows)[0], "ana/small");
        sort_repos(&mut rows, "forks", false);
        assert_eq!(names(&rows)[0], "ana/small");
        sort_repos(&mut rows, "open_issues", false);
        assert_eq!(names(&rows)[0], "ana/big");
        let totals = repo_totals(&rows);
        assert_eq!(totals["repos"], 4);
        assert_eq!(totals["stars"], 54);
        assert_eq!(totals["forks"], 12);
        assert_eq!(totals["open_issues"], 8);
        assert_eq!(totals["forked_from_others"], 1);
    }

    #[test]
    fn each_network_lists_only_the_reports_it_can_answer() {
        let gh = reports(Platform::CodeHub).unwrap();
        assert!(gh.contains(&"repos"));
        assert!(!gh.contains(&"outliers"));
        assert!(reports(Platform::Forums).is_none());
        assert!(reports(Platform::ShortPost).is_none());
    }
}
