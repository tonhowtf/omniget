//! download menu. Flow-Desktop `extractors/player.rs` (GPL-3.0) is the

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::{ByteRange, Format, FormatKind, Storyboard, Thumb};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Playability {
    Ok,
    LoginRequired(String),
    AgeRestricted(String),
    Unplayable(String),
    LiveNotStarted(String),
    Error(String),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RawCaption {
    pub lang: String,
    pub name: String,
    pub auto: bool,
    pub base_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tracking {
    pub playback_url: Option<String>,
    pub watchtime_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerParsed {
    pub playability: Playability,
    pub formats: Vec<Format>,
    pub duration_s: u32,
    pub loudness_db: Option<f32>,
    pub expires_in_s: Option<u64>,
    pub is_live: bool,
    pub hls_url: Option<String>,
    pub captions: Vec<RawCaption>,
    pub storyboard_spec: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub channel_id: Option<String>,
    pub thumbs: Vec<Thumb>,
    pub tracking: Option<Tracking>,
}

fn text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    if let Some(runs) = v.get("runs").and_then(Value::as_array) {
        let s: String = runs
            .iter()
            .filter_map(|r| r.get("text").and_then(Value::as_str))
            .collect();
        return (!s.is_empty()).then_some(s);
    }
    v.as_str().map(str::to_string)
}

fn num_u64(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .or_else(|| v.as_f64().map(|f| f as u64))
}

fn range(v: &Value) -> Option<ByteRange> {
    Some(ByteRange {
        start: num_u64(v.get("start")?)?,
        end: num_u64(v.get("end")?)?,
    })
}

fn playability(json: &Value) -> Playability {
    let ps = &json["playabilityStatus"];
    let status = ps["status"].as_str().unwrap_or("");
    let reason = ps
        .get("reason")
        .and_then(text)
        .or_else(|| text(&ps["errorScreen"]["playerErrorMessageRenderer"]["reason"]))
        .or_else(|| {
            ps["messages"]
                .as_array()
                .and_then(|m| m.first())
                .and_then(text)
        })
        .unwrap_or_default();
    let lower = reason.to_lowercase();
    match status {
        "OK" => Playability::Ok,
        "" if json.get("streamingData").is_some() => Playability::Ok,
        "AGE_CHECK_REQUIRED" | "AGE_VERIFICATION_REQUIRED" => Playability::AgeRestricted(reason),
        "LOGIN_REQUIRED" if lower.contains("age") || lower.contains("inappropriate") => {
            Playability::AgeRestricted(reason)
        }
        "LOGIN_REQUIRED" => Playability::LoginRequired(reason),
        "LIVE_STREAM_OFFLINE" => Playability::LiveNotStarted(reason),
        "UNPLAYABLE" if ps.get("liveStreamability").is_some() => {
            Playability::LiveNotStarted(reason)
        }
        "UNPLAYABLE" | "CONTENT_CHECK_REQUIRED" => Playability::Unplayable(reason),
        "" => Playability::Error("empty player response".into()),
        _ => Playability::Error(if reason.is_empty() {
            status.to_string()
        } else {
            reason
        }),
    }
}

fn query_has(url: &str, key: &str) -> bool {
    let Some((_, q)) = url.split_once('?') else {
        return false;
    };
    q.split('&').any(|pair| pair.split('=').next() == Some(key))
}

pub fn url_is_clean(url: &str) -> bool {
    url.starts_with("https://") && !query_has(url, "n") && !query_has(url, "s")
}

fn xtag(url: &str, key: &str) -> Option<String> {
    let (_, q) = url.split_once('?')?;
    let raw = q.split('&').find_map(|p| p.strip_prefix("xtags="))?;
    let decoded = urlencoding::decode(raw).ok()?.into_owned();
    decoded.split(':').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

fn parse_format(f: &Value, muxed: bool) -> Option<Format> {
    let itag = f["itag"].as_u64()? as u32;
    let url = f["url"].as_str()?.to_string();
    if !url_is_clean(&url) {
        return None;
    }
    let mime_full = f["mimeType"].as_str().unwrap_or("");
    let (mime, codecs) = match mime_full.split_once(';') {
        Some((m, rest)) => (
            m.trim().to_string(),
            rest.trim()
                .trim_start_matches("codecs=")
                .trim_matches('"')
                .to_string(),
        ),
        None => (mime_full.trim().to_string(), String::new()),
    };
    let kind = if muxed {
        FormatKind::Muxed
    } else if mime.starts_with("audio/") {
        FormatKind::Audio
    } else {
        FormatKind::Video
    };
    let track = &f["audioTrack"];
    let has_track = track.is_object();
    let acont = xtag(&url, "acont");
    let audio_lang = track["id"]
        .as_str()
        .and_then(|id| id.split('.').next())
        .map(str::to_string)
        .or_else(|| xtag(&url, "lang"));
    let auto_dubbed =
        track["isAutoDubbed"].as_bool().unwrap_or(false) || acont.as_deref() == Some("dubbed-auto");
    let audio_default = if has_track {
        track["audioIsDefault"].as_bool().unwrap_or(false) || acont.as_deref() == Some("original")
    } else {
        !auto_dubbed
    };
    let quality_label = f["qualityLabel"].as_str().map(str::to_string);
    let transfer = f["colorInfo"]["transferCharacteristics"]
        .as_str()
        .unwrap_or("");
    let hdr = transfer.contains("SMPTEST2084")
        || transfer.contains("ARIB_STD_B67")
        || quality_label.as_deref().is_some_and(|q| q.contains("HDR"));
    let drc = f["isDrc"].as_bool().unwrap_or(false) || xtag(&url, "drc").as_deref() == Some("1");
    Some(Format {
        itag,
        kind,
        mime,
        codecs,
        width: f["width"].as_u64().map(|v| v as u32),
        height: f["height"].as_u64().map(|v| v as u32),
        fps: f["fps"].as_u64().map(|v| v as u32),
        bitrate: num_u64(&f["bitrate"]).unwrap_or(0),
        content_length: num_u64(&f["contentLength"]),
        quality_label,
        audio_lang,
        audio_name: track["displayName"].as_str().map(str::to_string),
        audio_default,
        auto_dubbed,
        hdr,
        drc,
        init_range: range(&f["initRange"]),
        index_range: range(&f["indexRange"]),
        url,
    })
}

fn thumbs(v: &Value) -> Vec<Thumb> {
    v["thumbnails"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let url = t["url"].as_str()?;
                    let url = if let Some(rest) = url.strip_prefix("//") {
                        format!("https://{rest}")
                    } else {
                        url.replacen("http://", "https://", 1)
                    };
                    Some(Thumb {
                        url,
                        width: t["width"].as_u64().unwrap_or(0) as u32,
                        height: t["height"].as_u64().unwrap_or(0) as u32,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_player(json: &Value) -> PlayerParsed {
    let sd = &json["streamingData"];
    let mut formats = Vec::new();
    for (key, muxed) in [("adaptiveFormats", false), ("formats", true)] {
        if let Some(list) = sd[key].as_array() {
            formats.extend(list.iter().filter_map(|f| parse_format(f, muxed)));
        }
    }
    let vd = &json["videoDetails"];
    let captions = json["captions"]["playerCaptionsTracklistRenderer"]["captionTracks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let base_url = c["baseUrl"].as_str()?.to_string();
                    let lang = c["languageCode"].as_str().unwrap_or("").to_string();
                    Some(RawCaption {
                        name: text(&c["name"]).unwrap_or_else(|| lang.clone()),
                        auto: c["kind"].as_str() == Some("asr"),
                        lang,
                        base_url,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let pt = &json["playbackTracking"];
    let tracking = Tracking {
        playback_url: pt["videostatsPlaybackUrl"]["baseUrl"]
            .as_str()
            .map(str::to_string),
        watchtime_url: pt["videostatsWatchtimeUrl"]["baseUrl"]
            .as_str()
            .map(str::to_string),
    };
    let audio_cfg = &json["playerConfig"]["audioConfig"];
    PlayerParsed {
        playability: playability(json),
        formats,
        duration_s: num_u64(&vd["lengthSeconds"]).unwrap_or(0) as u32,
        loudness_db: audio_cfg["loudnessDb"]
            .as_f64()
            .or_else(|| audio_cfg["perceptualLoudnessDb"].as_f64())
            .map(|v| v as f32),
        expires_in_s: num_u64(&sd["expiresInSeconds"]),
        is_live: vd["isLive"].as_bool().unwrap_or(false),
        hls_url: sd["hlsManifestUrl"].as_str().map(str::to_string),
        captions,
        storyboard_spec: json["storyboards"]["playerStoryboardSpecRenderer"]["spec"]
            .as_str()
            .or_else(|| json["storyboards"]["playerLiveStoryboardSpecRenderer"]["spec"].as_str())
            .map(str::to_string),
        title: vd["title"].as_str().map(str::to_string),
        author: vd["author"].as_str().map(str::to_string),
        channel_id: vd["channelId"].as_str().map(str::to_string),
        thumbs: thumbs(&vd["thumbnail"]),
        tracking: (tracking.playback_url.is_some() || tracking.watchtime_url.is_some())
            .then_some(tracking),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PickPrefs {
    pub max_height: Option<u32>,
    pub codec: String,
    pub webkit: bool,
    pub audio_quality: String,
    pub lang: Option<String>,
}

impl Default for PickPrefs {
    fn default() -> Self {
        Self {
            max_height: None,
            codec: "auto".into(),
            webkit: false,
            audio_quality: "high".into(),
            lang: None,
        }
    }
}

fn codec_family(f: &Format) -> &'static str {
    let c = f.codecs.as_str();
    if c.starts_with("avc1") || c.starts_with("avc3") {
        "avc1"
    } else if c.starts_with("vp9") || c.starts_with("vp09") {
        "vp9"
    } else if c.starts_with("av01") {
        "av01"
    } else if c.starts_with("mp4a") {
        "mp4a"
    } else if c.starts_with("opus") {
        "opus"
    } else {
        "other"
    }
}

fn codec_order(p: &PickPrefs) -> Vec<&'static str> {
    if p.webkit {
        return vec!["avc1", "vp9", "av01", "other"];
    }
    match p.codec.as_str() {
        "avc1" => vec!["avc1", "vp9", "av01", "other"],
        "av01" => vec!["av01", "vp9", "avc1", "other"],
        _ => vec!["vp9", "avc1", "av01", "other"],
    }
}

fn video_candidates<'a>(formats: &'a [Format], p: &PickPrefs) -> Vec<&'a Format> {
    let mut v: Vec<&Format> = formats
        .iter()
        .filter(|f| f.kind == FormatKind::Video)
        .collect();
    let want = if p.webkit {
        Some("avc1")
    } else {
        match p.codec.as_str() {
            "avc1" | "vp9" | "av01" => Some(p.codec.as_str()),
            _ => None,
        }
    };
    if let Some(want) = want {
        if v.iter().any(|f| codec_family(f) == want) {
            v.retain(|f| codec_family(f) == want);
        }
    }
    v
}

fn video_rank(f: &Format, order: &[&str]) -> (usize, bool, u32, u64) {
    let pos = order
        .iter()
        .position(|c| *c == codec_family(f))
        .unwrap_or(order.len());
    (usize::MAX - pos, !f.hdr, f.fps.unwrap_or(0), f.bitrate)
}

pub fn pick_video<'a>(formats: &'a [Format], p: &PickPrefs) -> Option<&'a Format> {
    let order = codec_order(p);
    let cap = p.max_height.unwrap_or(u32::MAX);
    let cands = video_candidates(formats, p);
    let under: Vec<&Format> = cands
        .iter()
        .copied()
        .filter(|f| f.height.unwrap_or(0) <= cap)
        .collect();
    let pool = if under.is_empty() {
        let min = cands.iter().filter_map(|f| f.height).min();
        cands
            .into_iter()
            .filter(|f| f.height == min)
            .collect::<Vec<_>>()
    } else {
        under
    };
    let best = pool
        .into_iter()
        .max_by_key(|f| (f.height.unwrap_or(0), video_rank(f, &order)));
    best.or_else(|| {
        formats
            .iter()
            .filter(|f| f.kind == FormatKind::Muxed)
            .filter(|f| f.height.unwrap_or(0) <= cap)
            .max_by_key(|f| (f.height.unwrap_or(0), f.bitrate))
    })
}

fn lang_matches(track: Option<&str>, want: &str) -> bool {
    let Some(t) = track else {
        return false;
    };
    let t = t.to_ascii_lowercase();
    let w = want.to_ascii_lowercase();
    t == w || t.split(['-', '_']).next() == w.split(['-', '_']).next()
}

pub fn pick_audio<'a>(formats: &'a [Format], p: &PickPrefs) -> Option<&'a Format> {
    let all: Vec<&Format> = formats
        .iter()
        .filter(|f| f.kind == FormatKind::Audio)
        .collect();
    if all.is_empty() {
        return None;
    }
    let tier = |f: &Format| -> u8 {
        let wanted = p
            .lang
            .as_deref()
            .is_some_and(|l| lang_matches(f.audio_lang.as_deref(), l));
        match (f.auto_dubbed, wanted, f.audio_default) {
            (false, true, _) => 0,
            (false, false, true) => 1,
            (false, false, false) => 2,
            (true, _, _) => 3,
        }
    };
    let best = all.iter().map(|f| tier(f)).min().unwrap_or(3);
    let mut pool: Vec<&Format> = all.into_iter().filter(|f| tier(f) == best).collect();
    if pool.iter().any(|f| !f.drc) {
        pool.retain(|f| !f.drc);
    }
    let family = if p.webkit { "mp4a" } else { "opus" };
    if pool.iter().any(|f| codec_family(f) == family) {
        pool.retain(|f| codec_family(f) == family);
    }
    match p.audio_quality.as_str() {
        "low" => pool.into_iter().min_by_key(|f| f.bitrate),
        "medium" => pool.into_iter().min_by_key(|f| f.bitrate.abs_diff(128_000)),
        _ => pool.into_iter().max_by_key(|f| f.bitrate),
    }
}

pub fn parse_storyboard(spec: &str, duration_s: u32) -> Option<(String, Storyboard)> {
    let mut parts = spec.split('|');
    let base = parts.next()?;
    let levels: Vec<&str> = parts.collect();
    let (idx, level) = levels
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let f: Vec<&str> = l.split('#').collect();
            (f.len() >= 8).then_some((i, f))
        })
        .max_by_key(|(_, f)| f[0].parse::<u32>().unwrap_or(0))?;
    let n = |i: usize| level[i].parse::<u32>().ok();
    let (width, height, count, cols, rows) = (n(0)?, n(1)?, n(2)?, n(3)?, n(4)?);
    if count == 0 || cols == 0 || rows == 0 {
        return None;
    }
    let interval_ms = match n(5) {
        Some(v) if v > 0 => v,
        _ => duration_s.saturating_mul(1000) / count.max(1),
    };
    let name = level[6];
    let sigh = level[7];
    let mut url = base.replace("$L", &idx.to_string()).replace("$N", name);
    url.push(if url.contains('?') { '&' } else { '?' });
    url.push_str("sigh=");
    url.push_str(sigh);
    let sb = Storyboard {
        url: url.clone(),
        width,
        height,
        count,
        cols,
        rows,
        interval_ms,
    };
    Some((url, sb))
}

pub fn caption_vtt_url(base_url: &str, tlang: Option<&str>) -> String {
    let base = if base_url.starts_with('/') {
        format!("https://www.youtube.com{base_url}")
    } else {
        base_url.to_string()
    };
    let (path, query) = base.split_once('?').unwrap_or((base.as_str(), ""));
    let mut pairs: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .filter(|p| {
            let k = p.split('=').next().unwrap_or("");
            k != "fmt" && k != "tlang"
        })
        .collect();
    pairs.push("fmt=vtt");
    let tl;
    if let Some(t) = tlang.filter(|t| !t.is_empty()) {
        tl = format!("tlang={}", urlencoding::encode(t));
        pairs.push(&tl);
    }
    format!("{path}?{}", pairs.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const U: &str = "https://rr1---sn-x.googlevideo.com/videoplayback?expire=1&itag=";

    fn fmt(itag: u32, mime: &str, extra: Value) -> Value {
        let mut v = json!({
            "itag": itag,
            "url": format!("{U}{itag}"),
            "mimeType": mime,
            "bitrate": 1000 * itag,
            "initRange": {"start": "0", "end": "740"},
            "indexRange": {"start": "741", "end": "1500"},
            "contentLength": "123456",
        });
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    }

    fn sample() -> Value {
        json!({
            "playabilityStatus": {"status": "OK"},
            "streamingData": {
                "expiresInSeconds": "21540",
                "hlsManifestUrl": "https://manifest.googlevideo.com/api/manifest/hls_variant/x",
                "formats": [fmt(18, "video/mp4; codecs=\"avc1.42001E, mp4a.40.2\"", json!({"width": 640, "height": 360, "qualityLabel": "360p"}))],
                "adaptiveFormats": [
                    fmt(137, "video/mp4; codecs=\"avc1.640028\"", json!({"width": 1920, "height": 1080, "fps": 30, "qualityLabel": "1080p"})),
                    fmt(248, "video/webm; codecs=\"vp9\"", json!({"width": 1920, "height": 1080, "fps": 30, "qualityLabel": "1080p"})),
                    fmt(271, "video/webm; codecs=\"vp9\"", json!({"width": 2560, "height": 1440, "fps": 30, "qualityLabel": "1440p"})),
                    fmt(136, "video/mp4; codecs=\"avc1.4d401f\"", json!({"width": 1280, "height": 720, "fps": 30})),
                    fmt(399, "video/mp4; codecs=\"av01.0.08M.08\"", json!({"width": 1920, "height": 1080, "fps": 30})),
                    fmt(337, "video/webm; codecs=\"vp9.2\"", json!({"width": 3840, "height": 2160, "fps": 60, "qualityLabel": "2160p60 HDR", "colorInfo": {"transferCharacteristics": "COLOR_TRANSFER_CHARACTERISTICS_SMPTEST2084"}})),
                    fmt(140, "audio/mp4; codecs=\"mp4a.40.2\"", json!({"bitrate": 130000, "audioTrack": {"displayName": "English original", "id": "en.4", "audioIsDefault": true}})),
                    fmt(251, "audio/webm; codecs=\"opus\"", json!({"bitrate": 140000, "audioTrack": {"displayName": "English original", "id": "en.4", "audioIsDefault": true}})),
                    fmt(250, "audio/webm; codecs=\"opus\"", json!({"bitrate": 70000, "audioTrack": {"displayName": "English original", "id": "en.4", "audioIsDefault": true}})),
                    fmt(600, "audio/webm; codecs=\"opus\"", json!({"bitrate": 160000, "audioTrack": {"displayName": "Português (dublado automaticamente)", "id": "pt-BR.10", "audioIsDefault": false, "isAutoDubbed": true}})),
                    fmt(601, "audio/mp4; codecs=\"mp4a.40.2\"", json!({"bitrate": 129000, "audioTrack": {"displayName": "Español", "id": "es.3", "audioIsDefault": false}})),
                    {"itag": 999, "signatureCipher": "s=abc&url=https%3A%2F%2Fx", "mimeType": "audio/mp4; codecs=\"mp4a.40.2\""},
                    {"itag": 998, "url": "https://x.googlevideo.com/videoplayback?n=abc&itag=998", "mimeType": "audio/mp4; codecs=\"mp4a.40.2\""},
                ]
            },
            "videoDetails": {
                "videoId": "dQw4w9WgXcQ", "title": "T", "author": "A", "channelId": "UCx",
                "lengthSeconds": "212", "isLive": false,
                "thumbnail": {"thumbnails": [{"url": "https://i.ytimg.com/vi/x/hq.jpg", "width": 480, "height": 360}]}
            },
            "playerConfig": {"audioConfig": {"loudnessDb": -3.5}},
            "captions": {"playerCaptionsTracklistRenderer": {"captionTracks": [
                {"baseUrl": "https://www.youtube.com/api/timedtext?v=x&lang=en&fmt=srv3", "name": {"simpleText": "English"}, "languageCode": "en"},
                {"baseUrl": "https://www.youtube.com/api/timedtext?v=x&lang=en&kind=asr", "name": {"runs": [{"text": "English (auto-generated)"}]}, "languageCode": "en", "kind": "asr"}
            ]}},
            "storyboards": {"playerStoryboardSpecRenderer": {"spec": "https://i.ytimg.com/sb/x/storyboard3_L$L/$N.jpg?sqp=abc|48#27#100#10#10#0#default#rs$A|80#45#72#10#10#3000#M$M#rs$B|160#90#72#5#5#3000#M$M#rs$C"}},
            "playbackTracking": {
                "videostatsPlaybackUrl": {"baseUrl": "https://s.youtube.com/api/stats/playback?x=1"},
                "videostatsWatchtimeUrl": {"baseUrl": "https://s.youtube.com/api/stats/watchtime?x=1"}
            }
        })
    }

    #[test]
    fn parses_full_response() {
        let p = parse_player(&sample());
        assert_eq!(p.playability, Playability::Ok);
        assert_eq!(p.duration_s, 212);
        assert_eq!(p.expires_in_s, Some(21540));
        assert_eq!(p.loudness_db, Some(-3.5));
        assert!(p.hls_url.is_some());
        assert_eq!(p.title.as_deref(), Some("T"));
        assert_eq!(p.channel_id.as_deref(), Some("UCx"));
        assert_eq!(p.thumbs.len(), 1);
        assert_eq!(p.formats.len(), 12);
        assert!(p.formats.iter().all(|f| f.itag != 999 && f.itag != 998));
        let muxed = p.formats.iter().find(|f| f.itag == 18).unwrap();
        assert_eq!(muxed.kind, FormatKind::Muxed);
        let v = p.formats.iter().find(|f| f.itag == 137).unwrap();
        assert_eq!(v.kind, FormatKind::Video);
        assert_eq!(v.mime, "video/mp4");
        assert_eq!(v.codecs, "avc1.640028");
        assert_eq!(v.init_range, Some(ByteRange { start: 0, end: 740 }));
        assert_eq!(
            v.index_range,
            Some(ByteRange {
                start: 741,
                end: 1500
            })
        );
        assert_eq!(v.content_length, Some(123456));
        assert!(p.formats.iter().find(|f| f.itag == 337).unwrap().hdr);
        let dub = p.formats.iter().find(|f| f.itag == 600).unwrap();
        assert!(dub.auto_dubbed);
        assert_eq!(dub.audio_lang.as_deref(), Some("pt-BR"));
        assert_eq!(p.captions.len(), 2);
        assert!(p.captions[1].auto);
        assert_eq!(p.captions[1].name, "English (auto-generated)");
        assert!(p.storyboard_spec.is_some());
        let t = p.tracking.unwrap();
        assert!(t.watchtime_url.unwrap().contains("watchtime"));
    }

    #[test]
    fn playability_states() {
        let st = |ps: Value| parse_player(&json!({ "playabilityStatus": ps })).playability;
        assert!(matches!(
            st(
                json!({"status": "LOGIN_REQUIRED", "reason": "Sign in to confirm you're not a bot"})
            ),
            Playability::LoginRequired(_)
        ));
        assert!(matches!(
            st(json!({"status": "LOGIN_REQUIRED", "reason": "Sign in to confirm your age"})),
            Playability::AgeRestricted(_)
        ));
        assert!(matches!(
            st(json!({"status": "AGE_CHECK_REQUIRED"})),
            Playability::AgeRestricted(_)
        ));
        assert!(matches!(
            st(
                json!({"status": "UNPLAYABLE", "reason": "Premieres in 3 hours", "liveStreamability": {}})
            ),
            Playability::LiveNotStarted(_)
        ));
        match st(
            json!({"status": "UNPLAYABLE", "errorScreen": {"playerErrorMessageRenderer": {"reason": {"runs": [{"text": "Video unavailable"}]}}}}),
        ) {
            Playability::Unplayable(r) => assert_eq!(r, "Video unavailable"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            st(json!({"status": "ERROR", "reason": "x"})),
            Playability::Error(_)
        ));
        assert!(matches!(
            parse_player(&json!({})).playability,
            Playability::Error(_)
        ));
    }

    #[test]
    fn clean_url_rules() {
        assert!(url_is_clean(
            "https://a.googlevideo.com/videoplayback?itag=1&expire=2"
        ));
        assert!(!url_is_clean(
            "https://a.googlevideo.com/videoplayback?itag=1&n=xyz"
        ));
        assert!(!url_is_clean(
            "https://a.googlevideo.com/videoplayback?s=abc&itag=1"
        ));
        assert!(url_is_clean(
            "https://a.googlevideo.com/videoplayback?sn=1&nx=2"
        ));
        assert!(!url_is_clean("http://a.googlevideo.com/videoplayback"));
    }

    #[test]
    fn xtags_mark_dub_and_drc() {
        let f = parse_format(
            &json!({"itag": 251, "mimeType": "audio/webm; codecs=\"opus\"",
                "url": "https://x/videoplayback?xtags=acont%3Ddubbed-auto%3Alang%3Dde&itag=251"}),
            false,
        )
        .unwrap();
        assert!(f.auto_dubbed);
        assert!(!f.audio_default);
        assert_eq!(f.audio_lang.as_deref(), Some("de"));
        let d = parse_format(
            &json!({"itag": 251, "mimeType": "audio/webm; codecs=\"opus\"",
                "url": "https://x/videoplayback?xtags=drc%3D1&itag=251"}),
            false,
        )
        .unwrap();
        assert!(d.drc);
        assert!(d.audio_default);
    }

    fn formats() -> Vec<Format> {
        parse_player(&sample()).formats
    }

    #[test]
    fn pick_video_rules() {
        let f = formats();
        let auto = PickPrefs::default();
        assert_eq!(pick_video(&f, &auto).unwrap().itag, 337);
        let capped = PickPrefs {
            max_height: Some(1080),
            ..PickPrefs::default()
        };
        assert_eq!(pick_video(&f, &capped).unwrap().itag, 248);
        let webkit = PickPrefs {
            webkit: true,
            ..PickPrefs::default()
        };
        assert_eq!(pick_video(&f, &webkit).unwrap().itag, 137);
        let webkit720 = PickPrefs {
            webkit: true,
            max_height: Some(720),
            ..PickPrefs::default()
        };
        assert_eq!(pick_video(&f, &webkit720).unwrap().itag, 136);
        let av1 = PickPrefs {
            codec: "av01".into(),
            ..PickPrefs::default()
        };
        assert_eq!(pick_video(&f, &av1).unwrap().itag, 399);
        let tiny = PickPrefs {
            max_height: Some(144),
            webkit: true,
            ..PickPrefs::default()
        };
        assert_eq!(pick_video(&f, &tiny).unwrap().itag, 136);
        let only_muxed: Vec<Format> = f
            .iter()
            .filter(|x| x.kind != FormatKind::Video)
            .cloned()
            .collect();
        assert_eq!(pick_video(&only_muxed, &auto).unwrap().itag, 18);
    }

    #[test]
    fn pick_audio_rules() {
        let f = formats();
        let auto = PickPrefs::default();
        assert_eq!(pick_audio(&f, &auto).unwrap().itag, 251);
        let webkit = PickPrefs {
            webkit: true,
            ..PickPrefs::default()
        };
        assert_eq!(pick_audio(&f, &webkit).unwrap().itag, 140);
        let low = PickPrefs {
            audio_quality: "low".into(),
            ..PickPrefs::default()
        };
        assert_eq!(pick_audio(&f, &low).unwrap().itag, 250);
        let pt = PickPrefs {
            lang: Some("pt".into()),
            ..PickPrefs::default()
        };
        assert_eq!(pick_audio(&f, &pt).unwrap().itag, 251);
        let es = PickPrefs {
            lang: Some("es-419".into()),
            ..PickPrefs::default()
        };
        assert_eq!(pick_audio(&f, &es).unwrap().itag, 601);
        let dub_only: Vec<Format> = f.iter().filter(|x| x.itag == 600).cloned().collect();
        assert_eq!(pick_audio(&dub_only, &auto).unwrap().itag, 600);
        assert!(pick_audio(&[], &auto).is_none());
    }

    #[test]
    fn storyboard_largest_level() {
        let spec = sample()["storyboards"]["playerStoryboardSpecRenderer"]["spec"]
            .as_str()
            .unwrap()
            .to_string();
        let (url, sb) = parse_storyboard(&spec, 212).unwrap();
        assert_eq!(
            url,
            "https://i.ytimg.com/sb/x/storyboard3_L2/M$M.jpg?sqp=abc&sigh=rs$C"
        );
        assert_eq!(
            (sb.width, sb.height, sb.count, sb.cols, sb.rows),
            (160, 90, 72, 5, 5)
        );
        assert_eq!(sb.interval_ms, 3000);
        let (_, zero) =
            parse_storyboard("https://x/$L/$N.jpg|48#27#100#10#10#0#default#rs$A", 200).unwrap();
        assert_eq!(zero.interval_ms, 2000);
        assert!(parse_storyboard("https://x", 10).is_none());
    }

    #[test]
    fn vtt_url() {
        assert_eq!(
            caption_vtt_url(
                "https://www.youtube.com/api/timedtext?v=x&lang=en&fmt=srv3",
                None
            ),
            "https://www.youtube.com/api/timedtext?v=x&lang=en&fmt=vtt"
        );
        assert_eq!(
            caption_vtt_url("/api/timedtext?v=x", Some("pt-BR")),
            "https://www.youtube.com/api/timedtext?v=x&fmt=vtt&tlang=pt-BR"
        );
    }
}
