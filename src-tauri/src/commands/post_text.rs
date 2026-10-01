//! Downloads › Tools › Captions & info: saves the text that comes with a post
//! or video (caption or description, author, date, link, tags) next to the
//! downloads, as a `.txt` a person can read and an optional `.json` with every
//! field the site gave.
//!
//! The idea comes from Instaloader, which writes the caption as `<post>.txt`
//! and the metadata as `<post>.json.xz` beside each download. Reimplemented on
//! the yt-dlp JSON the app already fetches; no code was copied.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;

#[derive(Debug, Clone, Serialize)]
pub struct PostTextResult {
    pub url: String,
    pub ok: bool,
    pub title: Option<String>,
    pub txt_path: Option<String>,
    pub json_path: Option<String>,
    pub error: Option<String>,
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}

/// `20240131` → `2024-01-31`.
fn upload_date(v: &Value) -> Option<String> {
    let d = s(v, "upload_date")?;
    if d.len() == 8 && d.chars().all(|c| c.is_ascii_digit()) {
        Some(format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..]))
    } else {
        Some(d)
    }
}

/// The readable sidecar: a short header, then the caption as the site wrote it.
pub fn render_txt(v: &Value) -> String {
    let mut out = String::new();
    let title = s(v, "title").or_else(|| s(v, "fulltitle"));
    if let Some(t) = &title {
        out.push_str(t);
        out.push('\n');
    }
    let author = s(v, "uploader")
        .or_else(|| s(v, "channel"))
        .or_else(|| s(v, "creator"));
    let mut meta: Vec<String> = Vec::new();
    if let Some(a) = author {
        match s(v, "uploader_id").or_else(|| s(v, "channel_id")) {
            Some(id) if !a.contains(&id) => meta.push(format!("{a} ({id})")),
            _ => meta.push(a),
        }
    }
    if let Some(d) = upload_date(v) {
        meta.push(d);
    }
    if let Some(site) = s(v, "extractor_key").or_else(|| s(v, "extractor")) {
        meta.push(site);
    }
    if !meta.is_empty() {
        out.push_str(&meta.join(" · "));
        out.push('\n');
    }
    if let Some(u) = s(v, "webpage_url").or_else(|| s(v, "original_url")) {
        out.push_str(&u);
        out.push('\n');
    }
    if let Some(desc) = s(v, "description") {
        // A caption that only repeats the title adds nothing.
        if title.as_deref() != Some(desc.as_str()) {
            out.push('\n');
            out.push_str(&desc);
            out.push('\n');
        }
    }
    let tags: Vec<String> = v
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|t| format!("#{}", t.trim_start_matches('#')))
                .collect()
        })
        .unwrap_or_default();
    if !tags.is_empty() {
        out.push('\n');
        out.push_str(&tags.join(" "));
        out.push('\n');
    }
    out
}

/// The JSON sidecar: every field except the stream lists, whose links expire
/// and would only make the file huge.
pub fn slim_json(v: &Value) -> Value {
    let mut v = v.clone();
    if let Some(o) = v.as_object_mut() {
        for k in [
            "formats",
            "requested_formats",
            "requested_downloads",
            "thumbnails",
            "automatic_captions",
            "http_headers",
            "url",
            "manifest_url",
            "fragments",
        ] {
            o.remove(k);
        }
    }
    v
}

/// `<title> [<id>]`, safe on every system and short enough for Windows paths.
pub fn base_name(v: &Value) -> String {
    let title = s(v, "title").unwrap_or_else(|| "post".to_string());
    let short: String = title.chars().take(120).collect();
    let name = match s(v, "id") {
        Some(id) => format!("{short} [{id}]"),
        None => short,
    };
    let clean = omniget_core::core::filename::sanitize_path_component(&name);
    if clean.is_empty() {
        "post".to_string()
    } else {
        clean
    }
}

fn write_sidecars(
    v: &Value,
    dir: &Path,
    with_json: bool,
) -> std::io::Result<(PathBuf, Option<PathBuf>)> {
    std::fs::create_dir_all(dir)?;
    let base = base_name(v);
    let txt = dir.join(format!("{base}.txt"));
    std::fs::write(&txt, render_txt(v))?;
    let json = if with_json {
        let p = dir.join(format!("{base}.info.json"));
        let body = serde_json::to_string_pretty(&slim_json(v)).unwrap_or_default();
        std::fs::write(&p, body)?;
        Some(p)
    } else {
        None
    };
    Ok((txt, json))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn save_post_text(
    urls: Vec<String>,
    output_dir: String,
    with_json: bool,
) -> Result<Vec<PostTextResult>, String> {
    if output_dir.trim().is_empty() {
        return Err("Choose a download folder first".to_string());
    }
    let ytdlp_path = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {}", e))?;
    let dir = PathBuf::from(&output_dir);
    let mut out = Vec::new();
    for url in urls
        .iter()
        .map(|u| u.trim())
        .filter(|u| !u.is_empty())
        .take(50)
    {
        let mut r = PostTextResult {
            url: url.to_string(),
            ok: false,
            title: None,
            txt_path: None,
            json_path: None,
            error: None,
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            r.error = Some("Not a web address".to_string());
            out.push(r);
            continue;
        }
        if let Err(e) = omniget_core::core::platform_optout::ensure_allowed(url) {
            r.error = Some(e);
            out.push(r);
            continue;
        }
        // One item, not a whole playlist: a playlist link saves its own text.
        let flags = vec!["--flat-playlist".to_string()];
        match ytdlp::get_video_info(&ytdlp_path, url, &flags).await {
            Ok(v) => {
                r.title = s(&v, "title");
                match write_sidecars(&v, &dir, with_json) {
                    Ok((txt, json)) => {
                        r.ok = true;
                        r.txt_path = Some(txt.to_string_lossy().into_owned());
                        r.json_path = json.map(|p| p.to_string_lossy().into_owned());
                    }
                    Err(e) => r.error = Some(e.to_string()),
                }
            }
            Err(e) => r.error = Some(format!("Failed to get formats: {e}")),
        }
        out.push(r);
    }
    Ok(out)
}
