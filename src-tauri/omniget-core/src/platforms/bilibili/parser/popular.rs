use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::super::wbi;
use super::{ContentMetadata, EpisodeItem, ParsedContent};

const POPULAR_URL: &str = "https://api.bilibili.com/x/web-interface/popular/series/one";

pub async fn parse(client: &ApiClient, num: u32) -> Result<ParsedContent> {
    let params: Vec<(&str, String)> = vec![
        ("number", num.to_string()),
        ("web_location", "333.934".to_string()),
    ];
    let signed = wbi::signed_query(client, &params).await?;
    let url = format!("{}?{}", POPULAR_URL, signed);
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;

    let label = data
        .get("config")
        .and_then(|c| c.get("label"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let title = if label.is_empty() {
        format!("Bilibili Popular #{}", num)
    } else {
        label
    };

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
        let title_v = v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let cover = v.get("pic").and_then(Value::as_str).map(String::from);
        let duration = v.get("duration").and_then(Value::as_f64);
        let pub_time = v.get("pubdate").and_then(Value::as_u64);
        items.push(EpisodeItem {
            episode_id: format!("popular:{}:{}", num, bvid),
            title: title_v,
            aid,
            bvid: Some(bvid.clone()),
            cid,
            duration_seconds: duration,
            cover_url: cover,
            pub_time_secs: pub_time,
            url: Some(format!("https://www.bilibili.com/video/{}", bvid)),
            ..EpisodeItem::default()
        });
    }

    if items.is_empty() {
        return Err(BilibiliError::ContentUnavailable);
    }

    Ok(ParsedContent {
        title,
        items,
        metadata: ContentMetadata::default(),
        pagination: None,
    })
}
