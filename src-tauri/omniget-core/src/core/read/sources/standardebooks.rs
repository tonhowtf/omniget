//! Standard Ebooks (https://standardebooks.org): carefully produced public
//! domain EPUBs. The full OPDS catalog is for Patrons Circle members; the
//! search and the new-releases feed are open, and that is what we use.
//!
//! Downloads need `?source=download`, otherwise the site answers with a
//! "thanks for downloading" page instead of the file.

use super::{Download, Hit, Page};
use crate::core::read::opds;

pub const BASE: &str = "https://standardebooks.org";

pub fn search_url(query: &str, page: u32) -> String {
    let mut u = url::Url::parse(&format!("{BASE}/feeds/opds/all")).expect("static url");
    u.query_pairs_mut()
        .append_pair("query", query.trim())
        .append_pair("page", &page.max(1).to_string())
        .append_pair("per-page", "48");
    u.to_string()
}

pub fn download_url(href: &str) -> String {
    if href.contains("source=") {
        href.replace("source=feed", "source=download")
    } else if href.contains('?') {
        format!("{href}&source=download")
    } else {
        format!("{href}?source=download")
    }
}

fn format_of(kind: &str, href: &str) -> (String, String) {
    if href.contains(".kepub.epub") {
        ("kepub".into(), "Kobo".into())
    } else if href.contains("_advanced.epub") {
        ("epub".into(), "EPUB 3 (advanced)".into())
    } else if href.ends_with(".epub") || href.contains(".epub?") {
        ("epub".into(), "EPUB".into())
    } else if href.contains(".azw3") {
        ("azw3".into(), "Kindle".into())
    } else {
        (
            kind.rsplit('/').next().unwrap_or("file").into(),
            kind.into(),
        )
    }
}

pub fn feed_to_page(f: opds::OpdsFeed) -> Page {
    let hits = f
        .entries
        .into_iter()
        .filter(|e| e.navigation.is_none() || !e.acquisitions.is_empty())
        .map(|e| {
            let downloads = e
                .acquisitions
                .iter()
                .map(|l| {
                    let (format, label) = format_of(&l.kind, &l.href);
                    Download {
                        url: download_url(&l.href),
                        format,
                        label,
                    }
                })
                .collect();
            let url = e.id.clone();
            Hit {
                source: "standardebooks".into(),
                key: e.id,
                title: e.title,
                authors: e.authors,
                cover: e.thumbnail.or(e.cover),
                summary: e.summary,
                language: e.language,
                year: None,
                subjects: e.categories,
                kind: "book".into(),
                downloads,
                url,
                extra: serde_json::Value::Null,
            }
        })
        .collect();
    Page {
        hits,
        total: f.total,
        next: f.next,
    }
}

/// Search when there is a query (or a cursor from a previous page), the
/// new-releases feed otherwise.
pub async fn browse(
    client: &reqwest::Client,
    query: &str,
    cursor: Option<&str>,
) -> Result<Page, String> {
    let (url, accept) = match (cursor, query.trim().is_empty()) {
        (Some(c), _) => (c.to_string(), "application/opds+json, application/atom+xml"),
        (None, false) => (search_url(query, 1), "application/opds+json"),
        (None, true) => (
            format!("{BASE}/feeds/atom/new-releases"),
            "application/atom+xml",
        ),
    };
    let r = client
        .get(&url)
        .header("accept", accept)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("{} from Standard Ebooks", r.status()));
    }
    let base = r.url().to_string();
    let body = r.text().await.map_err(|e| e.to_string())?;
    let mut feed = opds::parse(&body, &base)?;
    // The Atom new-releases feed links files as enclosures.
    if feed.entries.iter().all(|e| e.acquisitions.is_empty()) {
        let root = crate::core::read::xml::parse(&body);
        let mut entries = Vec::new();
        root.find_all("entry", &mut entries);
        for (i, e) in entries.iter().enumerate() {
            let Some(en) = feed.entries.get_mut(i) else {
                break;
            };
            for l in e.children_named("link") {
                if l.attr("rel") == Some("enclosure") {
                    en.acquisitions.push(opds::OpdsLink {
                        href: l.attr("href").unwrap_or("").to_string(),
                        rel: "enclosure".into(),
                        kind: l.attr("type").unwrap_or("").to_string(),
                        title: l.attr("title").unwrap_or("").to_string(),
                    });
                }
            }
            if en.thumbnail.is_none() {
                en.thumbnail = e
                    .find("thumbnail")
                    .and_then(|t| t.attr("url"))
                    .map(str::to_string);
            }
        }
    }
    Ok(feed_to_page(feed))
}
