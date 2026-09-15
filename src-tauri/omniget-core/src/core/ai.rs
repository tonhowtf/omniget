use crate::core::orcarouter::{self, invalidate_catalog_cache, AuthMethod, Credential};
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
    /// OrcaRouter with a key the user pasted from the console.
    Orcarouter,
    /// OrcaRouter with a key issued by the OAuth 2.0 + PKCE connect flow.
    ///
    /// Kept as its own variant, rather than folded into `Orcarouter`, so the
    /// two entrances stay explicitly separable in the provider picker, in
    /// status text and in logout — a single "OrcaRouter" button that sometimes
    /// asks for a key and sometimes opens a browser makes support,
    /// reauthentication and revocation much harder to reason about.
    ///
    /// Both variants share one inference adapter, one base URL, one model
    /// namespace and one catalog.
    #[serde(rename = "orcarouter-auth")]
    OrcarouterAuth,
}

impl AiProvider {
    /// Whether this variant routes to OrcaRouter, whichever entrance was used.
    pub fn is_orcarouter(self) -> bool {
        matches!(self, AiProvider::Orcarouter | AiProvider::OrcarouterAuth)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default)]
    pub provider: AiProvider,
    #[serde(default)]
    pub openai_key: String,
    #[serde(default)]
    pub anthropic_key: String,
    #[serde(default)]
    pub local_base_url: String,
    #[serde(default)]
    pub model: String,

    // -- OrcaRouter ------------------------------------------------------
    /// The `sk-orca-…` key. Written by whichever credential adapter ran; no
    /// request path knows or cares which one that was.
    #[serde(default)]
    pub orcarouter_key: String,
    /// `api_key` or `pkce`. Label only.
    #[serde(default)]
    pub orcarouter_method: String,
    /// OrcaRouter `user_id` from a PKCE exchange, when one was returned.
    #[serde(default)]
    pub orcarouter_account: String,
    /// The scope actually granted, read back from the exchange response.
    #[serde(default)]
    pub orcarouter_scope: String,
    /// Bumped on every successful credential save. A `401` only invalidates the
    /// generation that was actually rejected.
    #[serde(default)]
    pub orcarouter_generation: u64,
    /// Set when the relay rejected the current generation. Cleared by a new
    /// successful login.
    #[serde(default)]
    pub orcarouter_needs_reauth: bool,
}

// Sent to the frontend instead of AiConfig: key material is replaced with
// presence booleans so a raw key can never reach the UI, logs, or telemetry.
#[derive(Clone, Debug, Serialize)]
pub struct AiConfigView {
    pub provider: AiProvider,
    pub model: String,
    pub local_base_url: String,
    pub has_openai_key: bool,
    pub has_anthropic_key: bool,
    pub has_orcarouter_key: bool,
    /// Masked tail of the key, for a status line. Never the whole key.
    pub orcarouter_key_masked: String,
    pub orcarouter_method: String,
    pub orcarouter_account: String,
    pub orcarouter_scope: String,
    pub orcarouter_generation: u64,
    pub orcarouter_needs_reauth: bool,
    /// Resolved origins, so the UI never has to guess or rebuild them.
    pub orcarouter_auth_base: String,
    pub orcarouter_api_base: String,
}

impl AiConfig {
    pub fn view(&self) -> AiConfigView {
        let origins = orcarouter::origins();
        AiConfigView {
            provider: self.provider,
            model: self.model.clone(),
            local_base_url: self.local_base_url.clone(),
            has_openai_key: !self.openai_key.is_empty(),
            has_anthropic_key: !self.anthropic_key.is_empty(),
            has_orcarouter_key: !self.orcarouter_key.is_empty(),
            orcarouter_key_masked: mask_key(&self.orcarouter_key),
            orcarouter_method: self.orcarouter_method.clone(),
            orcarouter_account: self.orcarouter_account.clone(),
            orcarouter_scope: self.orcarouter_scope.clone(),
            orcarouter_generation: self.orcarouter_generation,
            orcarouter_needs_reauth: self.orcarouter_needs_reauth,
            orcarouter_auth_base: origins.auth,
            orcarouter_api_base: origins.api,
        }
    }

    pub fn is_configured(&self) -> bool {
        match self.provider {
            AiProvider::None => false,
            AiProvider::Openai => !self.openai_key.is_empty(),
            AiProvider::Anthropic => !self.anthropic_key.is_empty(),
            AiProvider::Local => !self.local_base_url.is_empty(),
            // A key that the relay has already rejected is not usable until a
            // new login succeeds; reporting "configured" would send the UI into
            // a failing request instead of a reconnect prompt.
            AiProvider::Orcarouter | AiProvider::OrcarouterAuth => {
                !self.orcarouter_key.is_empty() && !self.orcarouter_needs_reauth
            }
        }
    }
}

/// Last four characters behind eight dots. Never enough to use the key.
fn mask_key(key: &str) -> String {
    let n = key.chars().count();
    if n == 0 {
        return String::new();
    }
    if n <= 4 {
        return "•".repeat(n);
    }
    let tail: String = key.chars().skip(n - 4).collect();
    format!("{}{}", "•".repeat(8), tail)
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
    match std::fs::read_to_string(&path) {
        Ok(c) => serde_json::from_str(&c).unwrap_or_default(),
        Err(_) => AiConfig::default(),
    }
}

fn write_to_disk(cfg: &AiConfig) {
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
    orcarouter_key: Option<String>,
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
    if let Some(k) = orcarouter_key {
        let k = k.trim().to_string();
        if k.is_empty() {
            // An explicit clear also drops the account metadata, so a stale
            // workspace name cannot outlive the key it described.
            guard.orcarouter_key.clear();
            guard.orcarouter_method.clear();
            guard.orcarouter_account.clear();
            guard.orcarouter_scope.clear();
            guard.orcarouter_needs_reauth = false;
        } else {
            guard.orcarouter_key = k;
            guard.orcarouter_method = AuthMethod::ApiKey.as_str().to_string();
            guard.orcarouter_account.clear();
            guard.orcarouter_scope.clear();
            guard.orcarouter_needs_reauth = false;
        }
    }
    write_to_disk(&guard);
    invalidate_catalog_cache();
    guard.clone()
}

/// Persist a credential produced by either authentication adapter.
///
/// This is the single write point for OrcaRouter credentials. Both the
/// pasted-key adapter and the PKCE adapter end up here with the same
/// [`Credential`], so neither the provider request path nor model discovery can
/// tell them apart.
///
/// Bumping the generation is what makes the `401` path generation-safe: a
/// rejection that belonged to an older key can never mark this new one broken.
pub fn save_orcarouter_credential(c: &Credential) -> AiConfig {
    let mut guard = store().lock().unwrap();
    guard.orcarouter_key = c.key().to_string();
    guard.orcarouter_method = c.method.as_str().to_string();
    guard.orcarouter_account = c.account.clone().unwrap_or_default();
    guard.orcarouter_scope = c.scope.clone().unwrap_or_default();
    guard.orcarouter_generation = guard.orcarouter_generation.wrapping_add(1);
    guard.orcarouter_needs_reauth = false;
    write_to_disk(&guard);
    // A different key means a different workspace, so the cached catalog is no
    // longer authoritative.
    invalidate_catalog_cache();
    guard.clone()
}

/// Drop the OrcaRouter credential without touching the other providers.
///
/// Deliberately *not* called on a `401`: silently deleting a key before a
/// replacement exists turns a transient failure into irreversible account loss.
pub fn clear_orcarouter_credential() -> AiConfig {
    let mut guard = store().lock().unwrap();
    guard.orcarouter_key.clear();
    guard.orcarouter_method.clear();
    guard.orcarouter_account.clear();
    guard.orcarouter_scope.clear();
    guard.orcarouter_needs_reauth = false;
    write_to_disk(&guard);
    invalidate_catalog_cache();
    guard.clone()
}

/// The credential generation currently in force, for a request to quote back.
pub fn orcarouter_generation() -> u64 {
    store().lock().unwrap().orcarouter_generation
}

/// How the stored OrcaRouter credential was obtained (`api_key` or `pkce`).
/// Diagnostic only — no request path branches on it.
pub fn orcarouter_method() -> String {
    store().lock().unwrap().orcarouter_method.clone()
}

/// Mark the exact rejected generation as needing reauthentication.
///
/// `generation` is the value the failing request read *before* it was sent. If
/// the stored generation has moved on — the user already reconnected — this is
/// a late failure from a superseded request and must do nothing.
///
/// Returns true when the state actually changed.
pub fn mark_orcarouter_unauthorized(generation: u64) -> bool {
    let mut guard = store().lock().unwrap();
    if guard.orcarouter_key.is_empty() || generation != guard.orcarouter_generation {
        return false;
    }
    if guard.orcarouter_needs_reauth {
        return false;
    }
    guard.orcarouter_needs_reauth = true;
    write_to_disk(&guard);
    true
}

/// Whether the stored OrcaRouter credential needs reauthentication.
pub fn orcarouter_needs_reauth() -> bool {
    store().lock().unwrap().orcarouter_needs_reauth
}

fn http_client() -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120));
    crate::core::http_client::apply_global_proxy(builder)
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))
}

pub async fn chat(system: &str, user: &str) -> Result<String, String> {
    let cfg = get();
    if cfg.model.is_empty() {
        return Err("No AI model configured".to_string());
    }
    match cfg.provider {
        AiProvider::None => Err("AI is not configured".to_string()),
        AiProvider::Openai => {
            openai_chat(
                "https://api.openai.com/v1/chat/completions",
                &cfg.openai_key,
                &cfg.model,
                system,
                user,
            )
            .await
        }
        AiProvider::Local => {
            if cfg.local_base_url.is_empty() {
                return Err("No local endpoint configured".to_string());
            }
            let endpoint = format!("{}/chat/completions", cfg.local_base_url);
            openai_chat(&endpoint, &cfg.openai_key, &cfg.model, system, user).await
        }
        AiProvider::Anthropic => anthropic_chat(&cfg.anthropic_key, &cfg.model, system, user).await,
        // Both OrcaRouter entrances share one adapter, one base URL and one
        // model namespace. The key came from one of two adapters; nothing here
        // branches on which.
        AiProvider::Orcarouter | AiProvider::OrcarouterAuth => {
            orcarouter_chat(&cfg, system, user).await
        }
    }
}

/// Text generation through the OrcaRouter relay at `{api}/chat/completions`,
/// which is OpenAI-compatible.
async fn orcarouter_chat(cfg: &AiConfig, system: &str, user: &str) -> Result<String, String> {
    if cfg.orcarouter_key.is_empty() {
        return Err("ai_not_configured".to_string());
    }
    // A key the relay already rejected must produce a reconnect prompt, not a
    // request that is guaranteed to fail.
    if cfg.orcarouter_needs_reauth {
        return Err(orcarouter::NEEDS_REAUTH.to_string());
    }
    let origins = orcarouter::origins();
    orcarouter::validate_origins(&origins)?;
    let endpoint = origins.api_url("/chat/completions");
    openai_chat(&endpoint, &cfg.orcarouter_key, &cfg.model, system, user).await
}

async fn openai_chat(
    endpoint: &str,
    key: &str,
    model: &str,
    system: &str,
    user: &str,
) -> Result<String, String> {
    // Read the credential generation *before* the request goes out. A 401 is
    // then attributed to exactly this generation, so a late failure can never
    // mark a credential the user has since replaced.
    let generation = if endpoint.contains("orcarouter.ai") {
        orcarouter_generation()
    } else {
        0
    };
    let client = http_client()?;
    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    });
    let mut req = client.post(endpoint).json(&body);
    if !key.is_empty() {
        req = req.bearer_auth(key);
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
    if status.as_u16() == 401 && endpoint.contains("orcarouter.ai") {
        // Terminal reauthentication, not a retry loop: mark the exact
        // generation that made this request. If the user has already
        // reconnected, the generation has moved on and this is a no-op.
        mark_orcarouter_unauthorized(generation);
        return Err(orcarouter::NEEDS_REAUTH.to_string());
    }
    if endpoint.contains("orcarouter.ai") && !status.is_success() {
        // A 403 model_access_denied is a model-permission problem, not a
        // credential one; saying so keeps the user from reconnecting a key that
        // works. Nothing is marked needs_reauth here.
        if let Some(msg) = orcarouter::classify_relay_error(status.as_u16(), &text) {
            if msg != orcarouter::NEEDS_REAUTH {
                return Err(msg);
            }
        }
    }
    if !status.is_success() {
        return Err(format!("AI error ({})", status.as_u16()));
    }
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Bad JSON: {}", e))?;
    // Ledger de custo (Tools → Custos de IA): tokens que o provedor informou.
    let usage = json.get("usage");
    let input = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let provider = if endpoint.contains("orcarouter.ai") {
        "orcarouter"
    } else if endpoint.starts_with("https://api.openai.com") {
        "openai"
    } else {
        "local"
    };
    let cost = if provider == "local" { Some(0.0) } else { None };
    crate::core::tools::usage::record("chat", provider, model, input, output, cost);
    json.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "Empty AI response".to_string())
}

async fn anthropic_chat(
    key: &str,
    model: &str,
    system: &str,
    user: &str,
) -> Result<String, String> {
    if key.is_empty() {
        return Err("No Anthropic key configured".to_string());
    }
    let client = http_client()?;
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "system": system,
        "messages": [ { "role": "user", "content": user } ],
    });
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Read body failed: {}", e))?;
    if !status.is_success() {
        return Err(format!("AI error ({})", status.as_u16()));
    }
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Bad JSON: {}", e))?;
    let usage = json.get("usage");
    let input = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    crate::core::tools::usage::record("chat", "anthropic", model, input, output, None);
    json.get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "Empty AI response".to_string())
}

// Whisper-style transcription via the OpenAI-compatible audio endpoint. Only
// available for Openai/Local providers (Anthropic has no audio API). Reuses
// the configured key/base so no separate model management is needed.
//
// OrcaRouter is deliberately NOT wired in here: its catalog advertises no
// audio/transcription endpoint type, so routing this at the relay would send a
// request to an endpoint the provider does not publish. The Openai/Local
// behaviour is left exactly as it was.
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
            anthropic_key: String::new(),
            local_base_url: String::new(),
            model: "m".to_string(),
            ..Default::default()
        };
        let v = cfg.view();
        assert!(v.has_openai_key);
        assert!(!v.has_anthropic_key);
        let json = serde_json::to_string(&v).unwrap();
        assert!(!json.contains("secret"));
    }

    #[test]
    fn view_hides_the_orcarouter_key_and_only_exposes_a_masked_tail() {
        let cfg = AiConfig {
            provider: AiProvider::Orcarouter,
            orcarouter_key: "sk-orca-FAKEtestfixtureonly0001".to_string(),
            model: "orcarouter/auto".to_string(),
            ..Default::default()
        };
        let v = cfg.view();
        assert!(v.has_orcarouter_key);
        assert_eq!(v.orcarouter_key_masked, "••••••••0001");
        let json = serde_json::to_string(&v).unwrap();
        assert!(!json.contains("FAKEtestfixtureonly"));
        // The origins the UI needs are exposed; no secret rides along.
        assert!(json.contains("www.orcarouter.ai"));
        assert!(json.contains("api.orcarouter.ai/v1"));
    }

    #[test]
    fn a_rejected_credential_is_not_reported_as_configured() {
        let mut cfg = AiConfig {
            provider: AiProvider::OrcarouterAuth,
            orcarouter_key: "sk-orca-abcdefghijklmnop".to_string(),
            model: "orcarouter/auto".to_string(),
            ..Default::default()
        };
        assert!(cfg.is_configured());
        cfg.orcarouter_needs_reauth = true;
        assert!(!cfg.is_configured());
        // The other providers are unaffected by OrcaRouter state.
        cfg.provider = AiProvider::Openai;
        cfg.openai_key = "sk-x".to_string();
        assert!(cfg.is_configured());
    }

    #[test]
    fn both_orcarouter_entrances_are_recognised_as_orcarouter() {
        assert!(AiProvider::Orcarouter.is_orcarouter());
        assert!(AiProvider::OrcarouterAuth.is_orcarouter());
        assert!(!AiProvider::Openai.is_orcarouter());
        assert!(!AiProvider::None.is_orcarouter());
    }

    #[test]
    fn provider_ids_round_trip_through_serde() {
        // The API-key entrance keeps the plain id; the account entrance is
        // distinguishable in the persisted config.
        assert_eq!(
            serde_json::to_string(&AiProvider::Orcarouter).unwrap(),
            "\"orcarouter\""
        );
        assert_eq!(
            serde_json::to_string(&AiProvider::OrcarouterAuth).unwrap(),
            "\"orcarouter-auth\""
        );
        let parsed: AiProvider = serde_json::from_str("\"orcarouter-auth\"").unwrap();
        assert_eq!(parsed, AiProvider::OrcarouterAuth);
        // A config written before this change still loads.
        let old: AiConfig =
            serde_json::from_str(r#"{"provider":"openai","model":"gpt-4"}"#).unwrap();
        assert_eq!(old.provider, AiProvider::Openai);
        assert_eq!(old.orcarouter_generation, 0);
        assert!(!old.orcarouter_needs_reauth);
    }

    #[test]
    fn is_configured_logic() {
        let mut cfg = AiConfig::default();
        assert!(!cfg.is_configured());
        cfg.provider = AiProvider::Anthropic;
        assert!(!cfg.is_configured());
        cfg.anthropic_key = "k".to_string();
        assert!(cfg.is_configured());
        // An OrcaRouter provider with no key is not configured, on either
        // entrance.
        cfg.provider = AiProvider::Orcarouter;
        cfg.anthropic_key.clear();
        assert!(!cfg.is_configured());
        cfg.provider = AiProvider::OrcarouterAuth;
        assert!(!cfg.is_configured());
    }
}
