use serde::Serialize;
use serde_json::{json, Value};

use super::tiktok::iso;
use super::{client, fail, stats, strings, uint};

const PAGES: usize = 4;

pub const CAVEATS: &[&str] = &[
    "Play counts are Instagram plays only; plays of the same reel cross-posted to Facebook are not included.",
    "Compare each profile against its own median, never raw plays across profiles of different sizes.",
    "Every profile is cut to the same number of most recent reels; a profile with fewer reels than asked is flagged and lowers the compared sample.",
    "A sample under 10 reels is directional: confidence is low.",
    "Public data only; pinned reels can appear among the most recent.",
];

pub const GUIDE: &str = "Write the comparison from this evidence only. Start with a table: profile, compared sample, median plays, mean plays, likes and comments medians, reels per week, confidence. Then explain the patterns behind each profile's top and bottom reels using the captions and durations fetched for those outliers, with the reel URL beside every observation. State the view-count limits from `caveats` in plain words, flag every profile listed in `flags`, and do not rank profiles by raw plays or infer reach, saves or revenue.";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Reel {
    pub code: String,
    pub url: String,
    pub taken_at: Option<i64>,
    pub plays: Option<u64>,
    pub likes: Option<u64>,
    pub comments: Option<u64>,
    pub duration_s: Option<f64>,
    pub caption: String,
}

pub fn reel(item: &Value) -> Option<Reel> {
    let v = if item["media"].is_object() {
        &item["media"]
    } else {
        item
    };
    let code = v["code"].as_str().filter(|c| !c.is_empty())?.to_string();
    let caption = v["caption"]["text"]
        .as_str()
        .or_else(|| v["caption"].as_str())
        .unwrap_or_default();
    Some(Reel {
        url: v["url"]
            .as_str()
            .filter(|u| u.starts_with("http"))
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://www.instagram.com/reel/{code}/")),
        code,
        taken_at: v["taken_at"].as_i64().filter(|t| *t > 0),
        plays: uint(&v["ig_play_count"]).or_else(|| uint(&v["play_count"])),
        likes: uint(&v["like_count"]),
        comments: uint(&v["comment_count"]),
        duration_s: v["video_duration"].as_f64(),
        caption: caption.chars().take(300).collect(),
    })
}

pub fn reels(body: &Value) -> Result<Vec<Reel>, String> {
    match body.get("items") {
        Some(Value::Array(items)) => Ok(items.iter().filter_map(reel).collect()),
        Some(Value::Null) => Ok(Vec::new()),
        _ => Err(fail(
            "UPSTREAM_CHANGED",
            "the Instagram answer has no items list",
            false,
            "call instagram_user_reels with research_call to see the new shape and report it",
            Value::Null,
        )),
    }
}

pub fn compared_sample(asked: usize, available: &[usize]) -> usize {
    available
        .iter()
        .copied()
        .filter(|n| *n > 0)
        .min()
        .map_or(0, |least| least.min(asked))
}

fn newest(list: &[Reel], sample: usize) -> Vec<&Reel> {
    let mut sorted: Vec<&Reel> = list.iter().collect();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.taken_at.unwrap_or(0)));
    sorted.truncate(sample);
    sorted
}

pub fn cadence(times: &[i64]) -> Value {
    let mut sorted = times.to_vec();
    sorted.sort_unstable();
    let gaps: Vec<f64> = sorted
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) as f64 / 86_400.0)
        .collect();
    let span_days = match (sorted.first(), sorted.last()) {
        (Some(first), Some(last)) if last > first => Some((last - first) as f64 / 86_400.0),
        _ => None,
    };
    let round = |n: f64| (n * 10.0).round() / 10.0;
    json!({
        "reels_per_week": span_days.map(|days| round(gaps.len() as f64 / days * 7.0)),
        "median_gap_days": stats::median(&gaps).map(round),
        "span_days": span_days.map(round),
    })
}

fn row(reel: &Reel, median: Option<f64>) -> Value {
    let lift = reel.plays.and_then(|p| stats::lift(p as f64, median));
    json!({
        "url": reel.url,
        "posted": reel.taken_at.and_then(iso),
        "plays": reel.plays,
        "likes": reel.likes,
        "comments": reel.comments,
        "duration_s": reel.duration_s,
        "caption": reel.caption,
        "lift_vs_own_median": lift,
        "lift_label": stats::lift_label(lift),
    })
}

fn floats(values: impl Iterator<Item = Option<u64>>) -> Vec<f64> {
    values.flatten().map(|n| n as f64).collect()
}

pub fn profile_report(handle: &str, list: &[Reel], asked: usize, compared: usize) -> Value {
    let used = newest(list, compared);
    let plays = floats(used.iter().map(|r| r.plays));
    let median = stats::median(&plays);
    let times: Vec<i64> = used.iter().filter_map(|r| r.taken_at).collect();
    let mut by_plays: Vec<&&Reel> = used.iter().filter(|r| r.plays.is_some()).collect();
    by_plays.sort_by_key(|r| std::cmp::Reverse(r.plays));
    let mut flags: Vec<&str> = Vec::new();
    if list.len() < asked {
        flags.push("sample_smaller_than_asked");
    }
    if plays.len() < used.len() {
        flags.push("some_reels_without_play_count");
    }
    if list.is_empty() {
        flags.push("no_reels_found");
    }
    json!({
        "handle": handle,
        "profile_url": format!("https://www.instagram.com/{handle}/"),
        "available": list.len(),
        "sample_asked": asked,
        "sample": used.len(),
        "confidence": stats::confidence(plays.len()),
        "date_from": times.iter().min().and_then(|t| iso(*t)),
        "date_to": times.iter().max().and_then(|t| iso(*t)),
        "plays": stats::summary(&plays),
        "likes_median": stats::median(&floats(used.iter().map(|r| r.likes))),
        "comments_median": stats::median(&floats(used.iter().map(|r| r.comments))),
        "cadence": cadence(&times),
        "top": by_plays.first().map(|r| row(r, median)),
        "bottom": by_plays.last().filter(|_| by_plays.len() > 1).map(|r| row(r, median)),
        "reels": used.iter().map(|r| row(r, median)).collect::<Vec<_>>(),
        "flags": flags,
    })
}

pub fn detail(body: &Value) -> Value {
    let media = &body["data"]["xdt_shortcode_media"];
    let caption = media["edge_media_to_caption"]["edges"][0]["node"]["text"]
        .as_str()
        .map(|text| text.chars().take(1500).collect::<String>());
    json!({
        "caption_full": caption,
        "duration_s": media["video_duration"].as_f64(),
        "plays_detail": uint(&media["video_play_count"]),
        "paid_partnership": media["is_paid_partnership"].as_bool(),
        "music": media["clips_music_attribution_info"]["song_name"],
    })
}

async fn feed(handle: &str, sample: usize) -> Result<Vec<Reel>, String> {
    let mut all: Vec<Reel> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..PAGES {
        let body = client::get(
            "instagram_user_reels",
            json!({"handle": handle, "max_id": cursor}),
        )
        .await?;
        let page = reels(&body)?;
        let empty = page.is_empty();
        for reel in page {
            if !all.iter().any(|r| r.code == reel.code) {
                all.push(reel);
            }
        }
        cursor = body["max_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if all.len() >= sample || empty || cursor.is_none() {
            break;
        }
    }
    Ok(all)
}

async fn enrich(report: &mut Value, side: &str) {
    let Some(url) = report[side]["url"].as_str().map(str::to_string) else {
        return;
    };
    match client::get("instagram_post", json!({ "url": url })).await {
        Ok(body) => report[side]["detail"] = detail(&body),
        Err(error) => {
            report[side]["detail_error"] = json!(error.split(':').next().unwrap_or_default())
        }
    }
}

pub async fn call(a: &Value) -> Result<Value, String> {
    let handles: Vec<String> = strings(&a["profiles"], 10, 60)
        .into_iter()
        .map(|h| h.trim_start_matches('@').to_string())
        .collect();
    if handles.len() < 2 {
        return Err(fail(
            "INVALID_PARAMS",
            "profiles needs 2 to 10 Instagram handles",
            false,
            "pass the handles to compare, without @",
            Value::Null,
        ));
    }
    let asked = a["sample"].as_u64().unwrap_or(12).clamp(3, 50) as usize;
    let with_detail = a["detail"].as_str() != Some("none");
    let before = client::credits_spent();
    let feeds = futures::future::join_all(handles.iter().map(|h| feed(h, asked))).await;
    if let Some(Err(error)) = feeds.iter().find(|f| {
        f.as_ref().is_err_and(|e| {
            ["NO_API_KEY", "INVALID_API_KEY", "OUT_OF_CREDITS"]
                .iter()
                .any(|code| e.starts_with(code))
        })
    }) {
        return Err(error.clone());
    }
    let available: Vec<usize> = feeds
        .iter()
        .map(|f| f.as_ref().map_or(0, Vec::len))
        .collect();
    let compared = compared_sample(asked, &available);
    let mut reports: Vec<Value> = handles
        .iter()
        .zip(&feeds)
        .map(|(handle, found)| match found {
            Ok(list) => profile_report(handle, list, asked, compared),
            Err(error) => json!({
                "handle": handle,
                "profile_url": format!("https://www.instagram.com/{handle}/"),
                "sample": 0,
                "flags": ["unavailable"],
                "error": error.split(':').next().unwrap_or_default(),
            }),
        })
        .collect();
    if with_detail {
        futures::future::join_all(reports.iter_mut().map(|report| async move {
            enrich(report, "top").await;
            enrich(report, "bottom").await;
        }))
        .await;
    }
    let flagged: Vec<Value> = reports
        .iter()
        .filter(|r| r["flags"].as_array().is_some_and(|f| !f.is_empty()))
        .map(|r| json!({"handle": r["handle"], "flags": r["flags"]}))
        .collect();
    let mut out = json!({
        "sample_asked": asked,
        "compared_sample": compared,
        "confidence": stats::confidence(compared),
        "profiles": reports,
        "total": reports.len(),
        "returned": reports.len(),
        "truncated": false,
        "flags": flagged,
        "credits_used": client::credits_spent() - before,
        "caveats": CAVEATS,
        "guide": GUIDE,
        "untrusted": true,
    });
    if compared == 0 {
        out["reason"] = json!("no_reels_found");
        out["next_step"] = json!("check the handles; private or empty profiles return no reels");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(code: &str, plays: u64, taken_at: i64) -> Value {
        json!({
            "id": "3641924023820356866_21393171",
            "code": code,
            "taken_at": taken_at,
            "caption": {"text": "Summer brings out all the girlies, soda, and global warming!!!"},
            "play_count": plays + 1,
            "ig_play_count": plays,
            "comment_count": 1096,
            "like_count": 71_768,
            "url": format!("https://www.instagram.com/reel/{code}/"),
        })
    }

    fn list(plays: &[u64]) -> Vec<Reel> {
        plays
            .iter()
            .enumerate()
            .map(|(i, p)| {
                reel(&item(
                    &format!("c{i}"),
                    *p,
                    1_748_000_000 + i as i64 * 2 * 86_400,
                ))
                .unwrap()
            })
            .collect()
    }

    #[test]
    fn a_reel_item_parses_with_instagram_plays_first() {
        let parsed = reel(&item("DKKtpqaxg0C", 1_685_367, 1_748_371_310)).unwrap();
        assert_eq!(parsed.url, "https://www.instagram.com/reel/DKKtpqaxg0C/");
        assert_eq!(parsed.plays, Some(1_685_367));
        assert_eq!(parsed.likes, Some(71_768));
        assert_eq!(parsed.comments, Some(1096));
        assert_eq!(parsed.duration_s, None);
        assert!(parsed.caption.starts_with("Summer"));
        let nested =
            reel(&json!({"media": {"code": "X", "play_count": 5, "video_duration": 12.5}}))
                .unwrap();
        assert_eq!((nested.plays, nested.duration_s), (Some(5), Some(12.5)));
        assert_eq!(nested.url, "https://www.instagram.com/reel/X/");
        assert_eq!(reel(&json!({"id": "1"})), None);
    }

    #[test]
    fn the_items_list_is_required_and_an_unknown_shape_is_reported() {
        assert_eq!(
            reels(&json!({"items": [item("a", 1, 1), {"no": "code"}]}))
                .unwrap()
                .len(),
            1
        );
        assert!(reels(&json!({"items": null})).unwrap().is_empty());
        assert!(reels(&json!({"reels": []}))
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
    }

    #[test]
    fn every_profile_is_compared_on_the_smallest_available_sample() {
        assert_eq!(compared_sample(12, &[30, 12, 20]), 12);
        assert_eq!(compared_sample(12, &[30, 7, 20]), 7);
        assert_eq!(compared_sample(12, &[30, 0, 9]), 9);
        assert_eq!(compared_sample(12, &[0, 0]), 0);
        assert_eq!(compared_sample(12, &[]), 0);
    }

    #[test]
    fn cadence_reports_reels_per_week_and_the_median_gap() {
        let times: Vec<i64> = (0..8).map(|n| n * 2 * 86_400).collect();
        let found = cadence(&times);
        assert_eq!(found["reels_per_week"], 3.5);
        assert_eq!(found["median_gap_days"], 2.0);
        assert_eq!(found["span_days"], 14.0);
        let single = cadence(&[5]);
        assert!(single["reels_per_week"].is_null());
        assert!(single["median_gap_days"].is_null());
    }

    #[test]
    fn a_profile_report_uses_the_newest_reels_and_names_top_and_bottom() {
        let all = list(&[900, 100, 200, 300, 5000, 150]);
        let report = profile_report("jane", &all, 5, 5);
        assert_eq!(report["available"], 6);
        assert_eq!(report["sample"], 5);
        assert_eq!(report["plays"]["median"], 200.0);
        assert_eq!(report["plays"]["mean"], 1150.0);
        assert_eq!(report["confidence"], "low");
        assert_eq!(report["top"]["plays"], 5000);
        assert_eq!(report["top"]["lift_vs_own_median"], 25.0);
        assert_eq!(report["top"]["lift_label"], "huge");
        assert_eq!(report["bottom"]["plays"], 100);
        assert_eq!(report["cadence"]["median_gap_days"], 2.0);
        assert_eq!(report["flags"], json!([]));
        assert_eq!(report["reels"].as_array().unwrap().len(), 5);
        assert_eq!(
            report["reels"][0]["url"],
            "https://www.instagram.com/reel/c5/"
        );
    }

    #[test]
    fn a_smaller_sample_and_missing_play_counts_are_flagged() {
        let mut few = list(&[10, 20, 30]);
        few[0].plays = None;
        let report = profile_report("jane", &few, 12, 3);
        assert_eq!(
            report["flags"],
            json!(["sample_smaller_than_asked", "some_reels_without_play_count"])
        );
        assert_eq!(report["plays"]["sample"], 2);
        let none = profile_report("ghost", &[], 12, 0);
        assert_eq!(
            none["flags"],
            json!(["sample_smaller_than_asked", "no_reels_found"])
        );
        assert!(none["top"].is_null());
        let one = profile_report("solo", &list(&[10]), 1, 1);
        assert!(one["bottom"].is_null());
        assert_eq!(one["top"]["plays"], 10);
    }

    #[test]
    fn post_detail_adds_caption_duration_and_partnership() {
        let body = json!({"data": {"xdt_shortcode_media": {
            "video_play_count": 425_901,
            "video_duration": 84.666,
            "is_paid_partnership": false,
            "edge_media_to_caption": {"edges": [{"node": {"text": "Full caption here"}}]},
        }}});
        let found = detail(&body);
        assert_eq!(found["caption_full"], "Full caption here");
        assert_eq!(found["duration_s"], 84.666);
        assert_eq!(found["plays_detail"], 425_901);
        assert_eq!(found["paid_partnership"], false);
        assert!(detail(&json!({}))["caption_full"].is_null());
    }

    #[tokio::test]
    async fn fewer_than_two_profiles_is_refused_before_any_request() {
        let error = call(&json!({"profiles": ["@jane"]})).await.unwrap_err();
        assert!(error.starts_with("INVALID_PARAMS"));
        for word in ["Instagram plays only", "Facebook"] {
            assert!(CAVEATS.join(" ").contains(word));
        }
    }
}
