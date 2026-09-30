//! Downloads › Tools › Search sites: look a name up on several sites at once
//! and download a result, without opening each site first.
//!
//! The idea comes from MediaHarbor, which searches its services side by side
//! and turns a result into a download. Reimplemented here with yt-dlp's own
//! search prefixes and the Internet Archive's public search API, so every
//! source works without an account or a key; no code was copied.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::core::ytdlp;

const MAX_PER_SITE: u32 = 10;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SiteHit {
    pub title: String,
    pub url: String,
    pub author: Option<String>,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SiteResults {
    pub site: String,
    pub hits: Vec<SiteHit>,
    pub error: Option<String>,
}

/// The sites the tool knows, in the order the panel shows them.
pub const SITES: &[&str] = &[
    "youtube",
    "youtube_music",
    "soundcloud",
    "bilibili",
    "niconico",
    "archive",
];

/// What yt-dlp is asked for, for a site searched through yt-dlp.
fn ytdlp_query(site: &str, query: &str, n: u32) -> Option<String> {
    Some(match site {
        "youtube" => format!("ytsearch{}:{}", n, query),
        "soundcloud" => format!("scsearch{}:{}", n, query),
        "bilibili" => format!("bilisearch{}:{}", n, query),
        "niconico" => format!("nicosearch{}:{}", n, query),
        // yt-dlp reads YouTube Music's search page; `#songs` keeps songs only.
        "youtube_music" => format!(
            "https://music.youtube.com/search?q={}#songs",
            urlencoding::encode(query)
        ),
        _ => return None,
    })
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A page URL for a flat search entry. yt-dlp gives `url` for most sites; when
/// it only gives an id, the page is built for that site, never for another.
fn entry_url(site: &str, entry: &Value) -> Option<String> {
    if let Some(u) = str_of(entry, "webpage_url").or_else(|| str_of(entry, "url")) {
        if u.starts_with("http://") || u.starts_with("https://") {
            return Some(u);
        }
    }
    let id = str_of(entry, "id")?;
    match site {
        "youtube" => Some(format!("https://www.youtube.com/watch?v={}", id)),
        "youtube_music" => Some(format!("https://music.youtube.com/watch?v={}", id)),
        "bilibili" => Some(format!("https://www.bilibili.com/video/{}", id)),
        "niconico" => Some(format!("https://www.nicovideo.jp/watch/{}", id)),
        _ => None,
    }
}

fn thumbnail_of(entry: &Value) -> Option<String> {
    entry
        .get("thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|a| a.last())
        .and_then(|t| str_of(t, "url"))
        .or_else(|| str_of(entry, "thumbnail"))
}

/// Parses yt-dlp's `--flat-playlist --dump-json` output: one JSON object per line.
fn parse_ytdlp_lines(site: &str, stdout: &str, max: usize) -> Vec<SiteHit> {
    let mut hits: Vec<SiteHit> = Vec::new();
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(url) = entry_url(site, &entry) else {
            continue;
        };
        if hits.iter().any(|h| h.url == url) {
            continue;
        }
        hits.push(SiteHit {
            title: str_of(&entry, "title").unwrap_or_else(|| url.clone()),
            url,
            author: str_of(&entry, "uploader")
                .or_else(|| str_of(&entry, "channel"))
                .or_else(|| str_of(&entry, "artist")),
            duration: entry.get("duration").and_then(|v| v.as_f64()),
            thumbnail: thumbnail_of(&entry),
        });
        if hits.len() >= max {
            break;
        }
    }
    hits
}

/// Parses the Internet Archive's `advancedsearch.php` JSON.
fn parse_archive(body: &Value) -> Vec<SiteHit> {
    body.pointer("/response/docs")
        .and_then(|d| d.as_array())
        .map(|docs| {
            docs.iter()
                .filter_map(|d| {
                    let id = str_of(d, "identifier")?;
                    let author = match d.get("creator") {
                        Some(Value::Array(a)) => {
                            a.first().and_then(|x| x.as_str()).map(str::to_string)
                        }
                        Some(Value::String(s)) => Some(s.clone()),
                        _ => None,
                    };
                    Some(SiteHit {
                        title: str_of(d, "title").unwrap_or_else(|| id.clone()),
                        url: format!("https://archive.org/details/{}", id),
                        author,
                        duration: None,
                        thumbnail: Some(format!("https://archive.org/services/img/{}", id)),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn search_ytdlp(site: &str, query: &str, n: u32) -> Result<Vec<SiteHit>, String> {
    let target = ytdlp_query(site, query, n).ok_or_else(|| format!("unknown site {}", site))?;
    let ytdlp_path = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {}", e))?;
    let mut args: Vec<String> = [
        "--flat-playlist",
        "--dump-json",
        "--no-warnings",
        "--socket-timeout",
        "15",
        "--playlist-end",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    args.push(n.to_string());
    args.extend(omniget_core::core::ytdlp::insecure_tls_args());
    args.push(target);

    let run = omniget_core::core::ytdlp::ytdlp_command(&ytdlp_path)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    let output = tokio::time::timeout(Duration::from_secs(45), run)
        .await
        .map_err(|_| "the site took too long to answer".to_string())?
        .map_err(|e| format!("Failed to run yt-dlp: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let hits = parse_ytdlp_lines(site, &stdout, n as usize);
    if hits.is_empty() && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("search failed");
        return Err(last.trim().to_string());
    }
    Ok(hits)
}

async fn search_archive(query: &str, n: u32) -> Result<Vec<SiteHit>, String> {
    let builder = reqwest::Client::builder()
        .user_agent(concat!("OmniGet/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20));
    let client = omniget_core::core::http_client::apply_global_proxy(builder)
        .build()
        .map_err(|e| e.to_string())?;
    // Films and sound only: the same kinds of item the downloader can save.
    let q = format!("({}) AND mediatype:(movies OR audio)", query);
    let rows = n.to_string();
    let body: Value = client
        .get("https://archive.org/advancedsearch.php")
        .query(&[
            ("q", q.as_str()),
            ("fl[]", "identifier"),
            ("fl[]", "title"),
            ("fl[]", "creator"),
            ("rows", rows.as_str()),
            ("page", "1"),
            ("output", "json"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(parse_archive(&body))
}

#[tauri::command]
pub async fn search_sites(
    query: String,
    sites: Vec<String>,
    per_site: Option<u32>,
) -> Result<Vec<SiteResults>, String> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err("Type something to search for".to_string());
    }
    let n = per_site.unwrap_or(5).clamp(1, MAX_PER_SITE);
    let wanted: Vec<&str> = SITES
        .iter()
        .copied()
        .filter(|s| sites.iter().any(|w| w == s))
        .collect();
    if wanted.is_empty() {
        return Err("Pick at least one site".to_string());
    }

    // Every site at once: the slowest one sets the wait, not their sum.
    let searches = wanted.iter().map(|site| {
        let query = query.clone();
        async move {
            let result = if *site == "archive" {
                search_archive(&query, n).await
            } else {
                search_ytdlp(site, &query, n).await
            };
            match result {
                Ok(hits) => SiteResults {
                    site: site.to_string(),
                    hits,
                    error: None,
                },
                Err(e) => SiteResults {
                    site: site.to_string(),
                    hits: Vec::new(),
                    error: Some(e),
                },
            }
        }
    });
    Ok(futures::future::join_all(searches).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn each_site_gets_its_own_search() {
        assert_eq!(
            ytdlp_query("youtube", "big buck bunny", 5).unwrap(),
            "ytsearch5:big buck bunny"
        );
        assert_eq!(ytdlp_query("soundcloud", "x", 3).unwrap(), "scsearch3:x");
        assert_eq!(ytdlp_query("bilibili", "x", 3).unwrap(), "bilisearch3:x");
        assert_eq!(ytdlp_query("niconico", "x", 3).unwrap(), "nicosearch3:x");
        assert_eq!(
            ytdlp_query("youtube_music", "wing it & more", 5).unwrap(),
            "https://music.youtube.com/search?q=wing%20it%20%26%20more#songs"
        );
        assert!(ytdlp_query("archive", "x", 5).is_none());
    }

    #[test]
    fn an_entry_without_a_url_links_to_its_own_site() {
        let e = json!({ "id": "BV1eycyeyEec", "title": "WING IT!" });
        assert_eq!(
            entry_url("bilibili", &e).unwrap(),
            "https://www.bilibili.com/video/BV1eycyeyEec"
        );
        // SoundCloud entries without a URL are skipped, never sent to YouTube.
        assert!(entry_url("soundcloud", &json!({ "id": "123" })).is_none());
        let full = json!({ "id": "a", "url": "https://soundcloud.com/x/y" });
        assert_eq!(
            entry_url("soundcloud", &full).unwrap(),
            "https://soundcloud.com/x/y"
        );
    }

    #[test]
    fn flat_lines_become_hits_without_repeats() {
        let out = [
            json!({ "id": "aqz-KE-bpKQ", "url": "https://www.youtube.com/watch?v=aqz-KE-bpKQ", "title": "Big Buck Bunny", "channel": "Blender", "duration": 634.0 }).to_string(),
            "not json".to_string(),
            json!({ "id": "aqz-KE-bpKQ", "url": "https://www.youtube.com/watch?v=aqz-KE-bpKQ", "title": "again" }).to_string(),
            json!({ "id": "R6MlUcmOul8", "title": "Tears of Steel", "thumbnails": [{ "url": "a" }, { "url": "b" }] }).to_string(),
        ]
        .join("\n");
        let hits = parse_ytdlp_lines("youtube", &out, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].author.as_deref(), Some("Blender"));
        assert_eq!(hits[0].duration, Some(634.0));
        assert_eq!(hits[1].url, "https://www.youtube.com/watch?v=R6MlUcmOul8");
        assert_eq!(hits[1].thumbnail.as_deref(), Some("b"));
        assert_eq!(parse_ytdlp_lines("youtube", &out, 1).len(), 1);
    }

    #[test]
    fn archive_docs_become_detail_pages() {
        let body = json!({ "response": { "docs": [
            { "identifier": "The_Public_Domain_EP", "title": "The Public Domain EP", "creator": ["Hussalonia"] },
            { "identifier": "no_title", "creator": "Someone" },
            { "title": "no identifier" }
        ] } });
        let hits = parse_archive(&body);
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].url,
            "https://archive.org/details/The_Public_Domain_EP"
        );
        assert_eq!(hits[0].author.as_deref(), Some("Hussalonia"));
        assert_eq!(hits[1].title, "no_title");
        assert_eq!(hits[1].author.as_deref(), Some("Someone"));
        assert!(parse_archive(&json!({})).is_empty());
    }
}
