use serde::{Deserialize, Serialize};

use crate::core::secrets;

const SETTINGS_FILE: &str = "settings.json";
const VEXA_KEY: &str = "meetings:vexa";
const TEAMS_WEBHOOK: &str = "meetings:teams-webhook";

pub const DEFAULT_VEXA_URL: &str = "https://api.cloud.vexa.ai";
pub const DEFAULT_WHISPER_MODEL: &str = "large-v3-turbo-q5_0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionBackend {
    #[default]
    Auto,
    Local,
    Api,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeetingSettings {
    pub transcription: TranscriptionBackend,
    pub whisper_model: String,
    pub language: String,
    pub auto_summarize: bool,
    pub summary_language: String,
    pub vexa_url: String,
    pub bot_name: String,
}

impl Default for MeetingSettings {
    fn default() -> Self {
        Self {
            transcription: TranscriptionBackend::Auto,
            whisper_model: DEFAULT_WHISPER_MODEL.to_string(),
            language: "auto".to_string(),
            auto_summarize: true,
            summary_language: String::new(),
            vexa_url: DEFAULT_VEXA_URL.to_string(),
            bot_name: "OmniGet Notetaker".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MeetingSettingsView {
    #[serde(flatten)]
    pub settings: MeetingSettings,
    pub vexa_key_set: bool,
    pub teams_webhook_set: bool,
}

pub fn load() -> MeetingSettings {
    super::root()
        .and_then(|r| std::fs::read_to_string(r.join(SETTINGS_FILE)).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(s: &MeetingSettings) -> anyhow::Result<()> {
    let root =
        super::root().ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))?;
    std::fs::create_dir_all(&root)?;
    let mut s = s.clone();
    s.vexa_url = s.vexa_url.trim().trim_end_matches('/').to_string();
    if s.vexa_url.is_empty() {
        s.vexa_url = DEFAULT_VEXA_URL.to_string();
    }
    if s.whisper_model.trim().is_empty() {
        s.whisper_model = DEFAULT_WHISPER_MODEL.to_string();
    }
    std::fs::write(root.join(SETTINGS_FILE), serde_json::to_vec_pretty(&s)?)?;
    Ok(())
}

pub fn view() -> MeetingSettingsView {
    MeetingSettingsView {
        settings: load(),
        vexa_key_set: vexa_key().is_some(),
        teams_webhook_set: teams_webhook().is_some(),
    }
}

fn secret(account: &str) -> Option<String> {
    secrets::get(secrets::AI_KEYS, account)
        .ok()
        .flatten()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn vexa_key() -> Option<String> {
    secret(VEXA_KEY)
}

pub fn teams_webhook() -> Option<String> {
    secret(TEAMS_WEBHOOK)
}

pub fn set_secrets(vexa_key: Option<&str>, teams_webhook: Option<&str>) -> Result<(), String> {
    if let Some(v) = vexa_key {
        secrets::put_or_delete(secrets::AI_KEYS, VEXA_KEY, v.trim())?;
    }
    if let Some(v) = teams_webhook {
        let v = v.trim();
        if !v.is_empty() && !v.starts_with("https://") {
            return Err("The Teams webhook has to be an https:// URL".into());
        }
        secrets::put_or_delete(secrets::AI_KEYS, TEAMS_WEBHOOK, v)?;
    }
    Ok(())
}
