use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use super::auth::HotmartSession;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DetectedPlayer {
    HotmartNative { m3u8_url: String, name: String },
    Vimeo { embed_url: String },
    PandaVideo { embed_url: String, m3u8_url: String },
    YouTube { video_url: String, video_id: String },
    Unknown { src: String },
}

pub fn detect_players_from_html(html: &str) -> Vec<DetectedPlayer> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("iframe").unwrap();

    let allowlist = ["vimeo", "youtu", "pandavideo", "liquid", "wistia", "videodelivery"];

    doc.select(&sel)
        .filter_map(|el| {
            let src = el.value().attr("src")?;
            if !allowlist.iter().any(|kw| src.contains(kw)) {
                tracing::warn!("iframe desconhecido ignorado: {}", src);
                return None;
            }

            if src.contains("vimeo.com") {
                Some(DetectedPlayer::Vimeo {
                    embed_url: src.to_string(),
                })
            } else if src.contains("pandavideo") {
                let m3u8 = build_panda_m3u8(src)?;
                Some(DetectedPlayer::PandaVideo {
                    embed_url: src.to_string(),
                    m3u8_url: m3u8,
                })
            } else if src.contains("youtu") {
                let id = extract_youtube_id(src);
                Some(DetectedPlayer::YouTube {
                    video_url: src.to_string(),
                    video_id: id,
                })
            } else {
                Some(DetectedPlayer::Unknown {
                    src: src.to_string(),
                })
            }
        })
        .collect()
}

fn build_panda_m3u8(src: &str) -> Option<String> {
    let base = src.replace("player-vz-", "b-vz-");
    let base_part = base.split("/embed/").next()?;
    let video_id = src.split("v=").nth(1)?.split('&').next()?;
    Some(format!("{}/{}/playlist.m3u8", base_part, video_id))
}

fn extract_youtube_id(url: &str) -> String {
    let re = Regex::new(
        r"(?:youtube\.com/(?:watch\?v=|embed/|v/|shorts/)|youtu\.be/)([a-zA-Z0-9_-]{11})",
    )
    .unwrap();
    re.captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_default()
}

pub async fn fetch_player_media_assets(
    player_url: &str,
    session: &HotmartSession,
) -> anyhow::Result<Vec<serde_json::Value>> {
    tracing::info!("[download] Buscando player: {}", &player_url[..80.min(player_url.len())]);

    let resp = session
        .client
        .get(player_url)
        .header("Referer", "https://hotmart.com")
        .send()
        .await?
        .error_for_status()?;

    let html_text = resp.text().await?;
    let doc = Html::parse_document(&html_text);
    let sel = Selector::parse("script#__NEXT_DATA__").unwrap();

    let script_el = doc
        .select(&sel)
        .next()
        .ok_or_else(|| anyhow::anyhow!("__NEXT_DATA__ não encontrado no HTML do player"))?;

    let json_text = script_el.text().collect::<String>();
    let data: serde_json::Value = serde_json::from_str(&json_text)?;

    let assets = data
        .pointer("/props/pageProps/applicationData/mediaAssets")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    tracing::info!("[download] mediaAssets extraídos: {} assets", assets.len());
    Ok(assets)
}
