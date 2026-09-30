//! Meeting preferences (`<meetings>/settings.json`) and the two secrets they
//! need: the Vexa API key and the Teams incoming-webhook URL. Secrets live in
//! the same vault as the AI provider keys, under `meetings:*` accounts.

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
    /// Local whisper.cpp when it and a model are installed, else the AI
    /// provider's audio endpoint.
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
    /// "auto" or an ISO code.
    pub language: String,
    /// Summarize as soon as the transcript is ready.
    pub auto_summarize: bool,
    /// Language the summary is written in; empty = the meeting's language.
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

/// What the UI sees: settings plus whether each secret is set, never the value.
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

/// `None` leaves the secret alone, `Some("")` clears it.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_roundtrip() {
        let _root = super::super::test_support::temp_root();
        assert_eq!(load(), MeetingSettings::default());
        let s = MeetingSettings {
            vexa_url: " http://localhost:18056/ ".into(),
            whisper_model: "".into(),
            ..Default::default()
        };
        save(&s).unwrap();
        let back = load();
        assert_eq!(back.vexa_url, "http://localhost:18056");
        assert_eq!(back.whisper_model, DEFAULT_WHISPER_MODEL);
    }
}
