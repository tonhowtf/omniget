//! Global Downloader: a Chromium the app drives over the DevTools Protocol,
//! a network sniffer on every tab, sandboxed JavaScript connectors that turn a
//! page into a tree (collection → folders → items) and a job runner that
//! downloads the tree.
//!
//! - [`browser`]: finds or downloads a Chromium, launches it on its own
//!   profile, and speaks CDP over one WebSocket (own client, no crate).
//! - [`sniffer`]: classifies the responses every tab sees (HLS, DASH, video,
//!   audio, subtitles, documents) and keeps a request log for connector work.
//! - [`connector`]: manifest + script in QuickJS; `ctx.fetch` only reaches
//!   the declared domains and runs inside a tab of that site when one is open.
//! - [`jobs`]: walks a tree, names files, refuses DRM, skips what exists.
//! - [`tools`]: the `gd_*` MCP/broker tools.

pub mod blocklist;
pub mod browser;
pub mod cdp;
pub mod connector;
pub mod jobs;
pub mod page;
pub mod profiles;
pub mod sniffer;
pub mod tools;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

/// `<data dir>/global-downloader`, created on first use.
pub fn data_dir() -> PathBuf {
    let dir = crate::core::paths::app_data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("global-downloader");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Everything the UI listens to goes through one channel as
/// `{ "kind": "...", ... }`; the app forwards it as the `global-dl:event`
/// Tauri event.
pub fn events() -> &'static broadcast::Sender<Value> {
    static TX: OnceLock<broadcast::Sender<Value>> = OnceLock::new();
    TX.get_or_init(|| broadcast::channel(512).0)
}

pub fn emit(event: Value) {
    let _ = events().send(event);
}

/// A connector's answer: a folder with children, or a downloadable item.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Node {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// folder | video | audio | file | subtitle | text | link
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// The media is behind a license (Widevine, FairPlay, PlayReady). The
    /// core never downloads it.
    #[serde(default)]
    pub drm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// Stable id the connector understands. An item whose source is `lazy`
    /// (or missing) is resolved with `resolveItem(key, ctx)` right before it
    /// downloads, and again when its signed URL has expired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Root only: the connector that made the tree, its folder and the page it
    /// was resolved from, so the job can call `resolveItem` later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Source {
    /// direct | hls | dash | ytdlp | text
    #[serde(default)]
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Body of a `text` item (an article, a note).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<String>,
    /// Exact file name inside the item's folder; otherwise `NN - title.ext`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Post-processing after the download: `srt` turns a WebVTT file into
    /// SubRip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub convert: Option<String>,
    /// Other renditions of the same `direct` file, by height (a page that
    /// loaded `…480p.webm` and `…1080p.webm`). The job takes the tallest one
    /// within its max height, as it does with an HLS master.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renditions: Vec<Rendition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Rendition {
    pub height: u32,
    pub url: String,
}

impl Source {
    /// The URL to fetch for `max_height`: the tallest rendition within it,
    /// else the shortest one (the rule `select_best_variant` follows for
    /// HLS), else `url`.
    pub fn url_for(&self, max_height: Option<u32>) -> Option<String> {
        let best = match max_height {
            Some(h) => self
                .renditions
                .iter()
                .filter(|r| r.height <= h)
                .max_by_key(|r| r.height)
                .or_else(|| self.renditions.iter().min_by_key(|r| r.height)),
            None => self.renditions.iter().max_by_key(|r| r.height),
        };
        best.map(|r| r.url.clone()).or_else(|| self.url.clone())
    }
}

impl Node {
    pub fn is_folder(&self) -> bool {
        self.kind == "folder"
            || (self.source.is_none() && self.key.is_none() && !self.children.is_empty())
    }

    /// Resolved only when it downloads.
    pub fn is_lazy(&self) -> bool {
        self.key.is_some()
            && self
                .source
                .as_ref()
                .is_none_or(|s| s.format == "lazy" || (s.url.is_none() && s.content.is_none()))
    }

    /// Gives every node a path id (`0`, `0.3`, `0.3.1`) so the UI can send a
    /// selection back.
    pub fn assign_ids(&mut self, prefix: &str) {
        self.id = prefix.to_string();
        if self.kind.is_empty() {
            self.kind = if self.source.is_none() && self.key.is_none() {
                "folder".into()
            } else {
                "file".into()
            };
        }
        for (i, child) in self.children.iter_mut().enumerate() {
            child.assign_ids(&format!("{}.{}", prefix, i));
        }
    }

    /// (items, drm items, folders) under this node.
    pub fn counts(&self) -> (usize, usize, usize) {
        if self.is_folder() {
            let mut acc = (0, 0, 1);
            for c in &self.children {
                let (a, b, f) = c.counts();
                acc.0 += a;
                acc.1 += b;
                acc.2 += f;
            }
            acc
        } else {
            (1, usize::from(self.drm), 0)
        }
    }
}

/// Host part of a URL, lowercased, without `www.`-style stripping.
pub fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

/// `host` is `domain` or a subdomain of it.
pub fn host_matches(host: &str, domain: &str) -> bool {
    let d = domain.trim_start_matches("*.").trim_start_matches('.');
    host == d || host.ends_with(&format!(".{}", d))
}

#[cfg(test)]
mod tests {
    use super::{Rendition, Source};

    #[test]
    fn rendition_follows_max_height() {
        let s = Source {
            format: "direct".into(),
            url: Some("https://u/x.1080p.webm".into()),
            renditions: vec![
                Rendition {
                    height: 480,
                    url: "https://u/x.480p.webm".into(),
                },
                Rendition {
                    height: 1080,
                    url: "https://u/x.1080p.webm".into(),
                },
            ],
            ..Default::default()
        };
        assert_eq!(s.url_for(None).unwrap(), "https://u/x.1080p.webm");
        assert_eq!(s.url_for(Some(720)).unwrap(), "https://u/x.480p.webm");
        assert_eq!(s.url_for(Some(240)).unwrap(), "https://u/x.480p.webm");
        assert_eq!(s.url_for(Some(2160)).unwrap(), "https://u/x.1080p.webm");
        let plain = Source {
            url: Some("https://u/a.mp4".into()),
            ..Default::default()
        };
        assert_eq!(plain.url_for(Some(360)).unwrap(), "https://u/a.mp4");
    }
}
