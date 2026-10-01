use serde::Serialize;
use serde_json::Value;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StreamLink {
    pub format_id: String,
    pub kind: &'static str,
    pub ext: String,
    pub resolution: Option<String>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub filesize: Option<u64>,
    pub filesize_is_estimate: bool,
    pub tbr: Option<f64>,
    pub protocol: String,
    pub url: String,
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
    if protocol == "mhtml" {
        return None;
    }
    let vcodec = codec(f, "vcodec");
    let acodec = codec(f, "acodec");
    let height = f.get("height").and_then(|v| v.as_u64()).map(|h| h as u32);
    let width = f.get("width").and_then(|v| v.as_u64()).map(|w| w as u32);
    let declared_video = f.get("vcodec").is_some();
    let declared_audio = f.get("acodec").is_some();
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

pub fn parse_stream_links(json: &Value) -> StreamLinks {
    let mut streams: Vec<StreamLink> = match json.get("formats").and_then(|v| v.as_array()) {
        Some(formats) => formats.iter().filter_map(stream_of).collect(),
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
