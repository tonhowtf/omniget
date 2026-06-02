use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::super::wbi;
use super::{ContentMetadata, EpisodeItem, PaginationInfo, ParsedContent};

const TOVIEW_URL: &str = "https://api.bilibili.com/x/v2/history/toview/web";
const PAGE_SIZE: u32 = 20;

pub async fn parse(client: &ApiClient, page: u32) -> Result<ParsedContent> {
    if client.account_slug().is_none() {
        return Err(BilibiliError::NotLoggedIn);
    }

    let params: Vec<(&str, String)> = vec![
        ("pn", page.max(1).to_string()),
        ("ps", PAGE_SIZE.to_string()),
        ("viewed", "0".to_string()),
        ("key", String::new()),
        ("asc", "false".to_string()),
        ("need_split", "true".to_string()),
        ("web_location", "333.881".to_string()),
    ];
    let signed = wbi::signed_query(client, &params).await?;
    let url = format!("{}?{}", TOVIEW_URL, signed);
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;

    let count = data.get("count").and_then(Value::as_u64).unwrap_or(0) as u32;
    let total_pages = ((count + PAGE_SIZE - 1) / PAGE_SIZE).max(1);
    let list = data
        .get("list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut items: Vec<EpisodeItem> = Vec::new();
    for v in &list {
        let bvid = v
            .get("bvid")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if bvid.is_empty() {
            continue;
        }
        let aid = v.get("aid").and_then(Value::as_u64);
        let cid = v.get("cid").and_then(Value::as_u64);
        let title = v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let cover = v.get("pic").and_then(Value::as_str).map(String::from);
        let duration = v.get("duration").and_then(Value::as_f64);
        let pub_time = v.get("pubdate").and_then(Value::as_u64);
        let add_at = v.get("add_at").and_then(Value::as_u64);
        let bangumi = v.get("bangumi");
        let ep_id = bangumi.and_then(|b| b.get("ep_id")).and_then(Value::as_u64);
        let badge = if ep_id.is_some() {
            Some("badge.bilibili.pgc".to_string())
        } else {
            None
        };
        items.push(EpisodeItem {
            episode_id: format!("toview:{}", bvid),
            title,
            aid,
            bvid: Some(bvid.clone()),
            cid,
            ep_id,
            duration_seconds: duration,
            cover_url: cover,
            pub_time_secs: add_at.or(pub_time),
            badge,
            url: Some(if let Some(id) = ep_id {
                format!("https://www.bilibili.com/bangumi/play/ep{}", id)
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
        title: String::from("Watch Later"),
        items,
        metadata: ContentMetadata::default(),
        pagination: Some(PaginationInfo {
            total_items: count,
            total_pages,
            current_page: page.max(1),
            page_size: PAGE_SIZE,
        }),
    })
}
