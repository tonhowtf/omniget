use chrono::{DateTime, NaiveDate, TimeZone};
use serde_json::{json, Value};

use super::super::model::{Platform, Post};
use super::invalid;
use super::stats;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub from: i64,
    pub to: i64,
}

fn moment<Tz: TimeZone>(raw: &str, tz: &Tz, end: bool) -> Option<i64> {
    let raw = raw.trim();
    if let Ok(day) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let day = if end { day.succ_opt()? } else { day };
        return tz
            .from_local_datetime(&day.and_hms_opt(0, 0, 0)?)
            .earliest()
            .map(|d| d.timestamp());
    }
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.timestamp())
}

fn bound<Tz: TimeZone>(a: &Value, key: &str, tz: &Tz, end: bool) -> Result<Option<i64>, String> {
    match &a[key] {
        Value::Null => Ok(None),
        Value::String(s) => moment(s, tz, end).map(Some).ok_or_else(|| {
            invalid(format!(
                "{key} must be a day (2026-11-01) or a time with offset (2026-11-01T09:00:00-03:00)"
            ))
        }),
        _ => Err(invalid(format!("{key} must be text"))),
    }
}

pub fn ranges<Tz: TimeZone>(a: &Value, tz: &Tz) -> Result<(Range, Range), String> {
    let (Some(from), Some(to)) = (bound(a, "from", tz, false)?, bound(a, "to", tz, true)?) else {
        return Err(invalid(
            "compare_periods needs from and to (a day such as 2026-11-01; to is included)",
        ));
    };
    if from >= to {
        return Err(invalid("from must be before to"));
    }
    let first = Range { from, to };
    let second = match (
        bound(a, "compare_from", tz, false)?,
        bound(a, "compare_to", tz, true)?,
    ) {
        (None, None) => Range {
            from: from - (to - from),
            to: from,
        },
        (Some(f), Some(t)) if f < t => Range { from: f, to: t },
        (Some(_), Some(_)) => return Err(invalid("compare_from must be before compare_to")),
        _ => {
            return Err(invalid(
                "pass both compare_from and compare_to, or neither to compare with the same length right before",
            ))
        }
    };
    if first.from < second.to && second.from < first.to {
        return Err(invalid("the two periods overlap"));
    }
    Ok((first, second))
}

fn within(posts: &[Post], r: Range) -> Vec<Post> {
    posts
        .iter()
        .filter(|p| p.created_at.is_some_and(|t| t >= r.from && t < r.to))
        .cloned()
        .collect()
}

fn side(posts: &[Post], r: Range, platform: Platform) -> Value {
    let s = stats::summary(posts, platform);
    json!({
        "from": r.from,
        "to": r.to,
        "days": ((r.to - r.from) as f64 / 86400.0 * 10.0).round() / 10.0,
        "posts": s["analyzed"],
        "types": s["types"],
        "engagement": s["engagement"],
        "metrics": s["metrics"],
        "confidence": s["confidence"],
    })
}

fn diff(a: &Value, b: &Value) -> Value {
    let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) else {
        return json!({ "a": a, "b": b, "diff": Value::Null, "pct": Value::Null });
    };
    let round = |v: f64| (v * 100.0).round() / 100.0;
    json!({
        "a": a,
        "b": b,
        "diff": round(x - y),
        "pct": (y != 0.0).then(|| round((x - y) / y * 100.0)),
    })
}

pub fn compare(posts: &[Post], platform: Platform, a: Range, b: Range, whole: bool) -> Value {
    let first = side(&within(posts, a), a, platform);
    let second = side(&within(posts, b), b, platform);
    let mut difference = serde_json::Map::new();
    difference.insert("posts".into(), diff(&first["posts"], &second["posts"]));
    for stat in ["total", "mean", "median"] {
        difference.insert(
            format!("engagement_{stat}"),
            diff(&first["engagement"][stat], &second["engagement"][stat]),
        );
    }
    for metric in ["likes", "replies", "reposts", "saves", "views"] {
        difference.insert(
            format!("{metric}_mean"),
            diff(
                &first["metrics"][metric]["mean"],
                &second["metrics"][metric]["mean"],
            ),
        );
    }
    let oldest = posts.iter().filter_map(|p| p.created_at).min();
    let covers = |r: Range| whole || oldest.is_some_and(|o| o <= r.from);
    let mut coverage = json!({
        "sample": posts.len(),
        "sample_from": oldest,
        "sample_to": posts.iter().filter_map(|p| p.created_at).max(),
        "covers_a": covers(a),
        "covers_b": covers(b),
        "undated": posts.iter().filter(|p| p.created_at.is_none()).count(),
    });
    if !(covers(a) && covers(b)) {
        coverage["note"] = json!("The posts read do not go back to the start of a period, so that side is undercounted. Raise sample (200 at most) or pick later dates.");
    }
    let mut out = json!({
        "a": first,
        "b": second,
        "difference": difference,
        "difference_key": "diff = a minus b; pct = change against b in percent, null when b is zero or the network hides the number",
        "coverage": coverage,
    });
    if out["a"]["posts"] == 0 && out["b"]["posts"] == 0 {
        out["empty_reason"] = json!("No post of the sample falls inside either period.");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::social::model::Stats;
    use chrono::FixedOffset;

    const DAY: i64 = 86400;
    const NOV1: i64 = 1_793_502_000;

    fn tz() -> FixedOffset {
        FixedOffset::west_opt(3 * 3600).unwrap()
    }

    fn post(id: &str, at: Option<i64>, likes: u64, views: Option<u64>) -> Post {
        Post {
            id: id.into(),
            created_at: at,
            stats: Stats {
                likes: Some(likes),
                views,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn days_are_read_in_the_given_zone_and_to_is_included() {
        assert_eq!(moment("2026-11-01", &tz(), false), Some(NOV1));
        assert_eq!(moment("2026-11-01", &tz(), true), Some(NOV1 + DAY));
        assert_eq!(moment("2026-11-01T00:00:00-03:00", &tz(), true), Some(NOV1));
        assert_eq!(moment("01/11/2026", &tz(), false), None);
        let (a, b) = ranges(&json!({ "from": "2026-11-08", "to": "2026-11-14" }), &tz()).unwrap();
        assert_eq!(
            a,
            Range {
                from: NOV1 + 7 * DAY,
                to: NOV1 + 14 * DAY
            }
        );
        assert_eq!(
            b,
            Range {
                from: NOV1,
                to: NOV1 + 7 * DAY
            }
        );
        let (_, b) = ranges(
            &json!({ "from": "2026-11-08", "to": "2026-11-14", "compare_from": "2026-10-01", "compare_to": "2026-10-03" }),
            &tz(),
        )
        .unwrap();
        assert_eq!(b.to - b.from, 3 * DAY);
    }

    #[test]
    fn bad_periods_are_refused_with_a_next_step() {
        for bad in [
            json!({}),
            json!({ "from": "2026-11-08" }),
            json!({ "from": "2026-11-08", "to": "2026-11-01" }),
            json!({ "from": "yesterday", "to": "2026-11-01" }),
            json!({ "from": 5, "to": "2026-11-01" }),
            json!({ "from": "2026-11-08", "to": "2026-11-14", "compare_from": "2026-11-01" }),
            json!({ "from": "2026-11-08", "to": "2026-11-14", "compare_from": "2026-11-05", "compare_to": "2026-11-01" }),
            json!({ "from": "2026-11-08", "to": "2026-11-14", "compare_from": "2026-11-10", "compare_to": "2026-11-20" }),
        ] {
            let e = ranges(&bad, &tz()).unwrap_err();
            assert!(e.starts_with("INVALID_ARGUMENTS"), "{bad}");
            assert!(e.contains("next_step"));
        }
    }

    #[test]
    fn the_same_statistics_come_side_by_side_with_the_difference() {
        let a = Range {
            from: NOV1 + 7 * DAY,
            to: NOV1 + 14 * DAY,
        };
        let b = Range {
            from: NOV1,
            to: NOV1 + 7 * DAY,
        };
        let posts = vec![
            post("1", Some(NOV1 + 13 * DAY), 30, Some(300)),
            post("2", Some(NOV1 + 8 * DAY), 10, Some(100)),
            post("3", Some(NOV1 + 7 * DAY), 20, None),
            post("4", Some(NOV1 + 7 * DAY - 1), 10, Some(400)),
            post("5", Some(NOV1), 10, None),
            post("6", Some(NOV1 - 1), 999, None),
            post("7", None, 999, None),
        ];
        let v = compare(&posts, Platform::X, a, b, false);
        assert_eq!(v["a"]["posts"], 3);
        assert_eq!(v["b"]["posts"], 2);
        assert_eq!(v["a"]["days"], 7.0);
        assert_eq!(v["a"]["engagement"]["mean"], 20.0);
        assert_eq!(v["b"]["engagement"]["mean"], 10.0);
        let d = &v["difference"];
        assert_eq!(
            d["posts"],
            json!({ "a": 3, "b": 2, "diff": 1.0, "pct": 50.0 })
        );
        assert_eq!(d["engagement_mean"]["diff"], 10.0);
        assert_eq!(d["engagement_mean"]["pct"], 100.0);
        assert_eq!(d["engagement_total"]["diff"], 40.0);
        assert_eq!(d["views_mean"]["a"], 200.0);
        assert_eq!(d["views_mean"]["pct"], -50.0);
        assert_eq!(d["saves_mean"]["diff"], Value::Null);
        assert_eq!(v["coverage"]["covers_a"], true);
        assert_eq!(v["coverage"]["covers_b"], true);
        assert_eq!(v["coverage"]["undated"], 1);
        assert!(v["coverage"].get("note").is_none());
        assert!(v.get("empty_reason").is_none());
    }

    #[test]
    fn a_sample_that_does_not_reach_a_period_says_so_and_zero_has_no_percentage() {
        let a = Range {
            from: NOV1 + 7 * DAY,
            to: NOV1 + 14 * DAY,
        };
        let b = Range {
            from: NOV1,
            to: NOV1 + 7 * DAY,
        };
        let posts = vec![post("1", Some(NOV1 + 9 * DAY), 30, None)];
        let v = compare(&posts, Platform::Instagram, a, b, false);
        assert_eq!(v["coverage"]["covers_a"], false);
        assert_eq!(v["coverage"]["covers_b"], false);
        assert!(v["coverage"]["note"]
            .as_str()
            .unwrap()
            .contains("Raise sample"));
        assert_eq!(v["difference"]["posts"]["pct"], Value::Null);
        assert_eq!(v["difference"]["posts"]["diff"], 1.0);
        assert_eq!(v["difference"]["engagement_mean"]["diff"], Value::Null);
        let whole = compare(&posts, Platform::Instagram, a, b, true);
        assert_eq!(whole["coverage"]["covers_b"], true);
        let none = compare(&[], Platform::X, a, b, true);
        assert!(none["empty_reason"].is_string());
    }
}
