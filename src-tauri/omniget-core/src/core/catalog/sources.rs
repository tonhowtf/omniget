use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::model::CatalogItem;
use super::{agentkit_dir, catalog_dir, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Source {
    pub id: String,
    pub kind: String,
    pub spec: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    pub added: String,
    #[serde(default)]
    pub scanned: Option<String>,
    #[serde(default)]
    pub items: usize,
    #[serde(default)]
    pub excluded: usize,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SourcesFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    sources: Vec<Source>,
}

fn sources_path() -> Result<PathBuf> {
    Ok(agentkit_dir()?.join("sources.json"))
}

fn items_path(id: &str) -> Result<PathBuf> {
    Ok(catalog_dir()?.join("sources").join(format!("{id}.json")))
}

fn load_file() -> SourcesFile {
    sources_path()
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn list() -> Vec<Source> {
    load_file().sources
}

pub fn cached_items() -> Vec<(String, Vec<CatalogItem>)> {
    list()
        .into_iter()
        .filter(|s| s.kind != "marketplace")
        .filter_map(|s| {
            let bytes = std::fs::read(items_path(&s.id).ok()?).ok()?;
            let items: Vec<CatalogItem> = serde_json::from_slice(&bytes).ok()?;
            Some((s.id, items))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    Local(PathBuf),
    Git {
        repo: String,
        path: Option<String>,
        git_ref: Option<String>,
    },
    Marketplace(String),
}
