//! The idea comes from GDownloader (hstr0100/GDownloader, GPL-3.0), whose

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

const MAX_HTML_BYTES: usize = 5 * 1024 * 1024;
const MAX_FILES: usize = 300;
const SIZE_LOOKUPS: usize = 8;

const KINDS: &[(&str, &[&str])] = &[
    (
        "video",
        &[
            "mp4", "mkv", "webm", "mov", "m4v", "avi", "flv", "wmv", "ts", "mpg", "mpeg", "ogv",
        ],
    ),
    (
        "audio",
        &[
            "mp3", "m4a", "flac", "wav", "ogg", "opus", "aac", "wma", "aiff",
        ],
    ),
    (
        "archive",
        &["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst"],
    ),
    (
        "document",
        &["pdf", "epub", "djvu", "cbz", "cbr", "srt", "vtt"],
    ),
    (
        "image",
        &["jpg", "jpeg", "png", "gif", "webp", "tif", "tiff", "svg"],
    ),
    (
        "disk",
        &[
            "iso", "img", "dmg", "exe", "msi", "deb", "rpm", "apk", "appimage",
        ],
    ),
    ("torrent", &["torrent"]),
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PageFile {
    pub url: String,
    pub name: String,
    pub ext: String,
    pub kind: &'static str,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageFiles {
    pub page_url: String,
    pub title: Option<String>,
    pub files: Vec<PageFile>,
    pub truncated: bool,
}

fn attr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)\b(?:href|src|data-src|content)\s*=\s*(?:"([^"]+)"|'([^']+)'|([^\s"'<>]+))"#,
        )
        .unwrap()
    })
}

fn title_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap())
}

fn kind_of(ext: &str) -> Option<&'static str> {
    KINDS
        .iter()
        .find(|(_, exts)| exts.contains(&ext))
        .map(|(kind, _)| *kind)
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&#38;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

pub fn extract_files(base: &url::Url, html: &str) -> (Vec<PageFile>, bool) {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    let mut truncated = false;
    for cap in attr_re().captures_iter(html) {
        let raw = cap
            .get(1)
            .or_else(|| cap.get(2))
            .or_else(|| cap.get(3))
            .map(|m| unescape(m.as_str().trim()))
            .unwrap_or_default();
        if raw.is_empty() || raw.starts_with('#') || raw.starts_with("javascript:") {
            continue;
        }
        let Ok(mut link) = base.join(&raw) else {
            continue;
        };
        if link.scheme() != "http" && link.scheme() != "https" {
            continue;
        }
        link.set_fragment(None);
        let name = link
            .path_segments()
            .and_then(|mut s| s.next_back())
            .map(|s| {
                urlencoding::decode(s)
                    .map(|d| d.into_owned())
                    .unwrap_or_else(|_| s.to_string())
            })
            .unwrap_or_default();
        let Some((_, ext)) = name.rsplit_once('.') else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        let Some(kind) = kind_of(&ext) else {
            continue;
        };
        if !seen.insert(link.to_string()) {
            continue;
        }
        if files.len() == MAX_FILES {
            truncated = true;
            break;
        }
        files.push(PageFile {
            url: link.to_string(),
            name,
            ext,
            kind,
            size: None,
        });
    }
    (files, truncated)
}

pub fn page_title(html: &str) -> Option<String> {
    let t = title_re().captures(html)?.get(1)?.as_str();
    let t = unescape(&t.split_whitespace().collect::<Vec<_>>().join(" "));
    (!t.is_empty()).then_some(t)
}

fn client() -> Result<reqwest::Client, String> {
    omniget_core::core::http_client::apply_global_proxy(
        reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15")
            .timeout(std::time::Duration::from_secs(30)),
    )
    .build()
    .map_err(|e| e.to_string())
}

async fn head_size(client: reqwest::Client, url: String) -> Option<u64> {
    let r = client.head(&url).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.headers()
        .get(reqwest::header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

#[tauri::command]
pub async fn scan_page_files(url: String) -> Result<PageFiles, String> {
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    let base = url::Url::parse(url.trim()).map_err(|_| "This is not a web address".to_string())?;
    if base.scheme() != "http" && base.scheme() != "https" {
        return Err("This is not a web address".to_string());
    }
    let client = client()?;
    let mut res = client
        .get(base.clone())
        .send()
        .await
        .map_err(|e| format!("Could not open the page: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("The page answered HTTP {}", res.status().as_u16()));
    }
    let base = res.url().clone();
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| e.to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() >= MAX_HTML_BYTES {
            break;
        }
    }
    let html = String::from_utf8_lossy(&body);
    let (mut files, truncated) = extract_files(&base, &html);

    let urls: Vec<String> = files.iter().map(|f| f.url.clone()).collect();
    let mut sizes: Vec<Option<u64>> = Vec::with_capacity(urls.len());
    for chunk in urls.chunks(SIZE_LOOKUPS) {
        let mut batch = Vec::with_capacity(chunk.len());
        for u in chunk {
            batch.push(head_size(client.clone(), u.clone()));
        }
        sizes.extend(futures::future::join_all(batch).await);
    }
    for (f, s) in files.iter_mut().zip(sizes) {
        f.size = s;
    }
    Ok(PageFiles {
        page_url: base.to_string(),
        title: page_title(&html),
        files,
        truncated,
    })
}
