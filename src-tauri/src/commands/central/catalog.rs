//! Comandos Tauri do Catálogo da Central (`omniget_core::core::catalog`).
//!
//! Erros vão como `"CODE: mensagem"`. Trabalho de disco pesado (carregar o
//! índice, varrer, SQLite) roda em `spawn_blocking`.

use omniget_core::core::catalog::{
    collections, fetch, index, marketplace, registry,
    search::{self, Filters, Page, Sort},
    sources, CatalogError,
};
use serde_json::Value;

fn err(e: CatalogError) -> String {
    e.to_string()
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, CatalogError> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("CATALOG_IO: {e}"))?
        .map_err(err)
}

fn to_value<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("CATALOG_PARSE: {e}"))
}

/// Fonte, commit, contagens por tipo, data do índice e fontes extras.
#[tauri::command]
pub async fn catalog_meta() -> Result<Value, String> {
    blocking(index::meta).await
}

/// Busca paginada. `sort`: relevance|name|stars|updated.
#[tauri::command]
pub async fn catalog_search(
    query: Option<String>,
    filters: Option<Filters>,
    sort: Option<Sort>,
    page: Option<Page>,
) -> Result<Value, String> {
    let r = blocking(move || {
        let c = index::catalog()?;
        Ok(search::search(
            &c.items,
            query.as_deref().unwrap_or(""),
            &filters.unwrap_or_default(),
            sort.unwrap_or_default(),
            page.unwrap_or_default(),
        ))
    })
    .await?;
    to_value(r)
}

/// Facetas com contagem (tipo, categoria, fonte, licença, origin_tool).
#[tauri::command]
pub async fn catalog_facets(
    filters: Option<Filters>,
    query: Option<String>,
) -> Result<Value, String> {
    let r = blocking(move || {
        let c = index::catalog()?;
        Ok(search::facets(
            &c.items,
            query.as_deref().unwrap_or(""),
            &filters.unwrap_or_default(),
        ))
    })
    .await?;
    to_value(r)
}

/// Item completo (com frontmatter e lista de arquivos).
#[tauri::command]
pub async fn catalog_item(id: String) -> Result<Value, String> {
    let id2 = id.clone();
    let found = blocking(move || index::item(&id2)).await;
    let item = match found {
        Ok(i) => i,
        Err(e) if id.starts_with(&format!("{}:", registry::SOURCE_ID)) => {
            let _ = e;
            registry::item(&id).await.map_err(err)?
        }
        Err(e) => return Err(e),
    };
    to_value(item)
}

/// Baixa, confere (sha256 do índice / blob do git) e devolve caminho → conteúdo
/// (texto, ou base64 para binário).
#[tauri::command]
pub async fn catalog_item_files(id: String) -> Result<Value, String> {
    let id2 = id.clone();
    let item = match blocking(move || index::item(&id2)).await {
        Ok(i) => i,
        Err(_) if id.starts_with(&format!("{}:", registry::SOURCE_ID)) => {
            registry::item(&id).await.map_err(err)?
        }
        Err(e) => return Err(e),
    };
    // Fora do executor: o future de `item_files` não passa na checagem de
    // `Send` genérica do handler do Tauri.
    let files = tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(fetch::item_files(&item))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(err)?;
    to_value(files)
}

/// Official MCP Registry: busca por nome, paginada por cursor.
#[tauri::command]
pub async fn catalog_mcp_registry_search(
    query: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let page = registry::search(query.as_deref(), cursor.as_deref(), limit.unwrap_or(30))
        .await
        .map_err(err)?;
    to_value(page)
}

#[tauri::command]
pub async fn catalog_sources_list() -> Result<Value, String> {
    to_value(sources::list())
}

/// `spec`: `owner/repo[/path][@ref]`, URL do GitHub, pasta, ou `marketplace:owner/repo`.
/// `kind` (opcional) força `git`/`local`/`marketplace`.
#[tauri::command]
pub async fn catalog_sources_add(spec: String, kind: Option<String>) -> Result<Value, String> {
    to_value(sources::add(&spec, kind.as_deref()).await.map_err(err)?)
}

#[tauri::command]
pub async fn catalog_sources_rescan(id: String) -> Result<Value, String> {
    to_value(sources::rescan(&id).await.map_err(err)?)
}

#[tauri::command]
pub async fn catalog_sources_remove(id: String) -> Result<(), String> {
    blocking(move || sources::remove(&id)).await
}

/// Plugins de um marketplace Claude (`owner/repo`), buscados agora.
#[tauri::command]
pub async fn catalog_marketplace(repo: String) -> Result<Value, String> {
    to_value(marketplace::load(&repo).await.map_err(err)?)
}

/// Marketplaces curados (+ os já buscados), sem rede.
#[tauri::command]
pub async fn catalog_marketplaces() -> Result<Value, String> {
    let r = blocking(|| Ok(index::catalog()?.marketplaces.clone())).await?;
    to_value(r)
}

// ---------------------------------------------------------------- coleções

#[tauri::command]
pub async fn catalog_collections_list() -> Result<Value, String> {
    let r = blocking(|| collections::store()?.list()).await?;
    to_value(r)
}

#[tauri::command]
pub async fn catalog_collections_create(
    name: String,
    description: Option<String>,
    targets: Option<Vec<String>>,
    scope: Option<String>,
) -> Result<Value, String> {
    let r = blocking(move || {
        collections::store()?.create(
            &name,
            description.as_deref(),
            &targets.unwrap_or_default(),
            scope.as_deref(),
        )
    })
    .await?;
    to_value(r)
}

#[tauri::command]
pub async fn catalog_collections_rename(
    id: String,
    name: String,
    description: Option<String>,
) -> Result<Value, String> {
    let r =
        blocking(move || collections::store()?.rename(&id, &name, description.as_deref())).await?;
    to_value(r)
}

/// Alvos e escopo padrão da coleção.
#[tauri::command]
pub async fn catalog_collections_set_defaults(
    id: String,
    targets: Vec<String>,
    scope: Option<String>,
) -> Result<Value, String> {
    let r = blocking(move || collections::store()?.set_defaults(&id, &targets, scope.as_deref()))
        .await?;
    to_value(r)
}

#[tauri::command]
pub async fn catalog_collections_delete(id: String) -> Result<(), String> {
    blocking(move || collections::store()?.delete(&id)).await
}

#[tauri::command]
pub async fn catalog_collections_add(
    id: String,
    item_id: String,
    targets: Option<Vec<String>>,
    scope: Option<String>,
) -> Result<Value, String> {
    let r = blocking(move || {
        collections::store()?.add(
            &id,
            &item_id,
            &targets.unwrap_or_default(),
            scope.as_deref(),
        )
    })
    .await?;
    to_value(r)
}

#[tauri::command]
pub async fn catalog_collections_remove(id: String, item_id: String) -> Result<Value, String> {
    let r = blocking(move || collections::store()?.remove(&id, &item_id)).await?;
    to_value(r)
}

/// Move um item para a posição `to` (0 = primeiro).
#[tauri::command]
pub async fn catalog_collections_move(
    id: String,
    item_id: String,
    to: usize,
) -> Result<Value, String> {
    let r = blocking(move || collections::store()?.move_item(&id, &item_id, to)).await?;
    to_value(r)
}

/// Reordena as coleções.
#[tauri::command]
pub async fn catalog_collections_reorder(id: String, to: usize) -> Result<Value, String> {
    let r = blocking(move || collections::store()?.move_collection(&id, to)).await?;
    to_value(r)
}

/// Grava `<path>.omnistack` e devolve o caminho final.
#[tauri::command]
pub async fn catalog_collections_export(id: String, path: String) -> Result<String, String> {
    let p = blocking(move || collections::export(&id, std::path::Path::new(&path))).await?;
    Ok(p.to_string_lossy().into_owned())
}

/// Importa um `.omnistack`; devolve a coleção + itens ausentes/mudados.
#[tauri::command]
pub async fn catalog_collections_import(path: String) -> Result<Value, String> {
    let r = blocking(move || collections::import(std::path::Path::new(&path))).await?;
    to_value(r)
}

// ------------------------------------------------------------------ agentkit ↔ catálogo

/// Id do catálogo → componente canônico: item do índice (ou do MCP Registry),
/// arquivos baixados e conferidos (`fetch::item_bytes`, sha256/blob do git) e
/// `agentkit::parse::parse_item`. Chame fora do executor async (thread
/// bloqueante): o download roda em `block_on`.
pub fn resolve_catalog_component(
    id: &str,
) -> Result<
    omniget_core::core::agentkit::model::Component,
    omniget_core::core::agentkit::AgentkitError,
> {
    use omniget_core::core::agentkit::{parse, AgentkitError};
    let conv = |e: CatalogError| AgentkitError::new(e.code, e.message);
    let item = match index::item(id) {
        Ok(i) => i,
        Err(_) if id.starts_with(&format!("{}:", registry::SOURCE_ID)) => {
            tauri::async_runtime::block_on(registry::item(id)).map_err(conv)?
        }
        Err(e) => return Err(conv(e)),
    };
    let (files, _verified) =
        tauri::async_runtime::block_on(fetch::item_bytes(&item)).map_err(conv)?;
    let v = serde_json::to_value(&item)
        .map_err(|e| AgentkitError::new("CATALOG_PARSE", e.to_string()))?;
    // colisões de nome no disco o plano resolve (`<categoria>-<nome>`)
    parse::parse_item(&v, &files)
}

/// Registra o resolvedor de ids do catálogo no `agentkit` (idempotente). O
/// setup do app chama uma vez; os comandos `agentkit_*` e a ponte também
/// chamam antes de planejar, então funciona mesmo sem o setup.
pub fn install_catalog_resolver() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        omniget_core::core::agentkit::plan::set_catalog_resolver(resolve_catalog_component);
    });
}
