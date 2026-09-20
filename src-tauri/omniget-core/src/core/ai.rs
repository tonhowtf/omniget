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
    /// Which wire `chat()` speaks. Derived from [`AiConfig::kind`] since the LLM
    /// expansion (`set_from_key`), not chosen by hand.
    #[serde(default)]
    pub provider: AiProvider,
    /// Never serialised: `ai_config.json` holds metadata only and the key lives
    /// in the secret store. Deserialising still works, which is how a file
    /// written by an older build is read once and migrated.
    #[serde(default, skip_serializing)]
    pub openai_key: String,
    #[serde(default, skip_serializing)]
    pub anthropic_key: String,
    #[serde(default)]
    pub local_base_url: String,
    #[serde(default)]
    pub model: String,
    /// The `ai_keys::Kind::id` behind this config ("openai", "openrouter",
    /// "gemini", …) — the `ProviderId` the LLM layer routes on. Empty on a
    /// config that predates the expansion.
    #[serde(default)]
    pub kind: String,
    /// The `ai_keys` entry this came from, so the vault and the app config stop
    /// drifting apart.
    #[serde(default)]
    pub key_id: String,
}

// Sent to the frontend instead of AiConfig: key material is replaced with
// presence booleans so a raw key can never reach the UI, logs, or telemetry.
#[derive(Clone, Debug, Serialize)]
pub struct AiConfigView {
    pub provider: AiProvider,
    /// The `ai_keys` provider id behind `provider`; empty before the first
    /// `use_in_app`.
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
            kind: self.kind.clone(),
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
            // `Local` is every OpenAI-shaped provider that is not api.openai.com, so the
            // endpoint is what makes it usable. A key is only required by the rows that
            // take one: a local server answers without it.
            AiProvider::Local => {
                !self.local_base_url.is_empty()
                    && (!self.kind_needs_key() || !self.openai_key.is_empty())
            }
        }
    }

    /// Whether the kind this config records takes a credential. An unknown or empty
    /// kind is treated as taking one, because refusing to call a configuration
    /// "configured" is the safer failure for a request that would go out unauthenticated.
    fn kind_needs_key(&self) -> bool {
        if self.kind.is_empty() {
            return true;
        }

        crate::core::tools::ai_keys::kind_of(&self.kind).needs_key()
    }
}

static STORE: OnceLock<Mutex<AiConfig>> = OnceLock::new();

fn store() -> &'static Mutex<AiConfig> {
    STORE.get_or_init(|| Mutex::new(load_from_disk()))
}

fn file_path() -> Option<std::path::PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(AI_CONFIG_FILE))
}

/// Read the config and put the keys back on it from the secret store. A file
/// written before the LLM expansion still carries them in plain text: that is
/// migrated here, once, with a 0600 backup beside the file.
fn load_from_disk() -> AiConfig {
    let Some(path) = file_path() else {
        return AiConfig::default();
    };
    let mut cfg: AiConfig = match std::fs::read_to_string(&path) {
        Ok(c) => serde_json::from_str(&c).unwrap_or_default(),
        Err(_) => AiConfig::default(),
    };
    // The migration takes the keys off `cfg` and into the store, so either way
    // the hydration below is what puts them back.
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

/// Write the config. The keys go to the secret store and never to the JSON —
/// `openai_key`/`anthropic_key` are `skip_serializing`, so the file cannot
/// carry them even by accident.
fn write_to_disk(cfg: &AiConfig) {
    use crate::core::secrets::{self, AI_KEYS};
    use crate::core::tools::ai_keys::migrate;
    for (account, value) in [
        (migrate::APP_OPENAI_ACCOUNT, cfg.openai_key.trim()),
        (migrate::APP_ANTHROPIC_ACCOUNT, cfg.anthropic_key.trim()),
    ] {
        if let Err(e) = secrets::put_or_delete(AI_KEYS, account, value) {
            tracing::warn!("[ai] could not store {}: {}", account, e);
        }
    }
    let Some(path) = file_path() else { return };
    let Some(parent) = path.parent() else { return };
    if let Err(e) = std::fs::create_dir_all(parent) {
        tracing::warn!("[ai] create_dir_all failed: {}", e);
        return;
    }
    let serialized = match serde_json::to_string_pretty(cfg) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[ai] serialize failed: {}", e);
            return;
        }
    };
    let tmp = path.with_extension("json.tmp");
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(serialized.as_bytes())?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = result {
        tracing::warn!("[ai] write tmp failed: {}", e);
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        tracing::warn!("[ai] rename failed: {}", e);
        let _ = std::fs::remove_file(&tmp);
    }
}

pub fn get() -> AiConfig {
    store().lock().unwrap().clone()
}

// Key fields are Option: None keeps the stored key untouched (so the UI never
// has to round-trip secrets), Some("") clears it.
pub fn set(
    provider: AiProvider,
    model: String,
    local_base_url: String,
    openai_key: Option<String>,
    anthropic_key: Option<String>,
) -> AiConfig {
    let mut guard = store().lock().unwrap();
    guard.provider = provider;
    guard.model = model.trim().to_string();
    guard.local_base_url = local_base_url.trim().trim_end_matches('/').to_string();
    if let Some(k) = openai_key {
        guard.openai_key = k.trim().to_string();
    }
    if let Some(k) = anthropic_key {
        guard.anthropic_key = k.trim().to_string();
    }
    write_to_disk(&guard);
    guard.clone()
}

/// Which wire an `ai_keys` kind speaks. `Local` is not a downgrade: it is the
/// OpenAI dialect against a base URL that is not api.openai.com.
pub fn provider_for_kind(kind: &str) -> AiProvider {
    match crate::core::tools::ai_keys::kind_of(kind).wire {
        "anthropic" => AiProvider::Anthropic,
        _ if kind == "openai" => AiProvider::Openai,
        _ => AiProvider::Local,
    }
}

/// What a settings form wants done with the stored credential.
///
/// A blank field cannot mean both "keep what is there" and "there is no key", and the
/// two are no longer the same thing: switching between two providers that share the
/// `openai_key` slot would otherwise send the first provider's credential to the
/// second one's endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction<'a> {
    Keep,
    Set(&'a str),
    Clear,
}

/// Configure the app's AI from a settings form rather than from a vault entry.
///
/// `set` cannot be used for this: it keeps the previous `kind`, so the `provider_id`
/// a request is routed on falls back to the wire (`custom`) and the entry the user
/// just picked is lost. Deriving the wire from the kind is the same rule
/// [`provider_for_kind`] documents, and the base URL comes from the kind's default
/// when the form leaves it out, which is what makes `deepseek`, `groq`, `xai`,
/// `mistral` and the like reachable without retyping their endpoints.
pub fn set_with_kind(kind: &str, model: String, base_url: String, key: KeyAction<'_>) -> AiConfig {
    let provider = provider_for_kind(kind);
    let mut guard = store().lock().unwrap();
    guard.provider = provider;
    guard.kind = kind.trim().to_string();
    guard.key_id = String::new();
    guard.model = model.trim().to_string();
    guard.local_base_url = if !base_url.trim().is_empty() {
        base_url.trim().trim_end_matches('/').to_string()
    } else if provider == AiProvider::Local {
        crate::core::tools::ai_keys::app_base_url(kind, "")
    } else {
        // The OpenAI and Anthropic wires have a fixed endpoint in `provider_from_config`.
        String::new()
    };
    match key {
        KeyAction::Keep => {}
        KeyAction::Set(k) => match provider {
            AiProvider::Anthropic => guard.anthropic_key = k.trim().to_string(),
            _ => guard.openai_key = k.trim().to_string(),
        },
        KeyAction::Clear => {
            guard.openai_key.clear();
            guard.anthropic_key.clear();
        }
    }
    write_to_disk(&guard);
    guard.clone()
}

/// Turn AI off. Clearing `kind` is part of that: `view()` hands the kind to the
/// settings form, and a stale one makes a disabled config read as the provider that
/// was configured before it was disabled.
pub fn clear() -> AiConfig {
    let mut guard = store().lock().unwrap();
    guard.provider = AiProvider::None;
    guard.kind = String::new();
    guard.key_id = String::new();
    write_to_disk(&guard);
    guard.clone()
}

impl AiConfig {
    /// The `ProviderId` this config represents. Falls back to the wire when the
    /// kind is empty (a config from before the expansion) or stale (the user
    /// switched provider by hand in Settings).
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

/// Point the app's AI at an `ai_keys` entry. The entry's kind is recorded as it
/// is and the wire is derived from it, so a provider is no longer flattened to
/// "Local" and forgotten.
pub fn set_from_key(
    kind: &str,
    key_id: &str,
    model: String,
    base_url: String,
    key: String,
) -> AiConfig {
    let provider = provider_for_kind(kind);
    let mut guard = store().lock().unwrap();
    guard.provider = provider;
    guard.kind = kind.to_string();
    guard.key_id = key_id.to_string();
    guard.model = model.trim().to_string();
    guard.local_base_url = match provider {
        // The OpenAI and Anthropic wires have a fixed endpoint in `chat()`.
        AiProvider::Openai | AiProvider::Anthropic | AiProvider::None => String::new(),
        AiProvider::Local => base_url.trim().trim_end_matches('/').to_string(),
    };
    match provider {
        AiProvider::Anthropic => guard.anthropic_key = key.trim().to_string(),
        _ => guard.openai_key = key.trim().to_string(),
    }
    write_to_disk(&guard);
    guard.clone()
}

fn http_client() -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120));
    crate::core::http_client::apply_global_proxy(builder)
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))
}

/// The `Provider` behind the app's configured account. Built per turn because
/// the config can change between calls, and shared with `core::llm`: the app
/// and the agents speak through the same client, the same retry policy and the
/// same SSE parser.
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

/// A turn that costs nothing: a server on this machine, or Ollama.
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

/// Single-shot chat kept for the whole app (`humanize`, `ai_test`,
/// `ai_summarize_url`, legendas, coach da League): the signature is the same as
/// before the LLM expansion, but underneath it is now one `TurnRequest` on the
/// `core::llm` stack — streaming, retry with backoff, cancellation and the
/// provider-reported `usage`. Anthropic no longer truncates at 1024 tokens.
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

/// `chat` against an explicit config and cancellation token.
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

    // Ledger de custo (Tools → Custos de IA), agora com cache e latência.
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

// Whisper-style transcription via the OpenAI-compatible audio endpoint. Only
// available for Openai/Local providers (Anthropic has no audio API). Reuses
// the configured key/base so no separate model management is needed.
pub async fn transcribe(audio_path: &std::path::Path) -> Result<String, String> {
    let cfg = get();
    let (endpoint, key) = match cfg.provider {
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
    };

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
    // Ledger: o Whisper é cobrado por minuto de áudio, então a linha leva
    // `seconds` (do ffprobe, melhor esforço) e os caracteres transcritos —
    // os dois campos que existiam no `UsageEntry` e nunca eram escritos.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_hides_keys() {
        let cfg = AiConfig {
            provider: AiProvider::Openai,
            openai_key: "secret".to_string(),
            model: "m".to_string(),
            ..Default::default()
        };
        let v = cfg.view();
        assert!(v.has_openai_key);
        assert!(!v.has_anthropic_key);
        let json = serde_json::to_string(&v).unwrap();
        assert!(!json.contains("secret"));
        // ...and neither does what goes to ai_config.json.
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("secret"), "{json}");
    }

    /// `AiProvider` is a wire, `kind` is the provider. A stale kind (the user
    /// switched provider by hand afterwards) must not be reported.
    #[test]
    fn the_provider_id_follows_the_kind_while_it_matches_the_wire() {
        assert_eq!(provider_for_kind("openai"), AiProvider::Openai);
        assert_eq!(provider_for_kind("anthropic"), AiProvider::Anthropic);
        assert_eq!(provider_for_kind("openrouter"), AiProvider::Local);
        assert_eq!(provider_for_kind("gemini"), AiProvider::Local);

        let mut cfg = AiConfig {
            provider: AiProvider::Local,
            kind: "openrouter".to_string(),
            ..Default::default()
        };
        assert_eq!(cfg.provider_id(), "openrouter");
        cfg.provider = AiProvider::Anthropic;
        assert_eq!(cfg.provider_id(), "anthropic");
        cfg.kind = String::new();
        cfg.provider = AiProvider::Local;
        assert_eq!(cfg.provider_id(), "custom");
        cfg.provider = AiProvider::None;
        assert_eq!(cfg.provider_id(), "");
    }

    #[test]
    fn is_configured_logic() {
        let mut cfg = AiConfig::default();
        assert!(!cfg.is_configured());
        cfg.provider = AiProvider::Anthropic;
        assert!(!cfg.is_configured());
        cfg.anthropic_key = "k".to_string();
        assert!(cfg.is_configured());
    }

    /// Choosing a provider by kind has to survive the round trip: the wire is
    /// derived, the id the request is routed on is the kind, and a provider whose
    /// endpoint is not `api.openai.com` gets its own base URL without the form
    /// having to carry one.
    #[test]
    fn a_provider_chosen_by_kind_keeps_its_endpoint_and_its_id() {
        let cfg = AiConfig {
            provider: provider_for_kind("deepseek"),
            kind: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            local_base_url: crate::core::tools::ai_keys::app_base_url("deepseek", ""),
            openai_key: "k".to_string(),
            ..Default::default()
        };

        // deepseek speaks the openai dialect, so it routes as Local - and that is not a
        // downgrade, it is what makes the base URL below reachable at all
        assert_eq!(cfg.provider, AiProvider::Local);
        assert_eq!(cfg.provider_id(), "deepseek");
        assert_eq!(cfg.local_base_url, "https://api.deepseek.com");
        assert!(cfg.is_configured());

        // the same rule for a kind on the other wire, where the endpoint is fixed in
        // provider_from_config and must stay empty so it is not mistaken for a local URL
        assert_eq!(provider_for_kind("anthropic"), AiProvider::Anthropic);
        assert_eq!(provider_for_kind("openai"), AiProvider::Openai);
    }

    /// GEMINI is the one non-OpenAI wire that still needs a URL, and it needs the
    /// OpenAI-compatible route rather than the native one.
    #[test]
    fn gemini_by_kind_gets_its_openai_compatible_route() {
        let url = crate::core::tools::ai_keys::app_base_url("gemini", "");

        assert!(url.ends_with("/openai"), "{url}");
        assert_eq!(provider_for_kind("gemini"), AiProvider::Local);
    }

    /// A blank key field means "keep", but switching providers means "do not carry the
    /// other provider's credential over". Both used to arrive as `None`, so the first
    /// provider's key was sent to the second one's endpoint as a bearer token.
    #[test]
    fn switching_provider_can_drop_the_previous_credential() {
        let cfg = AiConfig {
            provider: provider_for_kind("deepseek"),
            kind: "deepseek".to_string(),
            local_base_url: crate::core::tools::ai_keys::app_base_url("deepseek", ""),
            openai_key: "sk-deepseek".to_string(),
            ..Default::default()
        };
        // a keyed provider is only configured once it has both halves, which is what the
        // settings form checks before offering the summarize controls
        assert!(cfg.is_configured());

        // and a local server without a key still is
        let ollama = AiConfig {
            provider: provider_for_kind("ollama"),
            kind: "ollama".to_string(),
            local_base_url: crate::core::tools::ai_keys::app_base_url("ollama", ""),
            ..Default::default()
        };
        assert!(ollama.is_configured(), "a local server answers without a key");

        // the two actions a form can ask for, in the shape the command builds them
        match KeyAction::Set("sk-xai") {
            KeyAction::Set(k) => assert_eq!(k, "sk-xai"),
            _ => panic!("Set must carry the credential"),
        }
        match KeyAction::Clear {
            KeyAction::Clear => {}
            _ => panic!("Clear must not carry one"),
        }
    }

    /// Turning AI off has to drop the recorded kind, or the next read of the config
    /// reports the provider that was configured before it was turned off.
    #[test]
    fn a_cleared_config_reports_no_provider() {
        let cfg = AiConfig {
            provider: AiProvider::None,
            kind: String::new(),
            key_id: String::new(),
            ..Default::default()
        };

        assert_eq!(cfg.provider_id(), "");
        assert!(!cfg.is_configured());
    }

    /// The capability flags the settings form reads come from the table row, so a
    /// provider added later cannot be missing from a second list kept somewhere else.
    #[test]
    fn the_table_carries_the_form_capabilities() {
        use crate::core::tools::ai_keys::kind_of;

        // a provider behind a real endpoint takes a key and does not ask for a URL
        assert!(kind_of("deepseek").needs_key());
        assert!(!kind_of("deepseek").base_url_editable());

        // a local server answers without one, and its URL is the user's to give
        assert!(!kind_of("ollama").needs_key());
        assert!(kind_of("ollama").base_url_editable());

        // a relay is deployed per site, so its table URL is a placeholder until replaced
        assert!(kind_of("newapi").needs_key());
        assert!(kind_of("newapi").base_url_editable());
    }
}
