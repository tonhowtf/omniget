use super::super::api::{ApiClient, BilibiliError, Result};
use super::super::url_kind::extract_festival_bvid;
use super::video;
use super::ParsedContent;

pub async fn parse(client: &ApiClient, url: &str) -> Result<ParsedContent> {
    let html = String::from_utf8(client.get_bytes(url).await?)
        .map_err(|_| BilibiliError::ContentUnavailable)?;
    let bvid = extract_festival_bvid(&html).ok_or(BilibiliError::ContentUnavailable)?;
    video::parse(client, &bvid, None).await
}
