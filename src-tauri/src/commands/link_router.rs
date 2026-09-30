//! Downloads › Tools › Engine per link: paste a list of links (or open a .txt),
//! see which engine OmniGet picks for each one, change it where you know
//! better, and queue them all.
//!
//! The idea comes from Media Downloader (mhogomchungu/media-downloader), whose
//! Batch tab reads links from a file and lets each row use its own engine
//! (yt-dlp, gallery-dl, aria2c...), with URL rules as the default.
//! Reimplemented on top of OmniGet's own routing; no code was copied.

use std::sync::Arc;

use serde::Serialize;

use crate::core::registry::PlatformRegistry;
use omniget_core::platforms::traits::PlatformDownloader;

/// Engines a person can force on a web link. Torrents, magnets and P2P codes
/// keep their own engine.
pub const FORCEABLE_ENGINES: &[&str] = &["generic", "gallery", "direct_file"];

/// Most links one check takes, so a huge file cannot stall the window.
const MAX_LINKS: usize = 1000;

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct ParsedList {
    pub links: Vec<String>,
    /// Links that were already in the list.
    pub duplicates: usize,
    /// Non-empty lines (or words) that are not links. `#` comments do not count.
    pub ignored: usize,
    /// True when the list was cut at `MAX_LINKS`.
    pub truncated: bool,
}

fn is_link(s: &str) -> bool {
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("magnet:")
        || s.starts_with("p2p:")
}

/// One link per line, or several on a line separated by spaces. Blank lines
/// and `#` comments are skipped, `url|anything` keeps the URL (the format the
/// batch file of the home screen already accepts), and repeats are dropped.
pub fn parse_link_list(text: &str) -> ParsedList {
    let mut out = ParsedList::default();
    let mut seen = std::collections::HashSet::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        for word in line.split_whitespace() {
            let candidate = word.split('|').next().unwrap_or(word).trim();
            if !is_link(candidate) {
                out.ignored += 1;
                continue;
            }
            if !seen.insert(candidate.to_string()) {
                out.duplicates += 1;
                continue;
            }
            if out.links.len() == MAX_LINKS {
                out.truncated = true;
                continue;
            }
            out.links.push(candidate.to_string());
        }
    }
    out
}

/// A downloader and the platform name the queue records for it.
pub type ForcedEngine = (Arc<dyn PlatformDownloader>, String);

/// The downloader for an engine the person picked, or `None` for "auto".
pub fn forced_engine(
    registry: &PlatformRegistry,
    engine: Option<&str>,
) -> Result<Option<ForcedEngine>, String> {
    let name = match engine.map(str::trim) {
        None | Some("") | Some("auto") => return Ok(None),
        Some(name) => name,
    };
    if !FORCEABLE_ENGINES.contains(&name) {
        return Err(format!("Unknown engine: {name}"));
    }
    let downloader = registry
        .find_by_name(name)
        .ok_or_else(|| format!("Engine not available: {name}"))?;
    Ok(Some((downloader, name.to_string())))
}

#[derive(Debug, Clone, Serialize)]
pub struct RoutedLink {
    pub url: String,
    /// Name of the downloader OmniGet picks on its own (`youtube`, `generic`,
    /// `gallery`, `direct_file`, `magnet`...). Empty when none takes the link.
    pub engine: String,
    /// Engines that can be forced on this link (empty: it keeps its own).
    pub choices: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteReport {
    pub links: Vec<RoutedLink>,
    pub duplicates: usize,
    pub ignored: usize,
    pub truncated: bool,
}

async fn route_one(registry: &PlatformRegistry, url: String) -> RoutedLink {
    let engine = crate::core::url_resolver::resolve_downloader(registry, &url)
        .await
        .map(|r| r.downloader.name().to_string())
        .unwrap_or_default();
    let web = url.starts_with("http://") || url.starts_with("https://");
    RoutedLink {
        choices: if web && !engine.is_empty() {
            FORCEABLE_ENGINES.to_vec()
        } else {
            Vec::new()
        },
        url,
        engine,
    }
}

#[tauri::command]
pub async fn route_links(
    state: tauri::State<'_, crate::AppState>,
    text: String,
) -> Result<RouteReport, String> {
    let parsed = parse_link_list(&text);
    let registry = &state.registry;
    // The direct-file check sends a short HEAD to unknown hosts: a few at a
    // time, so a long list neither waits in line nor floods one server.
    let mut links = Vec::with_capacity(parsed.links.len());
    for chunk in parsed.links.chunks(6) {
        let mut batch = Vec::with_capacity(chunk.len());
        for url in chunk {
            batch.push(route_one(registry, url.clone()));
        }
        links.extend(futures::future::join_all(batch).await);
    }
    Ok(RouteReport {
        links,
        duplicates: parsed.duplicates,
        ignored: parsed.ignored,
        truncated: parsed.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_links_in_order_and_skips_comments_blanks_and_repeats() {
        let text = "# my list\n\nhttps://a.example/1\n  https://b.example/2  \nhttps://a.example/1\nnot a link\n";
        let parsed = parse_link_list(text);
        assert_eq!(
            parsed.links,
            vec!["https://a.example/1", "https://b.example/2"]
        );
        assert_eq!(parsed.duplicates, 1);
        assert_eq!(parsed.ignored, 3);
        assert!(!parsed.truncated);
    }

    #[test]
    fn splits_words_and_pipe_suffixes() {
        let text = "https://a.example/1 https://b.example/2\nhttps://c.example/3|best\nmagnet:?xt=urn:btih:abc";
        let parsed = parse_link_list(text);
        assert_eq!(
            parsed.links,
            vec![
                "https://a.example/1",
                "https://b.example/2",
                "https://c.example/3",
                "magnet:?xt=urn:btih:abc"
            ]
        );
    }

    #[test]
    fn stops_at_the_limit() {
        let text: String = (0..MAX_LINKS + 5)
            .map(|i| format!("https://a.example/{i}\n"))
            .collect();
        let parsed = parse_link_list(&text);
        assert_eq!(parsed.links.len(), MAX_LINKS);
        assert!(parsed.truncated);
    }

    #[test]
    fn auto_and_unknown_engines() {
        let registry = PlatformRegistry::new();
        assert!(forced_engine(&registry, None).unwrap().is_none());
        assert!(forced_engine(&registry, Some("auto")).unwrap().is_none());
        assert!(forced_engine(&registry, Some("magnet")).is_err());
        // Known name, but this empty registry has no such downloader.
        assert!(forced_engine(&registry, Some("generic")).is_err());
    }
}
