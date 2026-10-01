use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CatalogItem {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub category: String,
    #[serde(default)]
    pub description: String,
    pub source: ItemSource,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_origin")]
    pub origin_tool: String,
    #[serde(default)]
    pub files: Vec<ItemFile>,
    #[serde(default)]
    pub entry: String,
    #[serde(default)]
    pub frontmatter: Value,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub stars: Option<u64>,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub normalization: Vec<String>,
    #[serde(default)]
    pub security: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collides_with: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_name: Option<String>,
}

fn default_origin() -> String {
    "claude".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ItemSource {
    pub id: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ItemFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MarketplaceInfo {
    pub repo: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub stars: Option<u64>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub plugins: u64,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CatalogIndex {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub generated: String,
    #[serde(default)]
    pub sources: Vec<Value>,
    #[serde(default)]
    pub items: Vec<CatalogItem>,
    #[serde(default)]
    pub marketplaces: Vec<MarketplaceInfo>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SearchHit {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub source_id: String,
    pub repo: Option<String>,
    pub license: Option<String>,
    pub author: Option<String>,
    pub tags: Vec<String>,
    pub origin_tool: String,
    pub stars: Option<u64>,
    pub updated: Option<String>,
    pub file_count: usize,
    pub size: u64,
    pub collides: bool,
    pub marketplace: Option<String>,
    pub url: Option<String>,
}

impl From<&CatalogItem> for SearchHit {
    fn from(i: &CatalogItem) -> Self {
        Self {
            id: i.id.clone(),
            kind: i.kind.clone(),
            name: i.name.clone(),
            category: i.category.clone(),
            description: i.description.clone(),
            source_id: i.source.id.clone(),
            repo: i.source.repo.clone(),
            license: i.license.clone(),
            author: i.author.clone(),
            tags: i.tags.clone(),
            origin_tool: i.origin_tool.clone(),
            stars: i.stars,
            updated: i.updated.clone(),
            file_count: i.files.len(),
            size: i.files.iter().map(|f| f.size).sum(),
            collides: !i.collides_with.is_empty(),
            marketplace: i.source.marketplace.clone(),
            url: i.source.url.clone(),
        }
    }
}

pub fn kind_plural(kind: &str) -> &'static str {
    match kind {
        "agent" => "agents",
        "command" => "commands",
        "skill" => "skills",
        "mcp" => "mcps",
        "hook" => "hooks",
        "setting" => "settings",
        "statusline" => "statuslines",
        "loop" => "loops",
        "workflow" => "workflows",
        "mod" => "mods",
        "plugin" => "plugins",
        "rule" => "rules",
        "template" => "templates",
        "sandbox" => "sandbox",
        _ => "items",
    }
}

pub fn content_hash(item: &CatalogItem) -> Option<String> {
    if item.files.is_empty() {
        return None;
    }
    let mut files: Vec<&ItemFile> = item.files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut buf = Vec::new();
    for f in files {
        buf.extend_from_slice(f.path.as_bytes());
        buf.push(0);
        buf.extend_from_slice(f.sha256.as_bytes());
        buf.push(b'\n');
    }
    Some(super::sha256_hex(&buf))
}
