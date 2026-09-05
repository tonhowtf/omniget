//! Return YouTube Dislike (estudo 44): leitura pública de `GET /votes`.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

const API: &str = "https://returnyoutubedislikeapi.com";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Votes {
    pub id: String,
    #[serde(rename = "dateCreated", default)]
    pub date_created: String,
    #[serde(default)]
    pub likes: u64,
    #[serde(default)]
    pub dislikes: u64,
    #[serde(default)]
    pub rating: f64,
    #[serde(rename = "viewCount", default)]
    pub view_count: u64,
    #[serde(default)]
    pub deleted: bool,
}

pub async fn votes(input: &str) -> anyhow::Result<Votes> {
    let id = super::sponsorblock::video_id(input)
        .ok_or_else(|| anyhow!("nao reconheci um video do YouTube em: {}", input))?;
    let client = super::client()?;
    let resp = client
        .get(format!("{}/votes?videoId={}", API, id))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Return YouTube Dislike: HTTP {}", resp.status()));
    }
    Ok(resp.json().await?)
}
