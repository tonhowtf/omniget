//! Busca, filtros, facetas e ordenação (funções puras sobre a lista de itens).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::model::{CatalogItem, SearchHit};

/// Filtros: dentro de um campo é OU, entre campos é E.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Filters {
    pub kinds: Vec<String>,
    pub categories: Vec<String>,
    /// `source.id` (`cct`, `owner/repo`, `mcp-registry`, `user-…`).
    pub sources: Vec<String>,
    /// SPDX; `unknown` casa item sem licença.
    pub licenses: Vec<String>,
    pub origin_tools: Vec<String>,
    pub tags: Vec<String>,
    /// Só plugins deste marketplace (`owner/repo`).
    pub marketplace: Option<String>,
    /// Esconde itens com colisão de nome.
    pub hide_collisions: bool,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    /// Relevância quando há texto; nome quando não há.
    #[default]
    Relevance,
    Name,
    Stars,
    Updated,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct Page {
    pub offset: usize,
    pub limit: usize,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 50,
        }
    }
}

pub const MAX_LIMIT: usize = 500;

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub items: Vec<SearchHit>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Facets {
    pub total: usize,
    pub kind: Vec<FacetValue>,
    pub category: Vec<FacetValue>,
    pub source: Vec<FacetValue>,
    pub license: Vec<FacetValue>,
    pub origin_tool: Vec<FacetValue>,
}

fn fold(s: &str) -> String {
    s.to_lowercase()
}

fn tokens(q: &str) -> Vec<String> {
    q.split(|c: char| c.is_whitespace() || c == ',')
        .map(|t| fold(t.trim()))
        .filter(|t| !t.is_empty())
        .collect()
}

/// Pontuação de um item para os tokens (0 = não casa). Todos os tokens precisam casar.
pub fn score(item: &CatalogItem, toks: &[String]) -> u32 {
    if toks.is_empty() {
        return 1;
    }
    let name = fold(&item.name);
    let desc = fold(&item.description);
    let cat = fold(&item.category);
    let id = fold(&item.id);
    let author = item.author.as_deref().map(fold).unwrap_or_default();
    let tags: Vec<String> = item.tags.iter().map(|t| fold(t)).collect();
    let mut total = 0u32;
    for t in toks {
        let mut s = 0u32;
        if name == *t {
            s += 100;
        } else if name.starts_with(t.as_str()) {
            s += 50;
        } else if name.contains(t.as_str()) {
            s += 30;
        }
        if tags.iter().any(|x| x == t) {
            s += 15;
        } else if tags.iter().any(|x| x.contains(t.as_str())) {
            s += 8;
        }
        if cat.contains(t.as_str()) {
            s += 10;
        }
        if desc.contains(t.as_str()) {
            s += 5;
        }
        if author.contains(t.as_str()) {
            s += 3;
        }
        if s == 0 && id.contains(t.as_str()) {
            s += 2;
        }
        if s == 0 {
            return 0;
        }
        total += s;
    }
    total
}

fn any_eq(list: &[String], v: &str) -> bool {
    list.is_empty() || list.iter().any(|x| x.eq_ignore_ascii_case(v))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Skip {
    None,
    Kind,
    Category,
    Source,
    License,
    Origin,
}

fn passes(item: &CatalogItem, f: &Filters, skip: Skip) -> bool {
    if skip != Skip::Kind && !any_eq(&f.kinds, &item.kind) {
        return false;
    }
    if skip != Skip::Category && !any_eq(&f.categories, &item.category) {
        return false;
    }
    if skip != Skip::Source && !any_eq(&f.sources, &item.source.id) {
        return false;
    }
    if skip != Skip::License && !any_eq(&f.licenses, item.license.as_deref().unwrap_or("unknown")) {
        return false;
    }
    if skip != Skip::Origin && !any_eq(&f.origin_tools, &item.origin_tool) {
        return false;
    }
    if !f.tags.is_empty()
        && !f
            .tags
            .iter()
            .any(|t| item.tags.iter().any(|x| x.eq_ignore_ascii_case(t)))
    {
        return false;
    }
    if let Some(m) = f.marketplace.as_deref().filter(|m| !m.is_empty()) {
        if item.source.marketplace.as_deref() != Some(m) {
            return false;
        }
    }
    if f.hide_collisions && !item.collides_with.is_empty() {
        return false;
    }
    true
}

fn cmp_name(a: &CatalogItem, b: &CatalogItem) -> Ordering {
    fold(&a.name)
        .cmp(&fold(&b.name))
        .then_with(|| a.id.cmp(&b.id))
}

/// Busca paginada.
pub fn search(
    items: &[CatalogItem],
    query: &str,
    filters: &Filters,
    sort: Sort,
    page: Page,
) -> SearchResult {
    let toks = tokens(query);
    let mut hits: Vec<(u32, &CatalogItem)> = items
        .iter()
        .filter(|i| passes(i, filters, Skip::None))
        .filter_map(|i| {
            let s = score(i, &toks);
            (s > 0).then_some((s, i))
        })
        .collect();
    let sort = if sort == Sort::Relevance && toks.is_empty() {
        Sort::Name
    } else {
        sort
    };
    match sort {
        Sort::Relevance => hits.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.stars.unwrap_or(0).cmp(&a.1.stars.unwrap_or(0)))
                .then_with(|| cmp_name(a.1, b.1))
        }),
        Sort::Name => hits.sort_by(|a, b| cmp_name(a.1, b.1)),
        Sort::Stars => hits.sort_by(|a, b| {
            b.1.stars
                .unwrap_or(0)
                .cmp(&a.1.stars.unwrap_or(0))
                .then_with(|| b.0.cmp(&a.0))
                .then_with(|| cmp_name(a.1, b.1))
        }),
        Sort::Updated => hits.sort_by(|a, b| {
            b.1.updated
                .as_deref()
                .unwrap_or("")
                .cmp(a.1.updated.as_deref().unwrap_or(""))
                .then_with(|| b.0.cmp(&a.0))
                .then_with(|| cmp_name(a.1, b.1))
        }),
    }
    let limit = page.limit.clamp(1, MAX_LIMIT);
    let total = hits.len();
    let items = hits
        .into_iter()
        .skip(page.offset)
        .take(limit)
        .map(|(_, i)| SearchHit::from(i))
        .collect();
    SearchResult {
        total,
        offset: page.offset,
        limit,
        items,
    }
}

fn to_facet(m: BTreeMap<String, usize>) -> Vec<FacetValue> {
    let mut v: Vec<FacetValue> = m
        .into_iter()
        .map(|(value, count)| FacetValue { value, count })
        .collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
    v
}

/// Facetas com contagem. Cada faceta ignora o próprio filtro (seleção múltipla).
pub fn facets(items: &[CatalogItem], query: &str, filters: &Filters) -> Facets {
    let toks = tokens(query);
    let matching: Vec<&CatalogItem> = items.iter().filter(|i| score(i, &toks) > 0).collect();
    let count = |skip: Skip, key: fn(&CatalogItem) -> String| {
        let mut m = BTreeMap::new();
        for i in matching.iter().filter(|i| passes(i, filters, skip)) {
            *m.entry(key(i)).or_insert(0usize) += 1;
        }
        to_facet(m)
    };
    Facets {
        total: matching
            .iter()
            .filter(|i| passes(i, filters, Skip::None))
            .count(),
        kind: count(Skip::Kind, |i| i.kind.clone()),
        category: count(Skip::Category, |i| i.category.clone()),
        source: count(Skip::Source, |i| i.source.id.clone()),
        license: count(Skip::License, |i| {
            i.license.clone().unwrap_or_else(|| "unknown".into())
        }),
        origin_tool: count(Skip::Origin, |i| i.origin_tool.clone()),
    }
}
