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

pub fn data_dir() -> PathBuf {
    let dir = crate::core::paths::app_data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("global-downloader");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn events() -> &'static broadcast::Sender<Value> {
    static TX: OnceLock<broadcast::Sender<Value>> = OnceLock::new();
    TX.get_or_init(|| broadcast::channel(512).0)
}

pub fn emit(event: Value) {
    let _ = events().send(event);
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Node {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(default)]
    pub drm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Source {
    #[serde(default)]
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub convert: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renditions: Vec<Rendition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Rendition {
    pub height: u32,
    pub url: String,
}

impl Source {
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

    pub fn is_lazy(&self) -> bool {
        self.key.is_some()
            && self
                .source
                .as_ref()
                .is_none_or(|s| s.format == "lazy" || (s.url.is_none() && s.content.is_none()))
    }

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

pub fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

pub fn host_matches(host: &str, domain: &str) -> bool {
    let d = domain.trim_start_matches("*.").trim_start_matches('.');
    host == d || host.ends_with(&format!(".{}", d))
}
