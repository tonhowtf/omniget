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
    pub proxy: ProxySettings,
    #[serde(default)]
    pub onboarding_completed: bool,
    #[serde(default)]
    pub start_with_windows: bool,
    #[serde(default)]
    pub portable_mode: bool,
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
    #[serde(default)]
    pub extra_ytdlp_flags: Vec<String>,
    #[serde(default = "default_true")]
    pub copy_to_clipboard_on_hotkey: bool,
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
    "%(title).200s [%(id)s].%(ext)s".into()
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxySettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_proxy_type")]
    pub proxy_type: String,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_proxy_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

fn default_proxy_type() -> String {
    "http".into()
}

fn default_proxy_port() -> u16 {
    8080
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            appearance: AppearanceSettings {
                theme: "system".into(),
                language: "en".into(),
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
                extra_ytdlp_flags: Vec::new(),
                copy_to_clipboard_on_hotkey: true,
            },
            advanced: AdvancedSettings {
                max_concurrent_segments: 20,
                max_retries: 3,
                max_concurrent_downloads: 2,
                concurrent_fragments: 8,
                stagger_delay_ms: 150,
            },
            telegram: TelegramSettings::default(),
            proxy: ProxySettings::default(),
            onboarding_completed: false,
            start_with_windows: false,
            portable_mode: false,
        }
    }
}
