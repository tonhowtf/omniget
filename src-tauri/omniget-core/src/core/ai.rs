use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

const AI_CONFIG_FILE: &str = "ai_config.json";

const MINIMAX_MODEL_IDS: [&str; 2] = ["MiniMax-M3", "MiniMax-M2.7"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MinimaxRegion {
    #[default]
    GlobalEn,
    CnZh,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MinimaxApiFormat {
    #[default]
    Openai,
    Anthropic,
}

struct MinimaxEndpointConfig {
    openai_base_url: &'static str,
    anthropic_base_url: &'static str,
}

const MINIMAX_GLOBAL_ENDPOINTS: MinimaxEndpointConfig = MinimaxEndpointConfig {
    openai_base_url: "https://api.minimax.io/v1",
    anthropic_base_url: "https://api.minimax.io/anthropic",
};

const MINIMAX_CN_ENDPOINTS: MinimaxEndpointConfig = MinimaxEndpointConfig {
    openai_base_url: "https://api.minimaxi.com/v1",
    anthropic_base_url: "https://api.minimaxi.com/anthropic",
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    #[default]
    None,
    Openai,
    Anthropic,
    Minimax,
    Local,
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
    pub minimax_key: String,
    #[serde(default)]
    pub minimax_region: MinimaxRegion,
    #[serde(default)]
    pub minimax_api_format: MinimaxApiFormat,
    #[serde(default)]
    pub local_base_url: String,
    #[serde(default)]
    pub model: String,
}

// Sent to the frontend instead of AiConfig: key material is replaced with
// presence booleans so a raw key can never reach the UI, logs, or telemetry.
#[derive(Clone, Debug, Serialize)]
pub struct AiConfigView {
    pub provider: AiProvider,
    pub model: String,
    pub local_base_url: String,
    pub minimax_region: MinimaxRegion,
    pub minimax_api_format: MinimaxApiFormat,
    pub has_openai_key: bool,
    pub has_anthropic_key: bool,
    pub has_minimax_key: bool,
}

impl AiConfig {
    pub fn view(&self) -> AiConfigView {
        AiConfigView {
            provider: self.provider,
            model: self.model.clone(),
            local_base_url: self.local_base_url.clone(),
            minimax_region: self.minimax_region,
            minimax_api_format: self.minimax_api_format,
            has_openai_key: !self.openai_key.is_empty(),
            has_anthropic_key: !self.anthropic_key.is_empty(),
            has_minimax_key: !self.minimax_key.is_empty(),
        }
    }

    pub fn is_configured(&self) -> bool {
        match self.provider {
            AiProvider::None => false,
            AiProvider::Openai => !self.openai_key.is_empty(),
            AiProvider::Anthropic => !self.anthropic_key.is_empty(),
            AiProvider::Minimax => !self.minimax_key.is_empty(),
            AiProvider::Local => !self.local_base_url.is_empty(),
        }
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

pub fn model_ids(provider: AiProvider) -> Vec<String> {
    match provider {
        AiProvider::Minimax => MINIMAX_MODEL_IDS
            .iter()
            .map(|model| (*model).to_string())
            .collect(),
        _ => Vec::new(),
    }
}

fn minimax_endpoint_config(region: MinimaxRegion) -> &'static MinimaxEndpointConfig {
    match region {
        MinimaxRegion::GlobalEn => &MINIMAX_GLOBAL_ENDPOINTS,
        MinimaxRegion::CnZh => &MINIMAX_CN_ENDPOINTS,
    }
}

fn minimax_chat_endpoint(region: MinimaxRegion, api_format: MinimaxApiFormat) -> String {
    let endpoints = minimax_endpoint_config(region);
    match api_format {
        MinimaxApiFormat::Openai => format!("{}/chat/completions", endpoints.openai_base_url),
        MinimaxApiFormat::Anthropic => format!("{}/v1/messages", endpoints.anthropic_base_url),
    }
}

// Key fields are Option: None keeps the stored key untouched (so the UI never
// has to round-trip secrets), Some("") clears it.
pub fn set(
    provider: AiProvider,
    model: String,
    local_base_url: String,
    minimax_region: MinimaxRegion,
    minimax_api_format: MinimaxApiFormat,
    openai_key: Option<String>,
    anthropic_key: Option<String>,
    minimax_key: Option<String>,
) -> AiConfig {
    let mut guard = store().lock().unwrap();
    guard.provider = provider;
    guard.model = model.trim().to_string();
    guard.local_base_url = local_base_url.trim().trim_end_matches('/').to_string();
    guard.minimax_region = minimax_region;
    guard.minimax_api_format = minimax_api_format;
    if let Some(k) = openai_key {
        guard.openai_key = k.trim().to_string();
    }
    if let Some(k) = anthropic_key {
        guard.anthropic_key = k.trim().to_string();
    }
    if let Some(k) = minimax_key {
        guard.minimax_key = k.trim().to_string();
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
        AiProvider::Anthropic => {
            if cfg.anthropic_key.is_empty() {
                return Err("No Anthropic key configured".to_string());
            }
            anthropic_chat(
                "https://api.anthropic.com/v1/messages",
                &cfg.anthropic_key,
                &cfg.model,
                system,
                user,
            )
            .await
        }
        AiProvider::Minimax => {
            if cfg.minimax_key.is_empty() {
                return Err("No MiniMax key configured".to_string());
            }
            let endpoint = minimax_chat_endpoint(cfg.minimax_region, cfg.minimax_api_format);
            match cfg.minimax_api_format {
                MinimaxApiFormat::Openai => {
                    openai_chat(&endpoint, &cfg.minimax_key, &cfg.model, system, user).await
                }
                MinimaxApiFormat::Anthropic => {
                    anthropic_chat(&endpoint, &cfg.minimax_key, &cfg.model, system, user).await
                }
            }
        }
    }
}

async fn openai_chat(
    endpoint: &str,
    key: &str,
    model: &str,
    system: &str,
    user: &str,
) -> Result<String, String> {
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
    if !status.is_success() {
        return Err(format!("AI error ({})", status.as_u16()));
    }
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Bad JSON: {}", e))?;
    json.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "Empty AI response".to_string())
}

async fn anthropic_chat(
    endpoint: &str,
    key: &str,
    model: &str,
    system: &str,
    user: &str,
) -> Result<String, String> {
    let client = http_client()?;
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "system": system,
        "messages": [ { "role": "user", "content": user } ],
    });
    let resp = client
        .post(endpoint)
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
            minimax_key: "minimax-secret".to_string(),
            minimax_region: MinimaxRegion::GlobalEn,
            minimax_api_format: MinimaxApiFormat::Openai,
            local_base_url: String::new(),
            model: "m".to_string(),
        };
        let v = cfg.view();
        assert!(v.has_openai_key);
        assert!(!v.has_anthropic_key);
        assert!(v.has_minimax_key);
        let json = serde_json::to_string(&v).unwrap();
        assert!(!json.contains("secret"));
    }

    #[test]
    fn is_configured_logic() {
        let mut cfg = AiConfig::default();
        assert!(!cfg.is_configured());
        cfg.provider = AiProvider::Anthropic;
        assert!(!cfg.is_configured());
        cfg.anthropic_key = "k".to_string();
        assert!(cfg.is_configured());
        cfg.provider = AiProvider::Minimax;
        assert!(!cfg.is_configured());
        cfg.minimax_key = "k".to_string();
        assert!(cfg.is_configured());
    }

    #[test]
    fn minimax_models_are_declared() {
        assert_eq!(
            model_ids(AiProvider::Minimax),
            vec!["MiniMax-M3".to_string(), "MiniMax-M2.7".to_string()]
        );
    }

    #[test]
    fn minimax_regions_select_matching_endpoints() {
        assert_eq!(
            minimax_chat_endpoint(MinimaxRegion::GlobalEn, MinimaxApiFormat::Openai),
            "https://api.minimax.io/v1/chat/completions"
        );
        assert_eq!(
            minimax_chat_endpoint(MinimaxRegion::GlobalEn, MinimaxApiFormat::Anthropic),
            "https://api.minimax.io/anthropic/v1/messages"
        );
        assert_eq!(
            minimax_chat_endpoint(MinimaxRegion::CnZh, MinimaxApiFormat::Openai),
            "https://api.minimaxi.com/v1/chat/completions"
        );
        assert_eq!(
            minimax_chat_endpoint(MinimaxRegion::CnZh, MinimaxApiFormat::Anthropic),
            "https://api.minimaxi.com/anthropic/v1/messages"
        );
    }

    #[test]
    fn minimax_config_values_use_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&AiProvider::Minimax).unwrap(),
            "\"minimax\""
        );
        assert_eq!(
            serde_json::to_string(&MinimaxRegion::GlobalEn).unwrap(),
            "\"global_en\""
        );
        assert_eq!(
            serde_json::to_string(&MinimaxRegion::CnZh).unwrap(),
            "\"cn_zh\""
        );
    }
}
