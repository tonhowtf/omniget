use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use omniget_core::core::llm::local_servers::{self, LocalKind};
use omniget_core::core::tools::ai_keys;
use once_cell::sync::Lazy;
use serde_json::{json, Value};

pub const CACHE_TTL: Duration = Duration::from_secs(3600);
pub const ERR_MODELS: &str = "ERR_LLM_MODELS";

struct Cached {
    at: Instant,
    models: Vec<String>,
}

static CACHE: Lazy<Mutex<HashMap<String, Cached>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn cached(provider: &str) -> Option<Vec<String>> {
    let cache = CACHE.lock().ok()?;
    let hit = cache.get(provider)?;
    if hit.at.elapsed() < CACHE_TTL {
        Some(hit.models.clone())
    } else {
        None
    }
}

fn store(provider: &str, models: &[String]) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.insert(
            provider.to_string(),
            Cached {
                at: Instant::now(),
                models: models.to_vec(),
            },
        );
    }
}

fn local_kind(provider: &str) -> Option<LocalKind> {
    LocalKind::all().into_iter().find(|k| k.id() == provider)
}

#[tauri::command]
pub async fn llm_models_list(provider: String, refresh: Option<bool>) -> Result<Value, String> {
    if refresh != Some(true) {
        if let Some(models) = cached(&provider) {
            return Ok(json!({ "provider": provider, "models": models, "cached": true }));
        }
    }

    let models = if let Some(kind) = local_kind(&provider) {
        let status = local_servers::detect(None).await;
        status
            .servers
            .into_iter()
            .find(|s| s.kind == kind)
            .map(|s| s.models)
            .unwrap_or_default()
    } else {
        let view = ai_keys::list()
            .into_iter()
            .find(|k| (k.id == provider || k.kind == provider) && k.has_key)
            .ok_or_else(|| format!("{ERR_MODELS}: no saved account for {provider}"))?;
        let entry =
            ai_keys::entry_with_secret(&view.id).map_err(|e| format!("{ERR_MODELS}: {e}"))?;
        ai_keys::models(&entry)
            .await
            .map_err(|e| format!("{ERR_MODELS}: {e}"))?
    };

    store(&provider, &models);
    Ok(json!({ "provider": provider, "models": models, "cached": false }))
}
