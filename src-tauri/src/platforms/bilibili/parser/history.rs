use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::{ContentMetadata, EpisodeItem, PaginationInfo, ParsedContent};

const HISTORY_URL: &str = "https://api.bilibili.com/x/web-interface/history/search";
const PAGE_SIZE: u32 = 20;

pub async fn parse(client: &ApiClient, page: u32) -> Result<ParsedContent> {
    if client.account_slug().is_none() {
        return Err(BilibiliError::NotLoggedIn);
    }
    let url = format!(
        "{}?pn={}&ps={}&keyword=&business=archive&add_time_start=0&add_time_end=0&arc_max_duration=0&arc_min_duration=0&device_type=0&web_location=333.1391",
        HISTORY_URL,
        page.max(1),
        PAGE_SIZE
    );
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;

    let total = data
        .get("page")
        .and_then(|p| p.get("total"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let total_pages = ((total + PAGE_SIZE - 1) / PAGE_SIZE).max(1);
    let list = data
        .get("list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut items: Vec<EpisodeItem> = Vec::new();
    for v in &list {
        let history = v.get("history");
        let business = history
            .and_then(|h| h.get("business"))
            .and_then(Value::as_str)
            .unwrap_or("archive");
        let bvid = history
            .and_then(|h| h.get("bvid"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let cid = history.and_then(|h| h.get("cid")).and_then(Value::as_u64);
        if bvid.is_empty() && business == "archive" {
            continue;
        }
        let title_main = v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let long_title = v
            .get("long_title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let composed_title = if business == "pgc" && !long_title.is_empty() {
            format!("{} - {}", title_main, long_title)
        } else {
            title_main
        };
        let cover = v.get("cover").and_then(Value::as_str).map(String::from);
        let duration = v.get("duration").and_then(Value::as_f64);
        let view_at = v.get("view_at").and_then(Value::as_u64);
        let expired = duration.map(|d| d <= 0.0).unwrap_or(true);
        let badge = if expired {
            Some("badge.bilibili.expired".to_string())
        } else {
            None
        };
        let ep_id = history
            .and_then(|h| h.get("epid").and_then(Value::as_u64))
            .or_else(|| history.and_then(|h| h.get("ep_id")).and_then(Value::as_u64));
        items.push(EpisodeItem {
            episode_id: format!("history:{}:{}", business, bvid),
            title: composed_title,
            bvid: if bvid.is_empty() {
                None
            } else {
                Some(bvid.clone())
            },
            cid,
            ep_id,
            duration_seconds: duration,
            cover_url: cover,
            pub_time_secs: view_at,
            badge,
            url: Some(if business == "pgc" {
                format!(
                    "https://www.bilibili.com/bangumi/play/ep{}",
                    ep_id.unwrap_or(0)
                )
            } else {
                format!("https://www.bilibili.com/video/{}", bvid)
            }),
            ..EpisodeItem::default()
        });
    }

    if items.is_empty() {
        return Err(BilibiliError::ContentUnavailable);
    }

    Ok(ParsedContent {
        title: String::from("History"),
        items,
        metadata: ContentMetadata::default(),
        pagination: Some(PaginationInfo {
            total_items: total,
            total_pages,
            current_page: page.max(1),
            page_size: PAGE_SIZE,
        }),
    })
}
