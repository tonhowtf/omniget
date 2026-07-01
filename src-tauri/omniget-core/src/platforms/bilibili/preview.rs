use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::{check_api_response, ApiClient, BilibiliError, Result};
use super::parser::EpisodeItem;
use super::url_kind::UrlKind;
use super::wbi;

const UGC_PLAYURL: &str = "https://api.bilibili.com/x/player/wbi/playurl";
const PGC_PLAYURL: &str = "https://api.bilibili.com/pgc/player/web/playurl";
const CHEESE_PLAYURL: &str = "https://api.bilibili.com/pugv/player/web/playurl";

pub const QN_AUTO: u32 = 200;
pub const QN_8K: u32 = 127;
pub const QN_DOLBY_VISION: u32 = 126;
pub const QN_HDR: u32 = 125;
pub const QN_4K: u32 = 120;
pub const QN_1080P60: u32 = 116;
pub const QN_1080P_PLUS: u32 = 112;
pub const QN_AI: u32 = 100;
pub const QN_1080P: u32 = 80;
pub const QN_720P: u32 = 64;
pub const QN_480P: u32 = 32;
pub const QN_360P: u32 = 16;

pub const CODEC_AUTO: u32 = 20;
pub const CODEC_AVC: u32 = 7;
pub const CODEC_HEVC: u32 = 12;
pub const CODEC_AV1: u32 = 13;

pub const AUDIO_AUTO: u32 = 30300;
pub const AUDIO_HIRES: u32 = 30251;
pub const AUDIO_DOLBY_ATMOS: u32 = 30250;
pub const AUDIO_192K: u32 = 30280;
pub const AUDIO_132K: u32 = 30232;
pub const AUDIO_64K: u32 = 30216;

const DEFAULT_VIDEO_QUALITY_PRIORITY: &[u32] = &[
    QN_8K,
    QN_DOLBY_VISION,
    QN_HDR,
    QN_4K,
    QN_1080P60,
    QN_1080P_PLUS,
    QN_1080P,
    QN_720P,
    QN_480P,
    QN_360P,
];

const DEFAULT_CODEC_PRIORITY: &[u32] = &[CODEC_AV1, CODEC_HEVC, CODEC_AVC];

const DEFAULT_AUDIO_PRIORITY: &[u32] = &[
    AUDIO_HIRES,
    AUDIO_DOLBY_ATMOS,
    AUDIO_192K,
    AUDIO_132K,
    AUDIO_64K,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaContainer {
    Dash,
    Mp4,
    Flv,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoStream {
    pub qn: u32,
    pub codec_id: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<String>,
    pub bandwidth: Option<u64>,
    pub base_url: String,
    pub backup_urls: Vec<String>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStream {
    pub qn: u32,
    pub codec: Option<String>,
    pub bandwidth: Option<u64>,
    pub base_url: String,
    pub backup_urls: Vec<String>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewInfo {
    pub container: MediaContainer,
    pub video_streams: Vec<VideoStream>,
    pub audio_streams: Vec<AudioStream>,
    pub accepted_qns: Vec<u32>,
    pub accepted_qn_labels: Vec<String>,
    pub timelength_ms: Option<u64>,
    pub premium_required: bool,
}

impl PreviewInfo {
    pub fn available_qns(&self) -> Vec<u32> {
        let mut set: BTreeMap<u32, ()> = BTreeMap::new();
        for v in &self.video_streams {
            set.insert(v.qn, ());
        }
        set.into_keys().rev().collect()
    }

    pub fn available_codecs_for(&self, qn: u32) -> Vec<u32> {
        let mut codecs: Vec<u32> = self
            .video_streams
            .iter()
            .filter(|v| v.qn == qn)
            .map(|v| v.codec_id)
            .collect();
        codecs.sort_unstable();
        codecs.dedup();
        codecs
    }

    pub fn available_audio_qns(&self) -> Vec<u32> {
        let mut set: BTreeMap<u32, ()> = BTreeMap::new();
        for a in &self.audio_streams {
            set.insert(a.qn, ());
        }
        set.into_keys().rev().collect()
    }

    pub fn pick_video(&self, qn_pref: u32, codec_pref: u32) -> Option<&VideoStream> {
        let qn = if qn_pref == QN_AUTO {
            self.video_streams
                .iter()
                .map(|v| v.qn)
                .filter(|q| DEFAULT_VIDEO_QUALITY_PRIORITY.contains(q))
                .max_by_key(|q| {
                    DEFAULT_VIDEO_QUALITY_PRIORITY
                        .iter()
                        .rev()
                        .position(|p| p == q)
                        .unwrap_or(0)
                })
                .or_else(|| self.video_streams.iter().map(|v| v.qn).max())?
        } else if self.video_streams.iter().any(|v| v.qn == qn_pref) {
            qn_pref
        } else {
            self.video_streams.iter().map(|v| v.qn).max()?
        };

        if codec_pref == CODEC_AUTO {
            for &c in DEFAULT_CODEC_PRIORITY {
                if let Some(s) = self
                    .video_streams
                    .iter()
                    .find(|v| v.qn == qn && v.codec_id == c)
                {
                    return Some(s);
                }
            }
            return self.video_streams.iter().find(|v| v.qn == qn);
        }
        if let Some(s) = self
            .video_streams
            .iter()
            .find(|v| v.qn == qn && v.codec_id == codec_pref)
        {
            return Some(s);
        }
        self.video_streams.iter().find(|v| v.qn == qn)
    }

    pub fn pick_audio(&self, qn_pref: u32) -> Option<&AudioStream> {
        if qn_pref == AUDIO_AUTO {
            for &q in DEFAULT_AUDIO_PRIORITY {
                if let Some(s) = self.audio_streams.iter().find(|a| a.qn == q) {
                    return Some(s);
                }
            }
            return self.audio_streams.first();
        }
        if let Some(s) = self.audio_streams.iter().find(|a| a.qn == qn_pref) {
            return Some(s);
        }
        self.audio_streams.first()
    }
}

pub fn qn_label(qn: u32) -> &'static str {
    match qn {
        QN_8K => "platforms.bilibili.qn.8k",
        QN_DOLBY_VISION => "platforms.bilibili.qn.dolby_vision",
        QN_HDR => "platforms.bilibili.qn.hdr",
        QN_4K => "platforms.bilibili.qn.4k",
        QN_1080P60 => "platforms.bilibili.qn.1080p60",
        QN_1080P_PLUS => "platforms.bilibili.qn.1080p_plus",
        QN_AI => "platforms.bilibili.qn.ai",
        QN_1080P => "platforms.bilibili.qn.1080p",
        QN_720P => "platforms.bilibili.qn.720p",
        QN_480P => "platforms.bilibili.qn.480p",
        QN_360P => "platforms.bilibili.qn.360p",
        _ => "platforms.bilibili.qn.unknown",
    }
}

pub fn codec_label(codec: u32) -> &'static str {
    match codec {
        CODEC_AVC => "platforms.bilibili.codec.avc",
        CODEC_HEVC => "platforms.bilibili.codec.hevc",
        CODEC_AV1 => "platforms.bilibili.codec.av1",
        _ => "platforms.bilibili.codec.unknown",
    }
}

pub fn audio_qn_label(qn: u32) -> &'static str {
    match qn {
        AUDIO_HIRES => "platforms.bilibili.audio.hires",
        AUDIO_DOLBY_ATMOS => "platforms.bilibili.audio.dolby_atmos",
        AUDIO_192K => "platforms.bilibili.audio.192k",
        AUDIO_132K => "platforms.bilibili.audio.132k",
        AUDIO_64K => "platforms.bilibili.audio.64k",
        _ => "platforms.bilibili.audio.unknown",
    }
}

pub async fn fetch(client: &ApiClient, item: &EpisodeItem, kind: &UrlKind) -> Result<PreviewInfo> {
    match kind {
        UrlKind::BangumiEpisode { .. }
        | UrlKind::BangumiSeason { .. }
        | UrlKind::BangumiMedia { .. } => fetch_pgc(client, item).await,
        UrlKind::CheeseEpisode { .. } | UrlKind::CheeseSeason { .. } => {
            fetch_cheese(client, item).await
        }
        _ => fetch_ugc(client, item).await,
    }
}

async fn fetch_ugc(client: &ApiClient, item: &EpisodeItem) -> Result<PreviewInfo> {
    let bvid = item
        .bvid
        .as_deref()
        .ok_or(BilibiliError::ContentUnavailable)?;
    let cid = item.cid.ok_or(BilibiliError::ContentUnavailable)?;
    let params: Vec<(&str, String)> = vec![
        ("bvid", bvid.to_string()),
        ("cid", cid.to_string()),
        ("qn", QN_1080P.to_string()),
        ("fnver", "0".to_string()),
        ("fnval", "4048".to_string()),
        ("fourk", "1".to_string()),
    ];
    let signed = wbi::signed_query(client, &params).await?;
    let url = format!("{}?{}", UGC_PLAYURL, signed);
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;
    build_preview(data)
}

async fn fetch_pgc(client: &ApiClient, item: &EpisodeItem) -> Result<PreviewInfo> {
    let bvid = item
        .bvid
        .as_deref()
        .ok_or(BilibiliError::ContentUnavailable)?;
    let cid = item.cid.ok_or(BilibiliError::ContentUnavailable)?;
    let url = format!(
        "{}?bvid={}&cid={}&qn={}&fnver=0&fnval=12240&fourk=1",
        PGC_PLAYURL, bvid, cid, QN_1080P
    );
    let raw = client.get_json(&url).await?;
    let result = check_api_response(&raw)?;
    build_preview(result)
}

async fn fetch_cheese(client: &ApiClient, item: &EpisodeItem) -> Result<PreviewInfo> {
    let aid = item.aid.ok_or(BilibiliError::ContentUnavailable)?;
    let cid = item.cid.ok_or(BilibiliError::ContentUnavailable)?;
    let ep_id = item.ep_id.unwrap_or(0);
    let url = format!(
        "{}?avid={}&cid={}&qn={}&fnver=0&fnval=4048&fourk=1&ep_id={}",
        CHEESE_PLAYURL, aid, cid, QN_1080P, ep_id
    );
    let raw = client.get_json(&url).await?;
    let data = check_api_response(&raw)?;
    build_preview(data)
}

fn build_preview(data: &Value) -> Result<PreviewInfo> {
    let timelength_ms = data.get("timelength").and_then(Value::as_u64);
    let accepted_qns: Vec<u32> = data
        .get("accept_quality")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_u64().map(|n| n as u32))
                .collect()
        })
        .unwrap_or_default();
    let accepted_qn_labels: Vec<String> = data
        .get("accept_description")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if let Some(dash) = data.get("dash").and_then(Value::as_object) {
        let video_streams = parse_dash_video(dash);
        let audio_streams = parse_dash_audio(dash);
        let premium_required = is_premium_required(&accepted_qns, &video_streams);
        return Ok(PreviewInfo {
            container: MediaContainer::Dash,
            video_streams,
            audio_streams,
            accepted_qns,
            accepted_qn_labels,
            timelength_ms,
            premium_required,
        });
    }

    if let Some(durl) = data.get("durl").and_then(Value::as_array) {
        let format = data.get("format").and_then(Value::as_str).unwrap_or("mp4");
        let container = if format.starts_with("flv") {
            MediaContainer::Flv
        } else {
            MediaContainer::Mp4
        };
        let quality = data.get("quality").and_then(Value::as_u64).unwrap_or(80) as u32;
        let video_streams: Vec<VideoStream> = durl
            .iter()
            .filter_map(|d| {
                let url = d.get("url").and_then(Value::as_str)?.to_string();
                let backups = d
                    .get("backup_url")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                Some(VideoStream {
                    qn: quality,
                    codec_id: CODEC_AVC,
                    width: None,
                    height: None,
                    frame_rate: None,
                    bandwidth: None,
                    base_url: url,
                    backup_urls: backups,
                    mime_type: Some(format!("video/{}", format)),
                })
            })
            .collect();
        return Ok(PreviewInfo {
            container,
            video_streams,
            audio_streams: Vec::new(),
            accepted_qns,
            accepted_qn_labels,
            timelength_ms,
            premium_required: false,
        });
    }

    Err(BilibiliError::ContentUnavailable)
}

fn parse_dash_video(dash: &serde_json::Map<String, Value>) -> Vec<VideoStream> {
    let mut streams: Vec<VideoStream> = Vec::new();
    let arr = match dash.get("video").and_then(Value::as_array) {
        Some(a) => a,
        None => return streams,
    };
    for entry in arr {
        let qn = entry.get("id").and_then(Value::as_u64).unwrap_or(0) as u32;
        let codec_id = entry.get("codecid").and_then(Value::as_u64).unwrap_or(0) as u32;
        let width = entry.get("width").and_then(Value::as_u64).map(|n| n as u32);
        let height = entry
            .get("height")
            .and_then(Value::as_u64)
            .map(|n| n as u32);
        let frame_rate = entry
            .get("frame_rate")
            .or_else(|| entry.get("frameRate"))
            .and_then(Value::as_str)
            .map(String::from);
        let bandwidth = entry.get("bandwidth").and_then(Value::as_u64);
        let base_url = entry
            .get("baseUrl")
            .or_else(|| entry.get("base_url"))
            .and_then(Value::as_str)
            .map(String::from);
        let backup_urls = entry
            .get("backupUrl")
            .or_else(|| entry.get("backup_url"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let mime_type = entry
            .get("mimeType")
            .or_else(|| entry.get("mime_type"))
            .and_then(Value::as_str)
            .map(String::from);
        if let Some(url) = base_url {
            streams.push(VideoStream {
                qn,
                codec_id,
                width,
                height,
                frame_rate,
                bandwidth,
                base_url: url,
                backup_urls,
                mime_type,
            });
        }
    }
    streams
}

fn parse_dash_audio(dash: &serde_json::Map<String, Value>) -> Vec<AudioStream> {
    let mut streams: Vec<AudioStream> = Vec::new();
    if let Some(flac) = dash
        .get("flac")
        .and_then(Value::as_object)
        .and_then(|o| o.get("audio"))
    {
        if let Some(s) = audio_entry(flac, "audio/flac") {
            streams.push(s);
        }
    }
    if let Some(dolby_arr) = dash
        .get("dolby")
        .and_then(Value::as_object)
        .and_then(|o| o.get("audio"))
        .and_then(Value::as_array)
    {
        for entry in dolby_arr {
            if let Some(s) = audio_entry(entry, "audio/eac3") {
                streams.push(s);
            }
        }
    }
    if let Some(arr) = dash.get("audio").and_then(Value::as_array) {
        for entry in arr {
            if let Some(s) = audio_entry(entry, "audio/mp4") {
                streams.push(s);
            }
        }
    }
    streams
}

fn audio_entry(entry: &Value, default_mime: &str) -> Option<AudioStream> {
    let qn = entry.get("id").and_then(Value::as_u64).unwrap_or(0) as u32;
    let codec = entry
        .get("codecs")
        .or_else(|| entry.get("codec"))
        .and_then(Value::as_str)
        .map(String::from);
    let bandwidth = entry.get("bandwidth").and_then(Value::as_u64);
    let base_url = entry
        .get("baseUrl")
        .or_else(|| entry.get("base_url"))
        .and_then(Value::as_str)
        .map(String::from)?;
    let backup_urls = entry
        .get("backupUrl")
        .or_else(|| entry.get("backup_url"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let mime_type = entry
        .get("mimeType")
        .or_else(|| entry.get("mime_type"))
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| Some(default_mime.to_string()));
    Some(AudioStream {
        qn,
        codec,
        bandwidth,
        base_url,
        backup_urls,
        mime_type,
    })
}

fn is_premium_required(accepted_qns: &[u32], video_streams: &[VideoStream]) -> bool {
    let has_high = accepted_qns
        .iter()
        .any(|q| *q == QN_4K || *q == QN_8K || *q == QN_HDR || *q == QN_DOLBY_VISION);
    if !has_high {
        return false;
    }
    let downloaded_high = video_streams
        .iter()
        .any(|v| v.qn == QN_4K || v.qn == QN_8K || v.qn == QN_HDR || v.qn == QN_DOLBY_VISION);
    !downloaded_high
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_stream(qn: u32, codec: u32) -> VideoStream {
        VideoStream {
            qn,
            codec_id: codec,
            width: None,
            height: None,
            frame_rate: None,
            bandwidth: None,
            base_url: "https://example/x.m4s".to_string(),
            backup_urls: vec![],
            mime_type: None,
        }
    }

    #[test]
    fn pick_video_auto_prefers_higher_quality_and_av1() {
        let info = PreviewInfo {
            container: MediaContainer::Dash,
            video_streams: vec![
                dummy_stream(QN_1080P, CODEC_AVC),
                dummy_stream(QN_1080P, CODEC_HEVC),
                dummy_stream(QN_1080P, CODEC_AV1),
                dummy_stream(QN_720P, CODEC_AV1),
            ],
            audio_streams: vec![],
            accepted_qns: vec![],
            accepted_qn_labels: vec![],
            timelength_ms: None,
            premium_required: false,
        };
        let picked = info.pick_video(QN_AUTO, CODEC_AUTO).expect("picked");
        assert_eq!(picked.qn, QN_1080P);
        assert_eq!(picked.codec_id, CODEC_AV1);
    }

    #[test]
    fn pick_video_specific_qn_codec() {
        let info = PreviewInfo {
            container: MediaContainer::Dash,
            video_streams: vec![
                dummy_stream(QN_720P, CODEC_AVC),
                dummy_stream(QN_720P, CODEC_HEVC),
            ],
            audio_streams: vec![],
            accepted_qns: vec![],
            accepted_qn_labels: vec![],
            timelength_ms: None,
            premium_required: false,
        };
        let picked = info.pick_video(QN_720P, CODEC_HEVC).expect("picked");
        assert_eq!(picked.codec_id, CODEC_HEVC);
    }

    #[test]
    fn pick_audio_auto_prefers_hires() {
        let info = PreviewInfo {
            container: MediaContainer::Dash,
            video_streams: vec![],
            audio_streams: vec![
                AudioStream {
                    qn: AUDIO_192K,
                    codec: Some("mp4a".into()),
                    bandwidth: None,
                    base_url: "https://x/192k".into(),
                    backup_urls: vec![],
                    mime_type: None,
                },
                AudioStream {
                    qn: AUDIO_HIRES,
                    codec: Some("flac".into()),
                    bandwidth: None,
                    base_url: "https://x/hires".into(),
                    backup_urls: vec![],
                    mime_type: None,
                },
            ],
            accepted_qns: vec![],
            accepted_qn_labels: vec![],
            timelength_ms: None,
            premium_required: false,
        };
        let picked = info.pick_audio(AUDIO_AUTO).expect("picked");
        assert_eq!(picked.qn, AUDIO_HIRES);
    }

    #[test]
    fn labels_have_i18n_keys() {
        assert!(qn_label(QN_4K).starts_with("platforms.bilibili."));
        assert!(codec_label(CODEC_AV1).starts_with("platforms.bilibili."));
        assert!(audio_qn_label(AUDIO_HIRES).starts_with("platforms.bilibili."));
    }
}
