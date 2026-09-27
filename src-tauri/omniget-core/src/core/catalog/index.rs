//! Índice carregado: embutido no binário ou atualizado em
//! `<app_data>/agentkit/catalog/`, mesclado com os itens das fontes do usuário
//! e das listagens de marketplace já buscadas.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, OnceLock, RwLock};

use serde::Serialize;
use serde_json::Value;

use super::model::{CatalogIndex, CatalogItem, MarketplaceInfo};
use super::{catalog_dir, CatalogError, Result, ERR_NOT_FOUND, ERR_PARSE};

/// Índice gerado por `scripts/agentkit-catalog/build.mjs` (passa de 4 MB, então vai o gz).
static EMBEDDED_INDEX_GZ: &[u8] =
    include_bytes!("../../../../../static/agentkit/catalog-index.json.gz");
/// Metadados do mesmo build.
static EMBEDDED_META: &str = include_str!("../../../../../static/agentkit/catalog-meta.json");

pub const INDEX_FILE_GZ: &str = "catalog-index.json.gz";
pub const INDEX_FILE: &str = "catalog-index.json";

/// Catálogo pronto para busca.
pub struct Catalog {
    pub items: Vec<CatalogItem>,
    pub by_id: HashMap<String, usize>,
    pub marketplaces: Vec<MarketplaceInfo>,
    pub base: IndexInfo,
    pub extra_counts: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexInfo {
    /// `embedded` ou `updated`.
    pub loaded_from: String,
    pub generated: String,
    pub sources: Vec<Value>,
    pub base_items: usize,
}

impl Catalog {
    pub fn get(&self, id: &str) -> Option<&CatalogItem> {
        self.by_id.get(id).map(|&i| &self.items[i])
    }
}

static CATALOG: OnceLock<RwLock<Option<Arc<Catalog>>>> = OnceLock::new();

fn cell() -> &'static RwLock<Option<Arc<Catalog>>> {
    CATALOG.get_or_init(|| RwLock::new(None))
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|e| CatalogError::new(ERR_PARSE, format!("gunzip: {e}")))?;
    Ok(out)
}

/// Decodifica bytes de índice (gz ou JSON puro).
pub fn decode_index(bytes: &[u8]) -> Result<CatalogIndex> {
    let raw = if bytes.starts_with(&[0x1f, 0x8b]) {
        gunzip(bytes)?
    } else {
        bytes.to_vec()
    };
    serde_json::from_slice::<CatalogIndex>(&raw)
        .map_err(|e| CatalogError::new(ERR_PARSE, format!("catalog index: {e}")))
}

/// Índice embutido.
pub fn embedded_index() -> Result<CatalogIndex> {
    decode_index(EMBEDDED_INDEX_GZ)
}

/// Metadados embutidos (`catalog-meta.json`).
pub fn embedded_meta() -> Value {
    serde_json::from_str(EMBEDDED_META).unwrap_or(Value::Null)
}

/// Índice atualizado em disco, se existir e for mais novo que o embutido.
fn updated_index(embedded_generated: &str) -> Option<CatalogIndex> {
    let dir = catalog_dir().ok()?;
    for name in [INDEX_FILE_GZ, INDEX_FILE] {
        let p = dir.join(name);
        if let Ok(bytes) = std::fs::read(&p) {
            match decode_index(&bytes) {
                Ok(ix) if ix.generated.as_str() > embedded_generated => return Some(ix),
                Ok(_) => {}
                Err(e) => tracing::warn!("[catalog] ignoring {}: {e}", p.display()),
            }
        }
    }
    None
}

/// Grava um índice novo (gz ou JSON) em `<app_data>/agentkit/catalog/` e recarrega.
pub fn install_index(bytes: &[u8]) -> Result<()> {
    let ix = decode_index(bytes)?;
    if ix.items.is_empty() {
        return Err(CatalogError::new(ERR_PARSE, "catalog index has no items"));
    }
    let name = if bytes.starts_with(&[0x1f, 0x8b]) {
        INDEX_FILE_GZ
    } else {
        INDEX_FILE
    };
    super::write_atomic(&catalog_dir()?.join(name), bytes)?;
    invalidate();
    Ok(())
}

fn build() -> Result<Catalog> {
    let embedded = embedded_index()?;
    let (base, loaded_from) = match updated_index(&embedded.generated) {
        Some(ix) => (ix, "updated"),
        None => (embedded, "embedded"),
    };
    let info = IndexInfo {
        loaded_from: loaded_from.into(),
        generated: base.generated.clone(),
        sources: base.sources.clone(),
        base_items: base.items.len(),
    };
    let mut items = base.items;
    let mut marketplaces = base.marketplaces;
    let mut extra_counts = HashMap::new();
    // fontes do usuário (varridas e guardadas em cache)
    for (sid, extra) in super::sources::cached_items() {
        extra_counts.insert(sid, extra.len());
        items.extend(extra);
    }
    // listagens de marketplace buscadas (curadas atualizadas ou adicionadas)
    for (info, extra) in super::marketplace::cached_listings() {
        let repo = info.repo.clone();
        items.retain(|i| i.source.marketplace.as_deref() != Some(repo.as_str()));
        extra_counts.insert(format!("marketplace:{repo}"), extra.len());
        items.extend(extra);
        if let Some(m) = marketplaces.iter_mut().find(|m| m.repo == repo) {
            *m = info;
        } else {
            marketplaces.push(info);
        }
    }
    let mut by_id = HashMap::with_capacity(items.len());
    let mut dedup = Vec::with_capacity(items.len());
    for it in items {
        if let Some(&pos) = by_id.get(&it.id) {
            // o mais recente (fonte do usuário/marketplace) vence
            dedup[pos] = it;
        } else {
            by_id.insert(it.id.clone(), dedup.len());
            dedup.push(it);
        }
    }
    Ok(Catalog {
        items: dedup,
        by_id,
        marketplaces,
        base: info,
        extra_counts,
    })
}

/// Catálogo atual (constrói na primeira chamada).
pub fn catalog() -> Result<Arc<Catalog>> {
    if let Some(c) = cell().read().ok().and_then(|g| g.clone()) {
        return Ok(c);
    }
    let built = Arc::new(build()?);
    if let Ok(mut g) = cell().write() {
        *g = Some(built.clone());
    }
    Ok(built)
}

/// Descarta o catálogo em memória (fontes mudaram).
pub fn invalidate() {
    if let Ok(mut g) = cell().write() {
        *g = None;
    }
}

/// Um item por id: índice + fontes + marketplaces + MCP Registry em memória.
pub fn item(id: &str) -> Result<CatalogItem> {
    let c = catalog()?;
    if let Some(i) = c.get(id) {
        return Ok(i.clone());
    }
    if let Some(i) = super::registry::cached_item(id) {
        return Ok(i);
    }
    Err(CatalogError::new(ERR_NOT_FOUND, format!("item `{id}`")))
}

/// Metadados para a tela: fonte, commit, contagens por tipo, data.
pub fn meta() -> Result<Value> {
    let c = catalog()?;
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for it in &c.items {
        *counts.entry(it.kind.clone()).or_default() += 1;
    }
    let embedded = embedded_meta();
    Ok(serde_json::json!({
        "loaded_from": c.base.loaded_from,
        "generated": c.base.generated,
        "sources": c.base.sources,
        "base_items": c.base.base_items,
        "total": c.items.len(),
        "counts": counts,
        "extra_sources": c.extra_counts,
        "marketplaces": c.marketplaces,
        "build": embedded,
    }))
}
