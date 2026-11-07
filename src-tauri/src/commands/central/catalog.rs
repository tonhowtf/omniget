use omniget_core::core::catalog::{fetch, index, registry, CatalogError};
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
    let files = tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(fetch::item_files(&item))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(err)?;
    to_value(files)
}

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
    parse::parse_item(&v, &files)
}

pub fn install_catalog_resolver() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        omniget_core::core::agentkit::plan::set_catalog_resolver(resolve_catalog_component);
    });
}
