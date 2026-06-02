use serde_json::Value;

use super::super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::super::url_kind::av_to_bv;
use super::super::wbi;
use super::{ContentMetadata, EpisodeItem, ParsedContent};

const VIEW_URL: &str = "https://api.bilibili.com/x/web-interface/wbi/view";

pub async fn parse(
    client: &ApiClient,
    bvid_or_av: &str,
    page_filter: Option<u32>,
) -> Result<ParsedContent> {
    let bvid = normalize_bvid(bvid_or_av).ok_or(BilibiliError::ContentUnavailable)?;
    let params: Vec<(&str, String)> = vec![("bvid", bvid.clone())];
    let signed = wbi::signed_query(client, &params).await?;
    let url = format!("{}?{}", VIEW_URL, signed);
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;

    let title = data
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Bilibili Video")
        .to_string();
    let cover = data.get("pic").and_then(Value::as_str).map(String::from);
    let uploader = data
        .get("owner")
        .and_then(|o| o.get("name"))
        .and_then(Value::as_str)
        .map(String::from);
    let uploader_uid = data
        .get("owner")
        .and_then(|o| o.get("mid"))
        .and_then(Value::as_u64);
    let uploader_avatar = data
        .get("owner")
        .and_then(|o| o.get("face"))
        .and_then(Value::as_str)
        .map(String::from);
    let description = data.get("desc").and_then(Value::as_str).map(String::from);
    let pub_time = data.get("pubdate").and_then(Value::as_u64);
    let bvid_val = data
        .get("bvid")
        .and_then(Value::as_str)
        .unwrap_or(&bvid)
        .to_string();
    let aid = data.get("aid").and_then(Value::as_u64);
    let badge = if data
        .get("is_upower_exclusive")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Some("badge.bilibili.charging_exclusive".to_string())
    } else {
        None
    };

    let mut metadata = ContentMetadata {
        uploader,
        uploader_uid,
        uploader_avatar,
        description,
        cover: cover.clone(),
        premiered_secs: pub_time,
        ..ContentMetadata::default()
    };

    if let Some(season) = data.get("ugc_season").and_then(Value::as_object) {
        let series_title = season
            .get("title")
            .and_then(Value::as_str)
            .map(String::from);
        metadata.collection_title = series_title.clone();
        metadata.series_title = series_title;
        let sections = season.get("sections").and_then(Value::as_array);
        let mut items: Vec<EpisodeItem> = Vec::new();
        if let Some(secs) = sections {
            for sec in secs {
                let section_title = sec.get("title").and_then(Value::as_str).map(String::from);
                if let Some(eps) = sec.get("episodes").and_then(Value::as_array) {
                    for (i, ep) in eps.iter().enumerate() {
                        let ep_bvid = ep
                            .get("bvid")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let ep_aid = ep.get("aid").and_then(Value::as_u64);
                        let ep_cid = ep.get("cid").and_then(Value::as_u64);
                        let ep_title = ep
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let arc = ep.get("arc");
                        let dur = arc.and_then(|a| a.get("duration")).and_then(Value::as_f64);
                        let cover_ep = arc
                            .and_then(|a| a.get("pic"))
                            .and_then(Value::as_str)
                            .map(String::from);
                        let pub_ep = arc.and_then(|a| a.get("pubdate")).and_then(Value::as_u64);
                        items.push(EpisodeItem {
                            episode_id: format!("ugc:{}", ep_bvid),
                            title: ep_title,
                            aid: ep_aid,
                            bvid: Some(ep_bvid.clone()),
                            cid: ep_cid,
                            duration_seconds: dur,
                            cover_url: cover_ep,
                            pub_time_secs: pub_ep,
                            episode_number: Some((i as u32) + 1),
                            section_title: section_title.clone(),
                            url: Some(format!("https://www.bilibili.com/video/{}", ep_bvid)),
                            ..EpisodeItem::default()
                        });
                    }
                }
            }
        }
        if !items.is_empty() {
            return Ok(ParsedContent {
                title,
                items,
                metadata,
                pagination: None,
            });
        }
    }

    let pages = data.get("pages").and_then(Value::as_array);
    if let Some(pgs) = pages {
        if pgs.len() > 1 || page_filter.is_some() {
            let mut items: Vec<EpisodeItem> = Vec::new();
            for p in pgs {
                let page_num = p.get("page").and_then(Value::as_u64).unwrap_or(0) as u32;
                if let Some(filter) = page_filter {
                    if filter != page_num {
                        continue;
                    }
                }
                let cid = p.get("cid").and_then(Value::as_u64);
                let part = p
                    .get("part")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let dur = p.get("duration").and_then(Value::as_f64);
                items.push(EpisodeItem {
                    episode_id: format!("page:{}:{}", bvid_val, page_num),
                    title: if part.is_empty() {
                        title.clone()
                    } else {
                        part.clone()
                    },
                    aid,
                    bvid: Some(bvid_val.clone()),
                    cid,
                    duration_seconds: dur,
                    cover_url: cover.clone(),
                    pub_time_secs: pub_time,
                    page: Some(page_num),
                    page_title: Some(part),
                    badge: badge.clone(),
                    url: Some(format!(
                        "https://www.bilibili.com/video/{}?p={}",
                        bvid_val, page_num
                    )),
                    ..EpisodeItem::default()
                });
            }
            if !items.is_empty() {
                return Ok(ParsedContent {
                    title: title.clone(),
                    items,
                    metadata,
                    pagination: None,
                });
            }
        }
    }

    let cid = data.get("cid").and_then(Value::as_u64).or_else(|| {
        pages
            .and_then(|p| p.first())
            .and_then(|p| p.get("cid"))
            .and_then(Value::as_u64)
    });
    let duration = data.get("duration").and_then(Value::as_f64);
    let item = EpisodeItem {
        episode_id: format!("video:{}", bvid_val),
        title: title.clone(),
        aid,
        bvid: Some(bvid_val.clone()),
        cid,
        duration_seconds: duration,
        cover_url: cover,
        pub_time_secs: pub_time,
        badge,
        url: Some(format!("https://www.bilibili.com/video/{}", bvid_val)),
        ..EpisodeItem::default()
    };
    Ok(ParsedContent::single_with_meta(title, item, metadata))
}

fn normalize_bvid(input: &str) -> Option<String> {
    if input.starts_with("BV") {
        return Some(input.to_string());
    }
    if let Some(rest) = input.strip_prefix("av") {
        if let Ok(n) = rest.parse::<u64>() {
            return Some(av_to_bv(n));
        }
    }
    None
}

impl ParsedContent {
    pub fn single_with_meta(title: String, item: EpisodeItem, metadata: ContentMetadata) -> Self {
        Self {
            title,
            items: vec![item],
            metadata,
            pagination: None,
        }
    }
}
