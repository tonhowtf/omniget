//! SponsorBlock, DeArrow and Return YouTube Dislike: URLs and response
//! parsing. SponsorBlock is queried by hash prefix so the server never
//! learns which video is playing.

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::model::{DeArrow, Ryd, SbAction, SbCategory, SbSegment};

const SB_ACTIONS: &str = r#"["skip","mute","full","poi","chapter"]"#;

pub fn sb_url(server: &str, video_id: &str, categories: &[&str]) -> String {
    let hash = hex::encode(Sha256::digest(video_id.as_bytes()));
    let cats = serde_json::to_string(categories).unwrap_or_else(|_| "[]".into());
    format!(
        "{}/api/skipSegments/{}?categories={}&actionTypes={}",
        server.trim_end_matches('/'),
        &hash[..4],
        urlencoding::encode(&cats),
        urlencoding::encode(SB_ACTIONS)
    )
}

fn from_str<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(Value::String(s.to_string())).ok()
}

/// Segments of `video_id` from a hash-prefix response (which also carries
/// other videos sharing the prefix), sorted by start.
pub fn parse_sb(json: &Value, video_id: &str) -> Vec<SbSegment> {
    let Some(list) = json.as_array() else {
        return Vec::new();
    };
    let mut out: Vec<SbSegment> = list
        .iter()
        .filter(|v| v["videoID"].as_str() == Some(video_id))
        .flat_map(|v| v["segments"].as_array().cloned().unwrap_or_default())
        .filter_map(|s| {
            let category: SbCategory = from_str(s["category"].as_str()?)?;
            let action: SbAction = from_str(s["actionType"].as_str().unwrap_or("skip"))?;
            let seg = s["segment"].as_array()?;
            let start_s = seg.first()?.as_f64()?;
            let end_s = seg.get(1)?.as_f64()?;
            if end_s < start_s {
                return None;
            }
            Some(SbSegment {
                uuid: s["UUID"].as_str().unwrap_or_default().to_string(),
                category,
                action,
                start_s,
                end_s,
                votes: s["votes"].as_i64().unwrap_or(0) as i32,
                description: s["description"]
                    .as_str()
                    .filter(|d| !d.is_empty())
                    .map(str::to_string),
            })
        })
        .collect();
    out.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
    out
}

pub fn dearrow_url(video_id: &str) -> String {
    format!(
        "https://sponsor.ajay.app/api/branding?videoID={}",
        urlencoding::encode(video_id)
    )
}

/// DeArrow marks "keep this word as typed" with a leading `>`; it is not
/// meant to be shown.
fn clean_title(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut prev_space = true;
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '>' && prev_space && chars.peek().is_some_and(|n| !n.is_whitespace()) {
            continue;
        }
        prev_space = c.is_whitespace();
        out.push(c);
    }
    out.trim().to_string()
}

fn trusted(v: &Value) -> bool {
    v["locked"].as_bool().unwrap_or(false) || v["votes"].as_i64().unwrap_or(0) >= 0
}

/// The server already orders submissions (locked, then votes). Take the
/// first trusted one; an "original" winner means keep the original.
pub fn parse_dearrow(json: &Value, video_id: &str) -> DeArrow {
    let title = json["titles"]
        .as_array()
        .and_then(|a| a.iter().find(|t| trusted(t)))
        .filter(|t| !t["original"].as_bool().unwrap_or(false))
        .and_then(|t| t["title"].as_str())
        .map(clean_title)
        .filter(|t| !t.is_empty());
    let thumbs = json["thumbnails"].as_array();
    let mut time = thumbs
        .and_then(|a| a.iter().find(|t| trusted(t)))
        .filter(|t| !t["original"].as_bool().unwrap_or(false))
        .and_then(|t| t["timestamp"].as_f64());
    // No submissions at all: DeArrow's own fallback is a random frame.
    if time.is_none() && thumbs.is_none_or(|a| a.is_empty()) {
        if let (Some(r), Some(d)) = (
            json["randomTime"].as_f64(),
            json["videoDuration"].as_f64().filter(|d| *d > 0.0),
        ) {
            time = Some(r * d);
        }
    }
    DeArrow {
        title,
        thumb_time_s: time,
        thumb_url: time.map(|t| {
            format!(
                "https://dearrow-thumb.ajay.app/api/v1/getThumbnail?videoID={}&time={}",
                urlencoding::encode(video_id),
                t
            )
        }),
    }
}

pub fn ryd_url(video_id: &str) -> String {
    format!(
        "https://returnyoutubedislikeapi.com/votes?videoId={}",
        urlencoding::encode(video_id)
    )
}

pub fn parse_ryd(json: &Value) -> Option<Ryd> {
    let n = |k: &str| {
        json[k]
            .as_u64()
            .or_else(|| json[k].as_f64().map(|f| f as u64))
    };
    Some(Ryd {
        likes: n("likes")?,
        dislikes: n("dislikes")?,
        rating: json["rating"].as_f64().unwrap_or(0.0),
        view_count: n("viewCount").unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sb_url_uses_hash_prefix() {
        let u = sb_url(
            "https://sponsor.ajay.app/",
            "dQw4w9WgXcQ",
            &["sponsor", "music_offtopic"],
        );
        let prefix = &hex::encode(Sha256::digest(b"dQw4w9WgXcQ"))[..4];
        assert!(u.starts_with(&format!(
            "https://sponsor.ajay.app/api/skipSegments/{prefix}?categories="
        )));
        assert!(!u.contains("dQw4w9WgXcQ"));
        let decoded = urlencoding::decode(&u).unwrap();
        assert!(decoded.contains(r#"categories=["sponsor","music_offtopic"]"#));
        assert!(decoded.contains(r#"actionTypes=["skip","mute","full","poi","chapter"]"#));
    }

    #[test]
    fn sb_parse_filters_video() {
        let body = json!([
            {"videoID": "other", "segments": [{"category": "sponsor", "actionType": "skip", "segment": [0.0, 5.0], "UUID": "x"}]},
            {"videoID": "abc", "segments": [
                {"category": "selfpromo", "actionType": "mute", "segment": [30.5, 40.0], "UUID": "b", "votes": 2},
                {"category": "sponsor", "actionType": "skip", "segment": [10.0, 20.0], "UUID": "a", "votes": 5},
                {"category": "poi_highlight", "actionType": "poi", "segment": [60.0, 60.0], "UUID": "c"},
                {"category": "chapter", "actionType": "chapter", "segment": [0.0, 30.0], "UUID": "d", "description": "Intro talk"},
                {"category": "newcat", "actionType": "skip", "segment": [1.0, 2.0], "UUID": "e"}
            ]}
        ]);
        let s = parse_sb(&body, "abc");
        assert_eq!(s.len(), 4);
        assert_eq!(s[0].category, SbCategory::Chapter);
        assert_eq!(s[0].description.as_deref(), Some("Intro talk"));
        assert_eq!(s[1].uuid, "a");
        assert_eq!(s[1].votes, 5);
        assert_eq!(s[2].action, SbAction::Mute);
        assert_eq!(s[3].category, SbCategory::PoiHighlight);
        assert!(parse_sb(&json!({"error": 1}), "abc").is_empty());
    }

    #[test]
    fn dearrow_picks_trusted_non_original() {
        let body = json!({
            "titles": [
                {"title": "Why >iPhone batteries die", "original": false, "votes": 3, "locked": false},
                {"title": "ORIGINAL!!!", "original": true, "votes": 1}
            ],
            "thumbnails": [{"timestamp": 42.5, "original": false, "votes": 0}],
            "randomTime": 0.5, "videoDuration": 100
        });
        let d = parse_dearrow(&body, "abc");
        assert_eq!(d.title.as_deref(), Some("Why iPhone batteries die"));
        assert_eq!(d.thumb_time_s, Some(42.5));
        assert_eq!(
            d.thumb_url.as_deref(),
            Some("https://dearrow-thumb.ajay.app/api/v1/getThumbnail?videoID=abc&time=42.5")
        );

        let original_wins = json!({
            "titles": [{"title": "Real", "original": true, "votes": 5}],
            "thumbnails": [{"timestamp": null, "original": true, "votes": 2}]
        });
        let d = parse_dearrow(&original_wins, "abc");
        assert_eq!(d, DeArrow::default());

        let downvoted =
            json!({"titles": [{"title": "Bad", "original": false, "votes": -1}], "thumbnails": []});
        assert!(parse_dearrow(&downvoted, "abc").title.is_none());

        let random =
            json!({"titles": [], "thumbnails": [], "randomTime": 0.25, "videoDuration": 200.0});
        assert_eq!(parse_dearrow(&random, "abc").thumb_time_s, Some(50.0));
        assert_eq!(clean_title(">Hello a > b"), "Hello a > b");
    }

    #[test]
    fn ryd() {
        assert!(ryd_url("a b").ends_with("videoId=a%20b"));
        assert!(dearrow_url("abc").ends_with("videoID=abc"));
        let r = parse_ryd(
            &json!({"id": "x", "likes": 10, "dislikes": 2, "rating": 4.5, "viewCount": 100}),
        )
        .unwrap();
        assert_eq!((r.likes, r.dislikes, r.view_count), (10, 2, 100));
        assert!(parse_ryd(&json!({"error": "x"})).is_none());
    }
}
