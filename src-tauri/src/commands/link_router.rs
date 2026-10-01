use std::sync::Arc;

use serde::Serialize;

use crate::core::registry::PlatformRegistry;
use omniget_core::platforms::traits::PlatformDownloader;

pub const FORCEABLE_ENGINES: &[&str] = &["generic", "gallery", "direct_file"];

const MAX_LINKS: usize = 1000;

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct ParsedList {
    pub links: Vec<String>,
    pub duplicates: usize,
    pub ignored: usize,
    pub truncated: bool,
}

fn is_link(s: &str) -> bool {
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("magnet:")
        || s.starts_with("p2p:")
}

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

pub type ForcedEngine = (Arc<dyn PlatformDownloader>, String);

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
    pub engine: String,
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
