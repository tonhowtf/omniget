use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::{ContentMetadata, EpisodeItem, PaginationInfo, ParsedContent};

const RESOURCE_LIST_URL: &str = "https://api.bilibili.com/x/v3/fav/resource/list";
const PAGE_SIZE: u32 = 40;

pub async fn parse(client: &ApiClient, fid: u64, page: u32) -> Result<ParsedContent> {
    let url = format!(
        "{}?media_id={}&pn={}&ps={}&keyword=&order=mtime&type=0&tid=0&platform=web&web_location=333.1387",
        RESOURCE_LIST_URL,
        fid,
        page.max(1),
        PAGE_SIZE
    );
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;

    let info = data.get("info");
    let fav_title = info
        .and_then(|i| i.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let media_count = info
        .and_then(|i| i.get("media_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let owner_name = info
        .and_then(|i| i.get("upper"))
        .and_then(|u| u.get("name"))
        .and_then(Value::as_str)
        .map(String::from);
    let owner_id = info
        .and_then(|i| i.get("upper"))
        .and_then(|u| u.get("mid"))
        .and_then(Value::as_u64);
    let total_pages = ((media_count + PAGE_SIZE - 1) / PAGE_SIZE).max(1);

    let medias = data
        .get("medias")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut items: Vec<EpisodeItem> = Vec::new();
    for m in &medias {
        let bvid = m
            .get("bvid")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if bvid.is_empty() {
            continue;
        }
        let title = m
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let duration = m.get("duration").and_then(Value::as_f64);
        let cover = m.get("cover").and_then(Value::as_str).map(String::from);
        let pub_time = m.get("pubtime").and_then(Value::as_u64);
        let fav_time = m.get("fav_time").and_then(Value::as_u64);
        let is_ogv = m.get("ogv").is_some() && !m.get("ogv").unwrap_or(&Value::Null).is_null();
        let badge = if is_ogv {
            Some("badge.bilibili.pgc".to_string())
        } else {
            None
        };
        let ep_id = m
            .get("ogv")
            .and_then(|o| o.get("ep_id"))
            .and_then(Value::as_u64);
        items.push(EpisodeItem {
            episode_id: format!("fav:{}:{}", fid, bvid),
            title,
            bvid: Some(bvid.clone()),
            ep_id,
            duration_seconds: duration,
            cover_url: cover,
            pub_time_secs: fav_time.or(pub_time),
            badge,
            url: Some(if is_ogv {
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

    let metadata = ContentMetadata {
        favorites_name: Some(fav_title.clone()),
        favorites_owner: owner_name,
        uploader_uid: owner_id,
        ..ContentMetadata::default()
    };

    Ok(ParsedContent {
        title: fav_title,
        items,
        metadata,
        pagination: Some(PaginationInfo {
            total_items: media_count,
            total_pages,
            current_page: page.max(1),
            page_size: PAGE_SIZE,
        }),
    })
}
