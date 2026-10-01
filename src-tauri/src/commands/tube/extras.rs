use std::collections::HashMap;

use futures::StreamExt;
use omniget_core::core::tube::extras as core_extras;
use omniget_core::core::tube::{DeArrow, Ryd, SbSegment};
use tauri::AppHandle;

use super::db;

const SB_TTL_MS: i64 = 3_600_000;
const DEARROW_TTL_MS: i64 = 3_600_000;
const RYD_TTL_MS: i64 = 30 * 60_000;
const DEARROW_PARALLEL: usize = 6;
const DEARROW_MAX: usize = 60;

fn cached<T: serde::de::DeserializeOwned>(key: &str, ttl: i64) -> Option<T> {
    let json = db::with(|c| Ok(db::cache_get(c, key, ttl, db::now_ms()))).ok()??;
    serde_json::from_str(&json).ok()
}

fn store<T: serde::Serialize>(key: &str, value: &T) {
    if let Ok(json) = serde_json::to_string(value) {
        let _ = db::with(|c| db::cache_put(c, key, &json, db::now_ms()));
    }
}

fn is_not_found(err: &str) -> bool {
    err.contains("404")
}

pub fn active_categories(map: &std::collections::BTreeMap<String, String>) -> Vec<String> {
    map.iter()
        .filter(|(_, mode)| mode.as_str() != "off")
        .map(|(cat, _)| cat.clone())
        .collect()
}

#[tauri::command]
pub async fn tube_sponsorblock(
    app: AppHandle,
    video_id: String,
    channel_id: Option<String>,
) -> Result<Vec<SbSegment>, String> {
    let s = crate::storage::config::load_settings(&app).tube;
    if !s.sponsorblock {
        return Ok(Vec::new());
    }
    if channel_id
        .as_deref()
        .is_some_and(|id| s.sb_whitelist.iter().any(|w| w == id))
    {
        return Ok(Vec::new());
    }
    let cats = active_categories(&s.sb_categories);
    if cats.is_empty() {
        return Ok(Vec::new());
    }
    let server = s.sb_server.trim_end_matches('/').to_string();
    let key = format!("sb:{server}:{video_id}:{}", cats.join(","));
    if let Some(hit) = cached::<Vec<SbSegment>>(&key, SB_TTL_MS) {
        return Ok(hit);
    }
    let refs: Vec<&str> = cats.iter().map(String::as_str).collect();
    let url = core_extras::sb_url(&server, &video_id, &refs);
    let segments = match super::http::get_json(&url).await {
        Ok(json) => core_extras::parse_sb(&json, &video_id),
        Err(e) if is_not_found(&e) => Vec::new(),
        Err(e) => return Err(e),
    };
    store(&key, &segments);
    Ok(segments)
}

async fn dearrow_one(video_id: String) -> Option<(String, DeArrow)> {
    let key = format!("dearrow:{video_id}");
    if let Some(hit) = cached::<DeArrow>(&key, DEARROW_TTL_MS) {
        return Some((video_id, hit));
    }
    let branding = match super::http::get_json(&core_extras::dearrow_url(&video_id)).await {
        Ok(json) => core_extras::parse_dearrow(&json, &video_id),
        Err(e) if is_not_found(&e) => DeArrow::default(),
        Err(e) => {
            tracing::debug!("[tube] dearrow failed: {e}");
            return None;
        }
    };
    store(&key, &branding);
    Some((video_id, branding))
}

#[tauri::command]
pub async fn tube_dearrow(
    app: AppHandle,
    video_ids: Vec<String>,
) -> Result<HashMap<String, DeArrow>, String> {
    let s = crate::storage::config::load_settings(&app).tube;
    if !s.dearrow {
        return Ok(HashMap::new());
    }
    let mut ids = video_ids;
    ids.sort();
    ids.dedup();
    ids.truncate(DEARROW_MAX);
    let results: Vec<Option<(String, DeArrow)>> = futures::stream::iter(ids)
        .map(dearrow_one)
        .buffer_unordered(DEARROW_PARALLEL)
        .collect()
        .await;
    Ok(results
        .into_iter()
        .flatten()
        .filter_map(|(id, mut d)| {
            if !s.dearrow_thumbs {
                d.thumb_url = None;
                d.thumb_time_s = None;
            }
            (d.title.is_some() || d.thumb_url.is_some()).then_some((id, d))
        })
        .collect())
}

#[tauri::command]
pub async fn tube_ryd(app: AppHandle, video_id: String) -> Result<Option<Ryd>, String> {
    let s = crate::storage::config::load_settings(&app).tube;
    if !s.ryd {
        return Ok(None);
    }
    let key = format!("ryd:{video_id}");
    if let Some(hit) = cached::<Option<Ryd>>(&key, RYD_TTL_MS) {
        return Ok(hit);
    }
    let ryd = match super::http::get_json(&core_extras::ryd_url(&video_id)).await {
        Ok(json) => core_extras::parse_ryd(&json),
        Err(e) if is_not_found(&e) => None,
        Err(e) => return Err(e),
    };
    store(&key, &ryd);
    Ok(ryd)
}
