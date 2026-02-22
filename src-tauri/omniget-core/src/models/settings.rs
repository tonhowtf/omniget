use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub schema_version: u32,
    pub appearance: AppearanceSettings,
    pub download: DownloadSettings,
    pub advanced: AdvancedSettings,
    #[serde(default)]
    pub telegram: TelegramSettings,
    #[serde(default)]
    pub onboarding_completed: bool,
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
    #[serde(default = "default_true")]
    pub embed_metadata: bool,
    #[serde(default = "default_true")]
    pub embed_thumbnail: bool,
    #[serde(default)]
    pub clipboard_detection: bool,
    #[serde(default = "default_filename_template")]
    pub filename_template: String,
    #[serde(default)]
    pub organize_by_platform: bool,
    #[serde(default)]
    pub download_subtitles: bool,
    #[serde(default)]
    pub hotkey_enabled: bool,
    #[serde(default = "default_hotkey_binding")]
    pub hotkey_binding: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedSettings {
    pub max_concurrent_segments: u32,
    pub max_retries: u32,
    #[serde(default = "default_max_concurrent_downloads")]
    pub max_concurrent_downloads: u32,
    #[serde(default = "default_concurrent_fragments")]
    pub concurrent_fragments: u32,
    #[serde(default = "default_stagger_delay_ms")]
    pub stagger_delay_ms: u64,
}

fn default_concurrent_fragments() -> u32 {
    8
}

fn default_max_concurrent_downloads() -> u32 {
    2
}

fn default_stagger_delay_ms() -> u64 {
    150
}

fn default_true() -> bool {
    true
}

pub fn default_filename_template() -> String {
    "omniget-%(title).200s [%(id)s].%(ext)s".into()
}

fn default_hotkey_binding() -> String {
    "CmdOrCtrl+Shift+D".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramSettings {
    pub concurrent_downloads: u32,
    pub fix_file_extensions: bool,
}

impl Default for TelegramSettings {
    fn default() -> Self {
        Self {
            concurrent_downloads: 3,
            fix_file_extensions: true,
        }
    }
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
                embed_metadata: true,
                embed_thumbnail: true,
                clipboard_detection: false,
                filename_template: default_filename_template(),
                organize_by_platform: false,
                download_subtitles: false,
                hotkey_enabled: false,
                hotkey_binding: default_hotkey_binding(),
            },
            advanced: AdvancedSettings {
                max_concurrent_segments: 20,
                max_retries: 3,
                max_concurrent_downloads: 2,
                concurrent_fragments: 8,
                stagger_delay_ms: 150,
            },
            telegram: TelegramSettings::default(),
            onboarding_completed: false,
        }
    }
}
