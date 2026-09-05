//! SponsorBlock (estudo 43): segmentos por prefixo de hash, como a extensão
//! faz para não mandar o ID do vídeo ao servidor.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SERVER: &str = "https://sponsor.ajay.app";
pub const CATEGORIES: &[&str] = &[
    "sponsor",
    "selfpromo",
    "interaction",
    "intro",
    "outro",
    "preview",
    "music_offtopic",
    "filler",
    "exclusive_access",
    "poi_highlight",
    "chapter",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    #[serde(rename = "UUID", default)]
    pub uuid: String,
    pub segment: [f64; 2],
    pub category: String,
    #[serde(rename = "actionType", default)]
    pub action_type: String,
    #[serde(default)]
    pub votes: i64,
    #[serde(default)]
    pub locked: i64,
    #[serde(rename = "videoDuration", default)]
    pub video_duration: f64,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SponsorResult {
    pub video_id: String,
    pub segments: Vec<Segment>,
    pub skipped_seconds: f64,
    /// Argumentos equivalentes do yt-dlp para baixar sem os trechos.
    pub ytdlp_args: String,
}

/// Aceita URL completa, `youtu.be/ID`, `shorts/ID` ou o próprio ID.
pub fn video_id(input: &str) -> Option<String> {
    let s = input.trim();
    if s.len() == 11
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Some(s.to_string());
    }
    let re =
        regex::Regex::new(r"(?:v=|youtu\.be/|shorts/|embed/|live/)([A-Za-z0-9_-]{11})").ok()?;
    re.captures(s)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

pub async fn segments(input: &str, categories: &[String]) -> anyhow::Result<SponsorResult> {
    let id = video_id(input)
        .ok_or_else(|| anyhow!("nao reconheci um video do YouTube em: {}", input))?;
    let cats: Vec<&str> = if categories.is_empty() {
        CATEGORIES.to_vec()
    } else {
        categories.iter().map(|s| s.as_str()).collect()
    };
    let mut h = Sha256::new();
    h.update(id.as_bytes());
    let prefix = &hex::encode(h.finalize())[..4];
    let url = format!(
        "{}/api/skipSegments/{}?categories={}&actionTypes={}",
        SERVER,
        prefix,
        urlencoding::encode(&serde_json::to_string(&cats)?),
        urlencoding::encode(r#"["skip","mute","full","poi","chapter"]"#)
    );
    let client = super::client()?;
    let resp = client.get(&url).send().await?;
    let mut segs: Vec<Segment> = Vec::new();
    if resp.status().as_u16() == 404 {
        // sem segmentos para esse prefixo
    } else if resp.status().is_success() {
        let arr: Vec<serde_json::Value> = resp.json().await?;
        for v in arr {
            if v["videoID"].as_str() == Some(id.as_str()) {
                if let Some(list) = v["segments"].as_array() {
                    segs.extend(
                        list.iter()
                            .filter_map(|s| serde_json::from_value(s.clone()).ok()),
                    );
                }
            }
        }
    } else {
        return Err(anyhow!("SponsorBlock: HTTP {}", resp.status()));
    }
    segs.sort_by(|a, b| {
        a.segment[0]
            .partial_cmp(&b.segment[0])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let skipped: f64 = segs
        .iter()
        .filter(|s| s.action_type == "skip" || s.action_type.is_empty())
        .map(|s| (s.segment[1] - s.segment[0]).max(0.0))
        .sum();
    let mut used: Vec<&str> = segs
        .iter()
        .filter(|s| s.action_type != "chapter" && s.action_type != "poi")
        .map(|s| s.category.as_str())
        .collect();
    used.sort();
    used.dedup();
    let ytdlp_args = if used.is_empty() {
        String::new()
    } else {
        format!("--sponsorblock-remove {}", used.join(","))
    };
    Ok(SponsorResult {
        video_id: id,
        segments: segs,
        skipped_seconds: skipped,
        ytdlp_args,
    })
}

#[cfg(test)]
mod tests {
    use super::video_id;

    #[test]
    fn extracts_ids() {
        assert_eq!(
            video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(
            video_id("https://youtu.be/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(video_id("dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(video_id("https://example.com"), None);
    }
}
