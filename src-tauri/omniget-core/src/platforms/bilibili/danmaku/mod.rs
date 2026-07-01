use serde::{Deserialize, Serialize};

use super::api::{ApiClient, BilibiliError, Result};
use super::wbi;

pub mod ass;
pub mod json;
pub mod proto;
pub mod xml;

pub use proto::DanmakuElem;

const SEG_URL: &str = "https://api.bilibili.com/x/v2/dm/wbi/web/seg.so";
const SEGMENT_DURATION_SECS: u64 = 360;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DanmakuFormat {
    Xml,
    Ass,
    Json,
}

impl DanmakuFormat {
    pub fn extension(self) -> &'static str {
        match self {
            DanmakuFormat::Xml => "xml",
            DanmakuFormat::Ass => "ass",
            DanmakuFormat::Json => "json",
        }
    }
}

pub async fn fetch_elems(
    client: &ApiClient,
    cid: u64,
    duration_secs: u64,
) -> Result<Vec<DanmakuElem>> {
    let segments = ((duration_secs + SEGMENT_DURATION_SECS - 1) / SEGMENT_DURATION_SECS).max(1);
    let mut all: Vec<DanmakuElem> = Vec::new();
    for i in 1..=segments {
        let params: Vec<(&str, String)> = vec![
            ("type", "1".to_string()),
            ("oid", cid.to_string()),
            ("segment_index", i.to_string()),
        ];
        let signed = wbi::signed_query(client, &params).await?;
        let url = format!("{}?{}", SEG_URL, signed);
        match client.get_bytes(&url).await {
            Ok(bytes) => {
                if let Ok(elems) = proto::decode_segment(&bytes) {
                    all.extend(elems.into_iter().filter(|e| !e.content.is_empty()));
                }
            }
            Err(e) => {
                tracing::warn!(
                    "[bilibili] danmaku segment {}/{} fetch failed: {:?}",
                    i,
                    segments,
                    e
                );
            }
        }
    }
    if all.is_empty() {
        return Err(BilibiliError::ContentUnavailable);
    }
    all.sort_by_key(|e| e.progress_ms);
    Ok(all)
}

pub fn render(elems: &[DanmakuElem], format: DanmakuFormat) -> String {
    match format {
        DanmakuFormat::Xml => xml::render(elems),
        DanmakuFormat::Json => json::render(elems),
        DanmakuFormat::Ass => ass::render(elems, &ass::AssRenderOptions::default()),
    }
}
