//! Formato do item do índice (contrato `01-contrato-catalogo.md`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Um item do catálogo: arquivos crus + metadados. Nunca carrega conteúdo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CatalogItem {
    /// `"<fonte>:<tipo plural>/<categoria>/<nome>"`.
    pub id: String,
    /// agent|command|skill|mcp|hook|setting|statusline|loop|workflow|mod|plugin|rule|template|sandbox
    pub kind: String,
    pub name: String,
    pub category: String,
    /// Curta (≤ 300 chars, sem blocos `<example>`).
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
    /// Caminhos relativos a `source.dir`. Vazio para plugins/registry (resolvidos na hora).
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
    /// Ids do mesmo tipo com o mesmo nome (a instalação achata a categoria).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collides_with: Vec<String>,
    /// Nome sugerido para instalar sem sobrescrever o colidente.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_name: Option<String>,
}

fn default_origin() -> String {
    "claude".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ItemSource {
    /// `cct`, `owner/repo` (marketplace), `mcp-registry`, `user-…`.
    pub id: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// Arquivo principal (ou pasta, para skill/mod/template/sandbox) no repo.
    #[serde(default)]
    pub path: String,
    /// Pasta base dos `files` no repo.
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
    /// Raiz local (fonte local ou cópia extraída de uma fonte Git).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ItemFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// Repo curado de marketplace/plugin Claude (lista de `generate_plugins_json.py`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MarketplaceInfo {
    pub repo: String,
    #[serde(default)]
    pub name: String,
    /// marketplace|plugin|none|unavailable
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

/// Item na lista de resultados: sem frontmatter nem lista de arquivos.
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

/// Plural usado no id (`agents`, `mcps`, `sandbox`…).
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

/// Tipo a partir do plural (inverso de [`kind_plural`]).
pub fn kind_from_plural(p: &str) -> Option<&'static str> {
    Some(match p {
        "agents" => "agent",
        "commands" => "command",
        "skills" => "skill",
        "mcps" => "mcp",
        "hooks" => "hook",
        "settings" => "setting",
        "statuslines" => "statusline",
        "loops" => "loop",
        "workflows" => "workflow",
        "mods" => "mod",
        "plugins" => "plugin",
        "rules" => "rule",
        "templates" => "template",
        "sandbox" => "sandbox",
        _ => return None,
    })
}

/// Hash de conteúdo de um item: sha256 de `path\0sha256\n` ordenado.
/// `None` quando o item não tem lista de arquivos (plugin/registry).
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
