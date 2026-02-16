use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub schema_version: u32,
    pub appearance: AppearanceSettings,
    pub download: DownloadSettings,
    pub advanced: AdvancedSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    pub theme: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadSettings {
    pub default_output_dir: PathBuf,
    pub always_ask_path: bool,
    pub video_quality: String,
    pub skip_existing: bool,
    pub download_attachments: bool,
    pub download_descriptions: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedSettings {
    pub max_concurrent_segments: u32,
    pub max_retries: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            appearance: AppearanceSettings {
                theme: "system".into(),
                language: "pt".into(),
            },
            download: DownloadSettings {
                default_output_dir: dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")),
                always_ask_path: true,
                video_quality: "720p".into(),
                skip_existing: true,
                download_attachments: true,
                download_descriptions: true,
            },
            advanced: AdvancedSettings {
                max_concurrent_segments: 20,
                max_retries: 3,
            },
        }
    }
}
