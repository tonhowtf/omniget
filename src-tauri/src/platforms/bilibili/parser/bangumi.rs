use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::{ContentMetadata, EpisodeItem, ParsedContent};

const SEASON_URL: &str = "https://api.bilibili.com/pgc/view/web/season";
const REVIEW_USER_URL: &str = "https://api.bilibili.com/pgc/review/user";

pub async fn parse_by_ep(client: &ApiClient, ep_id: u64) -> Result<ParsedContent> {
    let url = format!("{}?ep_id={}", SEASON_URL, ep_id);
    let raw = client.get_json(&url).await?;
    let result = check_api_response(&raw)?;
    build_parsed_content(result, Some(ep_id))
}

pub async fn parse_by_season(client: &ApiClient, season_id: u64) -> Result<ParsedContent> {
    let url = format!("{}?season_id={}", SEASON_URL, season_id);
    let raw = client.get_json(&url).await?;
    let result = check_api_response(&raw)?;
    build_parsed_content(result, None)
}

pub async fn parse_by_media(client: &ApiClient, media_id: u64) -> Result<ParsedContent> {
    let lookup_url = format!("{}?media_id={}", REVIEW_USER_URL, media_id);
    let raw = client.get_json(&lookup_url).await?;
    let data = check_api_response(&raw)?;
    let season_id = data
        .get("media")
        .and_then(|m| m.get("season_id"))
        .and_then(Value::as_u64)
        .ok_or(BilibiliError::ContentUnavailable)?;
    parse_by_season(client, season_id).await
}

fn build_parsed_content(result: &Value, highlight_ep: Option<u64>) -> Result<ParsedContent> {
    let series_title = result
        .get("series")
        .and_then(|s| s.get("series_title"))
        .and_then(Value::as_str)
        .map(String::from);
    let season_title = result
        .get("season_title")
        .and_then(Value::as_str)
        .map(String::from);
    let title = series_title
        .clone()
        .or_else(|| season_title.clone())
        .unwrap_or_else(|| "Bilibili Bangumi".to_string());

    let cover = result
        .get("cover")
        .and_then(Value::as_str)
        .map(String::from);
    let poster = result
        .get("bkg_cover")
        .and_then(Value::as_str)
        .or_else(|| result.get("horizontal_cover").and_then(Value::as_str))
        .map(String::from)
        .or_else(|| cover.clone());
    let description = result
        .get("evaluate")
        .and_then(Value::as_str)
        .map(String::from);
    let media_id = result.get("media_id").and_then(Value::as_u64);
    let season_id = result.get("season_id").and_then(Value::as_u64);
    let actors = result
        .get("actors")
        .and_then(Value::as_str)
        .map(String::from);
    let rating = result
        .get("rating")
        .and_then(|r| r.get("score"))
        .and_then(Value::as_f64)
        .map(|v| v as f32);
    let premiered = result
        .get("publish")
        .and_then(|p| p.get("pub_time"))
        .and_then(Value::as_str)
        .and_then(parse_pub_time)
        .or_else(|| {
            result
                .get("publish")
                .and_then(|p| p.get("pub_time_show"))
                .and_then(Value::as_str)
                .and_then(parse_pub_time)
        });
    let areas: Vec<String> = result
        .get("areas")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("name").and_then(Value::as_str).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let styles: Vec<String> = result
        .get("styles")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let uploader = result
        .get("up_info")
        .and_then(|u| u.get("uname"))
        .and_then(Value::as_str)
        .map(String::from);
    let uploader_uid = result
        .get("up_info")
        .and_then(|u| u.get("mid"))
        .and_then(Value::as_u64);
    let uploader_avatar = result
        .get("up_info")
        .and_then(|u| u.get("avatar"))
        .and_then(Value::as_str)
        .map(String::from);

    let metadata = ContentMetadata {
        series_title: series_title.clone().or_else(|| Some(title.clone())),
        season_title,
        season_id,
        media_id,
        uploader,
        uploader_uid,
        uploader_avatar,
        description,
        cover: cover.clone(),
        poster,
        areas,
        styles,
        actors,
        rating,
        premiered_secs: premiered,
        ..ContentMetadata::default()
    };

    let mut items: Vec<EpisodeItem> = Vec::new();
    let mut counter: u32 = 0;
    if let Some(eps) = result.get("episodes").and_then(Value::as_array) {
        for ep in eps {
            if let Some(item) = build_episode_item(ep, None, &mut counter, false) {
                items.push(item);
            }
        }
    }
    if let Some(secs) = result.get("section").and_then(Value::as_array) {
        for sec in secs {
            let section_title = sec.get("title").and_then(Value::as_str).map(String::from);
            let is_preview = section_title
                .as_deref()
                .map(|t| t.contains("预告") || t.contains("PV"))
                .unwrap_or(false);
            if let Some(eps) = sec.get("episodes").and_then(Value::as_array) {
                for ep in eps {
                    if let Some(item) =
                        build_episode_item(ep, section_title.clone(), &mut counter, is_preview)
                    {
                        items.push(item);
                    }
                }
            }
        }
    }
    let _ = highlight_ep;
    if items.is_empty() {
        return Err(BilibiliError::ContentUnavailable);
    }
    Ok(ParsedContent {
        title,
        items,
        metadata,
        pagination: None,
    })
}

fn build_episode_item(
    ep: &Value,
    section_title: Option<String>,
    counter: &mut u32,
    is_preview_section: bool,
) -> Option<EpisodeItem> {
    let bvid = ep.get("bvid").and_then(Value::as_str)?;
    let cid = ep.get("cid").and_then(Value::as_u64);
    let aid = ep.get("aid").and_then(Value::as_u64);
    let ep_id = ep
        .get("id")
        .and_then(Value::as_u64)
        .or_else(|| ep.get("ep_id").and_then(Value::as_u64));
    let title = ep
        .get("long_title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| ep.get("show_title").and_then(Value::as_str))
        .or_else(|| ep.get("title").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let duration_ms = ep.get("duration").and_then(Value::as_f64);
    let duration_seconds = duration_ms.map(|ms| ms / 1000.0);
    let cover = ep.get("cover").and_then(Value::as_str).map(String::from);
    let pub_time = ep.get("pub_time").and_then(Value::as_u64);
    let badge_raw = ep.get("badge").and_then(Value::as_str).unwrap_or("");
    let is_preview = is_preview_section || badge_raw.contains("预告") || badge_raw.contains("PV");
    let badge = if is_preview {
        Some("badge.bilibili.preview".to_string())
    } else if badge_raw.contains("充电") {
        Some("badge.bilibili.charging_exclusive".to_string())
    } else if !badge_raw.is_empty() {
        Some(badge_raw.to_string())
    } else {
        None
    };
    let episode_number = if is_preview {
        None
    } else {
        *counter += 1;
        Some(*counter)
    };
    Some(EpisodeItem {
        episode_id: format!("ep:{}", ep_id.unwrap_or(0)),
        title,
        aid,
        bvid: Some(bvid.to_string()),
        cid,
        ep_id,
        duration_seconds,
        cover_url: cover,
        pub_time_secs: pub_time,
        badge,
        section_title,
        episode_number,
        url: ep_id.map(|id| format!("https://www.bilibili.com/bangumi/play/ep{}", id)),
        ..EpisodeItem::default()
    })
}

fn parse_pub_time(s: &str) -> Option<u64> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc().timestamp().max(0) as u64)
}
