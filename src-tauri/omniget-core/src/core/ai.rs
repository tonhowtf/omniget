use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

const AI_CONFIG_FILE: &str = "ai_config.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    #[default]
    None,
    Openai,
    Anthropic,
    Local,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default)]
    pub provider: AiProvider,
    #[serde(default, skip_serializing)]
    pub openai_key: String,
    #[serde(default, skip_serializing)]
    pub anthropic_key: String,
    #[serde(default)]
    pub local_base_url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub key_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AiConfigView {
    pub provider: AiProvider,
    pub kind: String,
    pub model: String,
    pub local_base_url: String,
    pub has_openai_key: bool,
    pub has_anthropic_key: bool,
}

impl AiConfig {
    pub fn view(&self) -> AiConfigView {
        AiConfigView {
            provider: self.provider,
            kind: self.provider_id(),
            model: self.model.clone(),
            local_base_url: self.local_base_url.clone(),
            has_openai_key: !self.openai_key.is_empty(),
            has_anthropic_key: !self.anthropic_key.is_empty(),
        }
    }

    pub fn is_configured(&self) -> bool {
        match self.provider {
            AiProvider::None => false,
            AiProvider::Openai => !self.openai_key.is_empty(),
            AiProvider::Anthropic => !self.anthropic_key.is_empty(),
            AiProvider::Local => {
                !self.local_base_url.is_empty()
                    && (!self.kind_needs_key() || !self.openai_key.is_empty())
            }
        }
    }

    fn kind_needs_key(&self) -> bool {
        crate::core::tools::ai_keys::kind_of(&self.provider_id()).needs_key()
    }
}

static STORE: OnceLock<Mutex<AiConfig>> = OnceLock::new();

fn store() -> &'static Mutex<AiConfig> {
    STORE.get_or_init(|| Mutex::new(load_from_disk()))
}

fn file_path() -> Option<std::path::PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(AI_CONFIG_FILE))
}

fn load_from_disk() -> AiConfig {
    let Some(path) = file_path() else {
        return AiConfig::default();
    };
    let mut cfg: AiConfig = match std::fs::read_to_string(&path) {
        Ok(c) => serde_json::from_str(&c).unwrap_or_default(),
        Err(_) => AiConfig::default(),
    };
    crate::core::tools::ai_keys::migrate::migrate_config_file(&path, &mut cfg);
    hydrate_keys(&mut cfg);
    cfg
}

fn stored_key(account: &str) -> String {
    use crate::core::secrets::{self, AI_KEYS};
    secrets::get(AI_KEYS, account)
        .unwrap_or_else(|e| {
            tracing::warn!("[ai] could not read {}: {}", account, e);
            None
        })
        .unwrap_or_default()
}

fn hydrate_keys(cfg: &mut AiConfig) {
    use crate::core::tools::ai_keys::migrate;
    cfg.openai_key = stored_key(migrate::APP_OPENAI_ACCOUNT);
    cfg.anthropic_key = stored_key(migrate::APP_ANTHROPIC_ACCOUNT);
}

fn write_config_file(path: &std::path::Path, cfg: &AiConfig) -> Result<(), String> {
    use std::io::Write;

    let parent = path
        .parent()
        .ok_or_else(|| "Could not find the AI settings directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create the AI settings directory: {e}"))?;
    let serialized = serde_json::to_string_pretty(cfg)
        .map_err(|e| format!("Could not encode AI settings: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(serialized.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Could not save AI settings: {e}"));
    }
    Ok(())
}

fn persist_config(
    previous: &AiConfig,
    next: &AiConfig,
    path: &std::path::Path,
    mut put_key: impl FnMut(&str, &str) -> Result<(), String>,
) -> Result<(), String> {
    use crate::core::tools::ai_keys::migrate;

    let keys = [
        (
            migrate::APP_OPENAI_ACCOUNT,
            previous.openai_key.trim(),
            next.openai_key.trim(),
        ),
        (
            migrate::APP_ANTHROPIC_ACCOUNT,
            previous.anthropic_key.trim(),
            next.anthropic_key.trim(),
        ),
    ];
    let mut written: Vec<(&'static str, String)> = Vec::new();
    for (account, old, value) in keys {
        if old == value {
            continue;
        }
        if let Err(error) = put_key(account, value) {
            let rollback_failed = restore_keys(&written, &mut put_key);
            return Err(persistence_error(error, rollback_failed));
        }
        written.push((account, old.to_string()));
    }

    if let Err(error) = write_config_file(path, next) {
        let rollback_failed = restore_keys(&written, &mut put_key);
        return Err(persistence_error(error, rollback_failed));
    }
    Ok(())
}

fn restore_keys(
    written: &[(&'static str, String)],
    put_key: &mut impl FnMut(&str, &str) -> Result<(), String>,
) -> bool {
    let mut failed = false;
    for (account, previous_value) in written.iter().rev() {
        if put_key(account, previous_value).is_err() {
            failed = true;
        }
    }
    failed
}

fn persistence_error(error: String, rollback_failed: bool) -> String {
    if rollback_failed {
        format!("{error}. The previous credential state could not be fully restored.")
    } else {
        error
    }
}

fn persist_current(previous: &AiConfig, next: &AiConfig) -> Result<(), String> {
    use crate::core::secrets::{self, AI_KEYS};

    let path = file_path().ok_or_else(|| "Could not find the AI settings file".to_string())?;
    persist_config(previous, next, &path, |account, value| {
        secrets::put_or_delete(AI_KEYS, account, value)
    })
}

pub fn get() -> AiConfig {
    store().lock().unwrap().clone()
}

pub fn set(
    provider: AiProvider,
    model: String,
    local_base_url: String,
    openai_key: Option<String>,
    anthropic_key: Option<String>,
) -> Result<AiConfig, String> {
    let mut guard = store().lock().unwrap();
    let previous = guard.clone();
    let mut next = previous.clone();
    next.provider = provider;
    next.model = model.trim().to_string();
    next.local_base_url = local_base_url.trim().trim_end_matches('/').to_string();
    if let Some(k) = openai_key {
        next.openai_key = k.trim().to_string();
    }
    if let Some(k) = anthropic_key {
        next.anthropic_key = k.trim().to_string();
    }
    clear_changed_connection_metadata(&previous, &mut next);
    persist_current(&previous, &next)?;
    *guard = next.clone();
    Ok(next)
}

pub fn set_kind(
    kind: &str,
    model: String,
    base_url: String,
    openai_key: Option<String>,
    anthropic_key: Option<String>,
) -> Result<AiConfig, String> {
    let mut guard = store().lock().unwrap();
    let previous = guard.clone();
    let next = apply_kind(&previous, kind, model, base_url, openai_key, anthropic_key);
    persist_current(&previous, &next)?;
    *guard = next.clone();
    Ok(next)
}

fn apply_kind(
    previous: &AiConfig,
    kind: &str,
    model: String,
    base_url: String,
    openai_key: Option<String>,
    anthropic_key: Option<String>,
) -> AiConfig {
    let kind = kind.trim();
    let provider = provider_for_kind(kind);
    let switched = previous.provider != AiProvider::None && previous.provider_id() != kind;
    let mut next = previous.clone();
    next.provider = provider;
    next.model = model.trim().to_string();
    next.local_base_url = match provider {
        AiProvider::Openai | AiProvider::Anthropic | AiProvider::None => String::new(),
        AiProvider::Local
            if base_url.trim().is_empty()
                && kind != "ollama"
                && crate::core::tools::ai_keys::kind_of(kind).base_url_editable() =>
        {
            String::new()
        }
        AiProvider::Local => crate::core::tools::ai_keys::app_base_url(kind, &base_url),
    };
    let (openai_key, anthropic_key) = match (switched, provider) {
        (true, AiProvider::Anthropic) => (openai_key, anthropic_key.or(Some(String::new()))),
        (true, _) => (openai_key.or(Some(String::new())), anthropic_key),
        (false, _) => (openai_key, anthropic_key),
    };
    if let Some(k) = openai_key {
        next.openai_key = k.trim().to_string();
    }
    if let Some(k) = anthropic_key {
        next.anthropic_key = k.trim().to_string();
    }
    if switched
        || previous.local_base_url != next.local_base_url
        || previous.openai_key != next.openai_key
        || previous.anthropic_key != next.anthropic_key
    {
        next.key_id.clear();
    }
    next.kind = kind.to_string();
    next
}

fn clear_changed_connection_metadata(previous: &AiConfig, next: &mut AiConfig) {
    if previous.provider != next.provider || previous.local_base_url != next.local_base_url {
        next.kind.clear();
        next.key_id.clear();
    } else if previous.openai_key != next.openai_key || previous.anthropic_key != next.anthropic_key
    {
        next.key_id.clear();
    }
}

pub fn provider_for_kind(kind: &str) -> AiProvider {
    match crate::core::tools::ai_keys::kind_of(kind).wire {
        "anthropic" => AiProvider::Anthropic,
        _ if kind == "openai" => AiProvider::Openai,
        _ => AiProvider::Local,
    }
}

impl AiConfig {
    pub fn provider_id(&self) -> String {
        if !self.kind.is_empty() && provider_for_kind(&self.kind) == self.provider {
            return self.kind.clone();
        }
        match self.provider {
            AiProvider::None => String::new(),
            AiProvider::Openai => "openai".to_string(),
            AiProvider::Anthropic => "anthropic".to_string(),
            AiProvider::Local => "custom".to_string(),
        }
    }
}

fn http_client() -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120));
    crate::core::http_client::apply_global_proxy(builder)
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))
}

pub fn provider_from_config(
    cfg: &AiConfig,
) -> Result<std::sync::Arc<dyn crate::core::llm::providers::Provider>, String> {
    use crate::core::llm::providers::anthropic::AnthropicProvider;
    use crate::core::llm::providers::openai_compat::OpenAiCompat;
    use crate::core::llm::types::ProviderId;

    let id = ProviderId::new(cfg.provider_id());
    match cfg.provider {
        AiProvider::None => Err("AI is not configured".to_string()),
        AiProvider::Anthropic => {
            AnthropicProvider::new(cfg.local_base_url.clone(), cfg.anthropic_key.clone())
                .map(|p| {
                    std::sync::Arc::new(p)
                        as std::sync::Arc<dyn crate::core::llm::providers::Provider>
                })
                .map_err(|e| e.to_string())
        }
        AiProvider::Openai => {
            OpenAiCompat::new(id, "https://api.openai.com/v1", cfg.openai_key.clone())
                .map(|p| {
                    std::sync::Arc::new(p)
                        as std::sync::Arc<dyn crate::core::llm::providers::Provider>
                })
                .map_err(|e| e.to_string())
        }
        AiProvider::Local => {
            if cfg.local_base_url.is_empty() {
                return Err("No local endpoint configured".to_string());
            }
            OpenAiCompat::new(id, cfg.local_base_url.clone(), cfg.openai_key.clone())
                .map(|p| {
                    std::sync::Arc::new(p)
                        as std::sync::Arc<dyn crate::core::llm::providers::Provider>
                })
                .map_err(|e| e.to_string())
        }
    }
}

fn is_free_endpoint(cfg: &AiConfig) -> bool {
    if cfg.kind == "ollama" {
        return true;
    }
    let base = cfg.local_base_url.to_ascii_lowercase();
    base.contains("://localhost")
        || base.contains("://127.0.0.1")
        || base.contains("://[::1]")
        || base.contains("://0.0.0.0")
}

pub async fn chat(system: &str, user: &str) -> Result<String, String> {
    let cfg = get();
    if cfg.model.is_empty() {
        return Err("No AI model configured".to_string());
    }
    chat_with(
        &cfg,
        system,
        user,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
}

pub async fn chat_with(
    cfg: &AiConfig,
    system: &str,
    user: &str,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<String, String> {
    use crate::core::llm::types::{
        GenParams, Message, ModelRef, ProviderId, Role, TurnEvent, TurnRequest,
    };
    use futures::StreamExt;

    let provider = provider_from_config(cfg)?;
    let mut messages = Vec::with_capacity(2);
    if !system.trim().is_empty() {
        messages.push(Message::text(Role::System, system));
    }
    messages.push(Message::text(Role::User, user));
    let req = TurnRequest {
        model: ModelRef {
            provider: ProviderId::new(cfg.provider_id()),
            model: cfg.model.clone(),
        },
        messages,
        tools: Vec::new(),
        params: GenParams::default(),
        cancel,
        agent_id: None,
    };

    let mut stream = provider.turn(req).await.map_err(|e| e.to_string())?;
    let mut text = String::new();
    let mut reported: Option<crate::core::llm::types::Usage> = None;
    let mut failure: Option<crate::core::llm::error::LlmError> = None;
    while let Some(ev) = stream.next().await {
        match ev {
            TurnEvent::TextDelta { text: t } => text.push_str(&t),
            TurnEvent::Usage { usage } => reported = Some(usage),
            TurnEvent::Error { error } => failure = Some(error),
            _ => {}
        }
    }

    let free = is_free_endpoint(cfg);
    let mut entry =
        crate::core::tools::usage::UsageEntry::now("chat", &cfg.provider_id(), &cfg.model);
    if let Some(u) = &reported {
        entry.input_tokens = u.input_tokens as u64;
        entry.output_tokens = u.output_tokens as u64;
        entry.cache_read_tokens = u.cache_read_tokens as u64;
        entry.cache_write_tokens = u.cache_write_tokens as u64;
        entry.first_token_ms = u.first_token_ms;
        entry.cost_usd = if free { Some(0.0) } else { u.cost_usd };
    } else if free {
        entry.cost_usd = Some(0.0);
    }
    if entry.input_tokens > 0 || entry.output_tokens > 0 || !text.is_empty() {
        crate::core::tools::usage::record_entry(entry);
    }

    if let Some(e) = failure {
        if text.trim().is_empty() {
            return Err(e.to_string());
        }
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Empty AI response".to_string());
    }
    Ok(trimmed.to_string())
}

pub fn transcription_target(cfg: &AiConfig) -> Result<(String, String), String> {
    Ok(match cfg.provider {
        AiProvider::Openai => (
            "https://api.openai.com/v1/audio/transcriptions".to_string(),
            cfg.openai_key.clone(),
        ),
        AiProvider::Local => {
            if cfg.local_base_url.is_empty() {
                return Err("No local endpoint configured".to_string());
            }
            (
                format!("{}/audio/transcriptions", cfg.local_base_url),
                cfg.openai_key.clone(),
            )
        }
        _ => return Err("Transcription needs an OpenAI-compatible provider".to_string()),
    })
}

pub async fn transcribe(audio_path: &std::path::Path) -> Result<String, String> {
    let cfg = get();
    let (endpoint, key) = transcription_target(&cfg)?;

    let bytes = tokio::fs::read(audio_path)
        .await
        .map_err(|e| format!("Read audio failed: {}", e))?;
    let file_name = audio_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio.mp3".to_string());

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(file_name)
        .mime_str("application/octet-stream")
        .map_err(|e| format!("Multipart error: {}", e))?;
    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .text("response_format", "text")
        .part("file", part);

    let client = http_client()?;
    let mut req = client.post(&endpoint).multipart(form);
    if !key.is_empty() {
        req = req.bearer_auth(&key);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Read body failed: {}", e))?;
    if !status.is_success() {
        return Err(format!("Transcription error ({})", status.as_u16()));
    }
    let seconds = crate::core::ffmpeg::probe(audio_path)
        .await
        .map(|p| p.duration_seconds)
        .unwrap_or(0.0);
    crate::core::tools::usage::record_entry(crate::core::tools::usage::UsageEntry {
        characters: text.chars().count() as u64,
        seconds,
        cost_usd: if endpoint.starts_with("https://api.openai.com") {
            None
        } else {
            Some(0.0)
        },
        ..crate::core::tools::usage::UsageEntry::now("transcribe", &cfg.provider_id(), "whisper-1")
    });

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Empty transcription".to_string());
    }
    Ok(trimmed.to_string())
}

const AI_HISTORY_FILE: &str = "ai_history.json";
const MAX_HISTORY: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiHistoryEntry {
    pub id: u64,
    pub kind: String,
    pub url: String,
    pub title: String,
    pub content: String,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct AiHistoryFile {
    #[serde(default)]
    entries: Vec<AiHistoryEntry>,
}

fn history_path() -> Option<std::path::PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(AI_HISTORY_FILE))
}

pub fn history_list() -> Vec<AiHistoryEntry> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    match std::fs::read_to_string(&path) {
        Ok(c) => serde_json::from_str::<AiHistoryFile>(&c)
            .map(|f| f.entries)
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn history_add(kind: &str, url: &str, title: &str, content: &str) {
    let Some(path) = history_path() else { return };
    let Some(parent) = path.parent() else { return };
    let _ = std::fs::create_dir_all(parent);
    let mut entries = history_list();
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    entries.push(AiHistoryEntry {
        id,
        kind: kind.to_string(),
        url: url.to_string(),
        title: title.to_string(),
        content: content.to_string(),
        created_at_ms: id,
    });
    if entries.len() > MAX_HISTORY {
        let overflow = entries.len() - MAX_HISTORY;
        entries.drain(0..overflow);
    }
    if let Ok(s) = serde_json::to_string_pretty(&AiHistoryFile { entries }) {
        let tmp = path.with_extension("json.tmp");
        let ok = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(s.as_bytes())?;
            f.sync_all()?;
            Ok(())
        })()
        .is_ok();
        if ok {
            let _ = std::fs::rename(&tmp, &path);
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

pub fn history_clear() {
    if let Some(path) = history_path() {
        let _ = std::fs::remove_file(path);
    }
}
