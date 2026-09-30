//! Downloads › Tools › Stream links: every stream a page offers, with the
//! exact size and the direct media URL, before anything is downloaded.
//!
//! The idea comes from lux (`lux -i` lists each stream with its size, `lux -j`
//! prints the extracted URLs as JSON) and from `you-get -u`. Reimplemented on
//! top of the yt-dlp JSON the app already fetches; no code was copied.

use serde::Serialize;
use serde_json::Value;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StreamLink {
    pub format_id: String,
    /// "av" (video with audio), "video" (no audio) or "audio".
    pub kind: &'static str,
    pub ext: String,
    pub resolution: Option<String>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub filesize: Option<u64>,
    /// True when the size is yt-dlp's estimate (`filesize_approx`).
    pub filesize_is_estimate: bool,
    pub tbr: Option<f64>,
    /// yt-dlp protocol: `https`, `m3u8_native`, `http_dash_segments`...
    pub protocol: String,
    /// The direct URL, or the playlist/manifest URL for segmented streams.
    pub url: String,
    /// Headers the server expects (User-Agent, Referer, Cookie...).
    pub headers: Vec<(String, String)>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StreamLinks {
    pub title: String,
    pub site: String,
    pub page_url: String,
    pub duration: Option<f64>,
    pub streams: Vec<StreamLink>,
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn codec(v: &Value, key: &str) -> Option<String> {
    str_of(v, key).filter(|c| c != "none")
}

fn headers_of(v: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = v
        .get("http_headers")
        .and_then(|h| h.as_object())
        .map(|h| {
            h.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn stream_of(f: &Value) -> Option<StreamLink> {
    let format_id = str_of(f, "format_id")?;
    let url = str_of(f, "url").or_else(|| str_of(f, "manifest_url"))?;
    let protocol = str_of(f, "protocol").unwrap_or_else(|| "https".to_string());
    // Storyboards (`mhtml`) are thumbnails, not media.
    if protocol == "mhtml" {
        return None;
    }
    let vcodec = codec(f, "vcodec");
    let acodec = codec(f, "acodec");
    let height = f.get("height").and_then(|v| v.as_u64()).map(|h| h as u32);
    let width = f.get("width").and_then(|v| v.as_u64()).map(|w| w as u32);
    let declared_video = f.get("vcodec").is_some();
    let declared_audio = f.get("acodec").is_some();
    // Direct files and some extractors leave the codecs out: a stream with a
    // height is video, and without codec fields it is a whole file (both).
    let has_video = vcodec.is_some() || (!declared_video && (height.is_some() || !declared_audio));
    let has_audio = acodec.is_some() || (!declared_audio && !declared_video);
    let kind = match (has_video, has_audio) {
        (true, true) => "av",
        (true, false) => "video",
        (false, true) => "audio",
        (false, false) => return None,
    };
    let exact = f.get("filesize").and_then(|v| v.as_u64());
    let approx = f.get("filesize_approx").and_then(|v| v.as_u64());
    let resolution = match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some(format!("{w}x{h}")),
        _ => str_of(f, "resolution").filter(|r| r != "audio only"),
    };
    Some(StreamLink {
        format_id,
        kind,
        ext: str_of(f, "ext").unwrap_or_default(),
        resolution,
        height,
        fps: f.get("fps").and_then(|v| v.as_f64()),
        vcodec,
        acodec,
        filesize: exact.or(approx),
        filesize_is_estimate: exact.is_none() && approx.is_some(),
        tbr: f.get("tbr").and_then(|v| v.as_f64()),
        protocol,
        url,
        headers: headers_of(f),
        note: str_of(f, "format_note"),
    })
}

/// Turns yt-dlp's `-J` output into the list the panel shows: best first
/// (video with audio, then video, then audio; taller and larger first).
pub fn parse_stream_links(json: &Value) -> StreamLinks {
    let mut streams: Vec<StreamLink> = match json.get("formats").and_then(|v| v.as_array()) {
        Some(formats) => formats.iter().filter_map(stream_of).collect(),
        // A direct file has no `formats`: the page itself is the only stream.
        None => {
            let mut whole = json.clone();
            if let Some(o) = whole.as_object_mut() {
                o.entry("format_id").or_insert_with(|| Value::from("file"));
            }
            stream_of(&whole).into_iter().collect()
        }
    };
    let rank = |k: &str| match k {
        "av" => 0,
        "video" => 1,
        _ => 2,
    };
    streams.sort_by(|a, b| {
        rank(a.kind)
            .cmp(&rank(b.kind))
            .then(b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0)))
            .then(b.filesize.unwrap_or(0).cmp(&a.filesize.unwrap_or(0)))
            .then(
                b.tbr
                    .unwrap_or(0.0)
                    .partial_cmp(&a.tbr.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    StreamLinks {
        title: str_of(json, "title").unwrap_or_default(),
        site: str_of(json, "extractor_key")
            .or_else(|| str_of(json, "extractor"))
            .unwrap_or_default(),
        page_url: str_of(json, "webpage_url")
            .or_else(|| str_of(json, "original_url"))
            .unwrap_or_default(),
        duration: json.get("duration").and_then(|v| v.as_f64()),
        streams,
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn list_stream_links(url: String) -> Result<StreamLinks, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Paste a web address that starts with http:// or https://".to_string());
    }
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    let ytdlp_path = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {}", e))?;
    let json = ytdlp::get_video_info(&ytdlp_path, &url, &[])
        .await
        .map_err(|e| format!("Failed to get formats: {}", e))?;
    let links = parse_stream_links(&json);
    if links.streams.is_empty() {
        return Err("No stream with a direct link was found on this page".to_string());
    }
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lists_best_first_and_skips_storyboards() {
        let j = json!({
            "title": "WING IT!", "extractor_key": "BiliBili", "webpage_url": "https://b/1", "duration": 213.0,
            "formats": [
                {"format_id": "sb0", "protocol": "mhtml", "url": "https://sb", "vcodec": "none", "acodec": "none"},
                {"format_id": "a1", "url": "https://a", "protocol": "https", "vcodec": "none", "acodec": "mp4a.40.2", "filesize": 3000, "ext": "m4a"},
                {"format_id": "v480", "url": "https://v480", "protocol": "https", "vcodec": "avc1", "acodec": "none", "height": 480, "width": 854, "filesize_approx": 9000, "ext": "mp4"},
                {"format_id": "v1080", "url": "https://v1080", "protocol": "https", "vcodec": "avc1", "acodec": "none", "height": 1080, "width": 1920, "filesize": 19000, "ext": "mp4",
                 "http_headers": {"Referer": "https://b/", "User-Agent": "UA"}},
                {"format_id": "18", "url": "https://av", "protocol": "https", "vcodec": "avc1", "acodec": "mp4a", "height": 360, "ext": "mp4"}
            ]
        });
        let l = parse_stream_links(&j);
        assert_eq!(l.title, "WING IT!");
        assert_eq!(l.site, "BiliBili");
        let ids: Vec<&str> = l.streams.iter().map(|s| s.format_id.as_str()).collect();
        assert_eq!(ids, ["18", "v1080", "v480", "a1"]);
        assert_eq!(l.streams[0].kind, "av");
        assert_eq!(l.streams[1].kind, "video");
        assert_eq!(l.streams[3].kind, "audio");
        assert!(l.streams[2].filesize_is_estimate);
        assert!(!l.streams[1].filesize_is_estimate);
        assert_eq!(l.streams[1].resolution.as_deref(), Some("1920x1080"));
        assert_eq!(
            l.streams[1].headers,
            vec![
                ("Referer".to_string(), "https://b/".to_string()),
                ("User-Agent".to_string(), "UA".to_string())
            ]
        );
    }

    #[test]
    fn a_direct_file_is_its_own_stream() {
        let j = json!({
            "title": "tears_of_steel_720p", "extractor": "generic", "ext": "mov",
            "url": "https://download.blender.org/demo/movies/ToS/tears_of_steel_720p.mov",
            "webpage_url": "https://download.blender.org/demo/movies/ToS/tears_of_steel_720p.mov"
        });
        let l = parse_stream_links(&j);
        assert_eq!(l.streams.len(), 1);
        assert_eq!(l.streams[0].kind, "av");
        assert_eq!(l.streams[0].protocol, "https");
        assert_eq!(l.site, "generic");
    }

    #[test]
    fn hls_keeps_the_playlist_url() {
        let j = json!({"formats": [
            {"format_id": "hls-1080", "protocol": "m3u8_native", "manifest_url": "https://x/master.m3u8",
             "url": "https://x/1080.m3u8", "vcodec": "avc1", "acodec": "mp4a", "height": 1080}
        ]});
        let l = parse_stream_links(&j);
        assert_eq!(l.streams[0].url, "https://x/1080.m3u8");
        assert_eq!(l.streams[0].protocol, "m3u8_native");
    }
}
