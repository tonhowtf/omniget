use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use super::api::MediaSrc;
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
    doc.select(&sel)
        .filter_map(|el| {
            let src = el.value().attr("src")?;
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

pub async fn extract_native_urls(
    medias: &[MediaSrc],
    session: &HotmartSession,
) -> Vec<DetectedPlayer> {
    let mut players = Vec::new();

    for media in medias {
        if !media.media_type.eq_ignore_ascii_case("VIDEO") {
            continue;
        }

        tracing::info!("Extraindo URL nativa para: {}", media.media_name);

        let m3u8 = match fetch_native_m3u8(&media.media_src_url, session).await {
            Ok(url) => url,
            Err(e) => {
                tracing::warn!("Falha ao extrair m3u8 de {}: {}", media.media_name, e);
                continue;
            }
        };

        players.push(DetectedPlayer::HotmartNative {
            m3u8_url: m3u8,
            name: media.media_name.clone(),
        });
    }

    players
}

async fn fetch_native_m3u8(media_src_url: &str, session: &HotmartSession) -> anyhow::Result<String> {
    let resp = session
        .client
        .get(media_src_url)
        .header("Authorization", format!("Bearer {}", session.token))
        .send()
        .await?
        .error_for_status()?;

    let html_text = resp.text().await?;
    let doc = Html::parse_document(&html_text);
    let sel = Selector::parse("script#__NEXT_DATA__").unwrap();

    let script_el = doc
        .select(&sel)
        .next()
        .ok_or_else(|| anyhow::anyhow!("script#__NEXT_DATA__ não encontrado"))?;

    let json_text = script_el.text().collect::<String>();
    let data: serde_json::Value = serde_json::from_str(&json_text)?;

    let url = data
        .pointer("/props/pageProps/applicationData/mediaAssets")
        .and_then(|arr| arr.as_array())
        .and_then(|arr| arr.first())
        .and_then(|asset| asset.get("url"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("mediaAssets[0].url não encontrado no __NEXT_DATA__"))?;

    tracing::info!("m3u8 nativo extraído: {}...", &url[..60.min(url.len())]);

    Ok(url.to_string())
}
