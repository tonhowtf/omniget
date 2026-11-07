use omniget_core::core::ai::{self, AiConfigView, AiProvider};

#[tauri::command]
pub fn ai_get_config() -> AiConfigView {
    ai::get().view()
}

#[tauri::command]
pub fn ai_set_config(
    provider: AiProvider,
    kind: Option<String>,
    model: String,
    local_base_url: String,
    openai_key: Option<String>,
    anthropic_key: Option<String>,
) -> Result<AiConfigView, String> {
    let kind = kind
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty() && k != "none");
    match kind {
        Some(kind) if provider != AiProvider::None => {
            if ai::provider_for_kind(&kind) != provider {
                return Err(format!("provider {provider:?} does not match kind {kind}"));
            }
            ai::set_kind(&kind, model, local_base_url, openai_key, anthropic_key)
        }
        _ => ai::set(provider, model, local_base_url, openai_key, anthropic_key),
    }
    .map(|config| config.view())
}

#[tauri::command]
pub async fn ai_test() -> Result<String, String> {
    ai::chat(
        "You are a connectivity test. Reply with the single word: ok.",
        "ping",
    )
    .await
}
