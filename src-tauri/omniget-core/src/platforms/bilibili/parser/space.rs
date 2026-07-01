use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::super::wbi;
use super::{ContentMetadata, EpisodeItem, PaginationInfo, ParsedContent};

const SEARCH_URL: &str = "https://api.bilibili.com/x/space/wbi/arc/search";
const CARD_URL: &str = "https://api.bilibili.com/x/web-interface/card";
const PAGE_SIZE: u32 = 40;

pub async fn parse(client: &ApiClient, mid: u64, page: u32) -> Result<ParsedContent> {
    let card_url = format!("{}?mid={}", CARD_URL, mid);
    let card_raw = client.get_json(&card_url).await.ok();
    let space_owner = card_raw
        .as_ref()
        .and_then(|r| check_api_response(r).ok())
        .and_then(|d| d.get("card"))
        .and_then(|c| c.get("name"))
        .and_then(Value::as_str)
        .map(String::from);

    let pn = page.max(1).to_string();
    let ps = PAGE_SIZE.to_string();
    let mid_str = mid.to_string();
    let params: Vec<(&str, String)> = vec![
        ("mid", mid_str),
        ("pn", pn),
        ("ps", ps),
        ("tid", "0".to_string()),
        ("order", "pubdate".to_string()),
        ("platform", "web".to_string()),
        ("web_location", "333.1387".to_string()),
    ];
    let signed = wbi::signed_query(client, &params).await?;
    let url = format!("{}?{}", SEARCH_URL, signed);
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;

    let count = data
        .get("page")
        .and_then(|p| p.get("count"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let total_pages = ((count + PAGE_SIZE - 1) / PAGE_SIZE).max(1);

    let vlist = data
        .get("list")
        .and_then(|l| l.get("vlist"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut items: Vec<EpisodeItem> = Vec::new();
    for v in &vlist {
        let bvid = v
            .get("bvid")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if bvid.is_empty() {
            continue;
        }
        let aid = v.get("aid").and_then(Value::as_u64);
        let title = v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let cover = v.get("pic").and_then(Value::as_str).map(String::from);
        let duration = v
            .get("length")
            .and_then(Value::as_str)
            .and_then(parse_mmss_to_seconds);
        let created = v.get("created").and_then(Value::as_u64);
        let mut badge: Option<String> = None;
        if v.get("is_charging_arc")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            badge = Some("badge.bilibili.charging_exclusive".to_string());
        } else if v.get("is_union_video").and_then(Value::as_u64).unwrap_or(0) > 0 {
            badge = Some("badge.bilibili.collab".to_string());
        }
        items.push(EpisodeItem {
            episode_id: format!("space:{}:{}", mid, bvid),
            title,
            aid,
            bvid: Some(bvid.clone()),
            duration_seconds: duration,
            cover_url: cover,
            pub_time_secs: created,
            badge,
            url: Some(format!("https://www.bilibili.com/video/{}", bvid)),
            ..EpisodeItem::default()
        });
    }

    if items.is_empty() {
        return Err(BilibiliError::ContentUnavailable);
    }

    let title = space_owner
        .clone()
        .map(|n| format!("{} · Bilibili", n))
        .unwrap_or_else(|| format!("Bilibili Space {}", mid));

    let metadata = ContentMetadata {
        space_owner,
        uploader_uid: Some(mid),
        ..ContentMetadata::default()
    };

    Ok(ParsedContent {
        title,
        items,
        metadata,
        pagination: Some(PaginationInfo {
            total_items: count,
            total_pages,
            current_page: page.max(1),
            page_size: PAGE_SIZE,
        }),
    })
}

fn parse_mmss_to_seconds(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        2 => {
            let m: u64 = parts[0].parse().ok()?;
            let sec: u64 = parts[1].parse().ok()?;
            Some((m * 60 + sec) as f64)
        }
        3 => {
            let h: u64 = parts[0].parse().ok()?;
            let m: u64 = parts[1].parse().ok()?;
            let sec: u64 = parts[2].parse().ok()?;
            Some((h * 3600 + m * 60 + sec) as f64)
        }
        _ => None,
    }
}
