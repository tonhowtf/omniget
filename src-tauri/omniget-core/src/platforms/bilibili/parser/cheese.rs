use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::{ContentMetadata, EpisodeItem, ParsedContent};

const SEASON_URL: &str = "https://api.bilibili.com/pugv/view/web/season/v2";

pub async fn parse_by_ep(client: &ApiClient, ep_id: u64) -> Result<ParsedContent> {
    let url = format!("{}?ep_id={}", SEASON_URL, ep_id);
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;
    build_parsed_content(data)
}

pub async fn parse_by_season(client: &ApiClient, season_id: u64) -> Result<ParsedContent> {
    let url = format!("{}?season_id={}", SEASON_URL, season_id);
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;
    build_parsed_content(data)
}

fn build_parsed_content(data: &Value) -> Result<ParsedContent> {
    let title = data
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Bilibili Course")
        .to_string();
    let cover = data.get("cover").and_then(Value::as_str).map(String::from);
    let description = data
        .get("subtitle")
        .and_then(Value::as_str)
        .map(String::from);
    let season_id = data.get("season_id").and_then(Value::as_u64);
    let uploader = data
        .get("up_info")
        .and_then(|u| u.get("uname"))
        .and_then(Value::as_str)
        .map(String::from);
    let uploader_uid = data
        .get("up_info")
        .and_then(|u| u.get("mid"))
        .and_then(Value::as_u64);
    let uploader_avatar = data
        .get("up_info")
        .and_then(|u| u.get("avatar"))
        .and_then(Value::as_str)
        .map(String::from);

    let mut items: Vec<EpisodeItem> = Vec::new();
    let mut counter: u32 = 0;
    let mut first_release: Option<u64> = None;
    if let Some(secs) = data.get("sections").and_then(Value::as_array) {
        for sec in secs {
            let section_title = sec.get("title").and_then(Value::as_str).map(String::from);
            if let Some(eps) = sec.get("episodes").and_then(Value::as_array) {
                for ep in eps {
                    let aid = ep.get("aid").and_then(Value::as_u64);
                    let cid = ep.get("cid").and_then(Value::as_u64);
                    let ep_id = ep
                        .get("id")
                        .and_then(Value::as_u64)
                        .or_else(|| ep.get("ep_id").and_then(Value::as_u64));
                    let title_ep = ep
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let dur = ep.get("duration").and_then(Value::as_f64);
                    let cover_ep = ep.get("cover").and_then(Value::as_str).map(String::from);
                    let release = ep.get("release_date").and_then(Value::as_u64);
                    if first_release.is_none() && release.is_some() {
                        first_release = release;
                    }
                    let status = ep.get("status").and_then(Value::as_u64).unwrap_or(0);
                    let badge = match status {
                        2 => Some("badge.bilibili.paid_locked".to_string()),
                        3 => Some("badge.bilibili.partial_preview".to_string()),
                        _ => None,
                    };
                    counter += 1;
                    items.push(EpisodeItem {
                        episode_id: format!("cheese:{}", ep_id.unwrap_or(0)),
                        title: title_ep,
                        aid,
                        cid,
                        ep_id,
                        season_id,
                        duration_seconds: dur,
                        cover_url: cover_ep,
                        pub_time_secs: release,
                        episode_number: Some(counter),
                        section_title: section_title.clone(),
                        badge,
                        url: ep_id
                            .map(|id| format!("https://www.bilibili.com/cheese/play/ep{}", id)),
                        ..EpisodeItem::default()
                    });
                }
            }
        }
    }

    if items.is_empty() {
        return Err(BilibiliError::ContentUnavailable);
    }

    let metadata = ContentMetadata {
        series_title: Some(title.clone()),
        season_id,
        uploader,
        uploader_uid,
        uploader_avatar,
        description,
        cover: cover.clone(),
        poster: cover,
        styles: vec!["Bilibili Cheese".to_string()],
        premiered_secs: first_release,
        ..ContentMetadata::default()
    };

    Ok(ParsedContent {
        title,
        items,
        metadata,
        pagination: None,
    })
}
