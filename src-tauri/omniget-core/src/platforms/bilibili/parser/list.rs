use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::{ContentMetadata, EpisodeItem, PaginationInfo, ParsedContent};

const COLLECTION_URL: &str = "https://api.bilibili.com/x/polymer/web-space/seasons_archives_list";
const SERIES_ARCHIVES_URL: &str = "https://api.bilibili.com/x/series/archives";
const SERIES_META_URL: &str = "https://api.bilibili.com/x/series/series";
const PAGE_SIZE: u32 = 30;

pub async fn parse_collection(
    client: &ApiClient,
    mid: u64,
    sid: u64,
    page: u32,
) -> Result<ParsedContent> {
    let url = format!(
        "{}?mid={}&season_id={}&sort_reverse=false&page_size={}&page_num={}",
        COLLECTION_URL,
        mid,
        sid,
        PAGE_SIZE,
        page.max(1)
    );
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;
    let meta_title = data
        .get("meta")
        .and_then(|m| m.get("name"))
        .and_then(Value::as_str)
        .map(String::from);
    let total = data
        .get("page")
        .and_then(|p| p.get("total"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let total_pages = ((total + PAGE_SIZE - 1) / PAGE_SIZE).max(1);
    let archives = data
        .get("archives")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let items = archives_to_items(&archives, format!("collection:{}", sid));
    if items.is_empty() {
        return Err(BilibiliError::ContentUnavailable);
    }
    let title = meta_title
        .clone()
        .unwrap_or_else(|| format!("Bilibili Collection {}", sid));
    let metadata = ContentMetadata {
        collection_title: meta_title,
        uploader_uid: Some(mid),
        ..ContentMetadata::default()
    };
    Ok(ParsedContent {
        title,
        items,
        metadata,
        pagination: Some(PaginationInfo {
            total_items: total,
            total_pages,
            current_page: page.max(1),
            page_size: PAGE_SIZE,
        }),
    })
}

pub async fn parse_series(
    client: &ApiClient,
    mid: u64,
    sid: u64,
    page: u32,
) -> Result<ParsedContent> {
    let url = format!(
        "{}?mid={}&series_id={}&only_normal=true&sort=desc&pn={}&ps={}&web_location=333.999",
        SERIES_ARCHIVES_URL,
        mid,
        sid,
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
    let archives = data
        .get("archives")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let items = archives_to_items(&archives, format!("series:{}", sid));
    if items.is_empty() {
        return Err(BilibiliError::ContentUnavailable);
    }

    let meta_title: Option<String> = {
        let meta_url = format!("{}?series_id={}", SERIES_META_URL, sid);
        let meta_raw = client.get_json(&meta_url).await.ok();
        meta_raw
            .as_ref()
            .and_then(|r| check_api_response(r).ok())
            .and_then(|d| d.get("meta"))
            .and_then(|m| m.get("name"))
            .and_then(Value::as_str)
            .map(String::from)
    };

    let title = meta_title
        .clone()
        .unwrap_or_else(|| format!("Bilibili Series {}", sid));
    let metadata = ContentMetadata {
        collection_title: meta_title,
        uploader_uid: Some(mid),
        ..ContentMetadata::default()
    };
    Ok(ParsedContent {
        title,
        items,
        metadata,
        pagination: Some(PaginationInfo {
            total_items: total,
            total_pages,
            current_page: page.max(1),
            page_size: PAGE_SIZE,
        }),
    })
}

fn archives_to_items(archives: &[Value], prefix: String) -> Vec<EpisodeItem> {
    let mut items: Vec<EpisodeItem> = Vec::new();
    for a in archives {
        let bvid = a
            .get("bvid")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if bvid.is_empty() {
            continue;
        }
        let aid = a.get("aid").and_then(Value::as_u64);
        let title = a
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let cover = a.get("pic").and_then(Value::as_str).map(String::from);
        let duration = a.get("duration").and_then(Value::as_f64);
        let pub_time = a.get("pubdate").and_then(Value::as_u64);
        items.push(EpisodeItem {
            episode_id: format!("{}:{}", prefix, bvid),
            title,
            aid,
            bvid: Some(bvid.clone()),
            duration_seconds: duration,
            cover_url: cover,
            pub_time_secs: pub_time,
            url: Some(format!("https://www.bilibili.com/video/{}", bvid)),
            ..EpisodeItem::default()
        });
    }
    items
}
