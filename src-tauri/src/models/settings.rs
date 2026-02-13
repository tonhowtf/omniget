use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub schema_version: u32,
    pub appearance: AppearanceSettings,
    pub save: SaveSettings,
    pub hotmart: HotmartSettings,
    pub advanced: AdvancedSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    pub theme: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveSettings {
    pub default_output_dir: PathBuf,
    pub filename_template: String,
    pub overwrite_existing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotmartSettings {
    pub email: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedSettings {
    pub max_concurrent_downloads: u32,
    pub max_retries: u32,
    pub proxy: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            appearance: AppearanceSettings {
                theme: "system".into(),
                language: "pt-BR".into(),
            },
            save: SaveSettings {
                default_output_dir: dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")),
                filename_template: "{title}".into(),
                overwrite_existing: false,
            },
            hotmart: HotmartSettings {
                email: None,
                token: None,
            },
            advanced: AdvancedSettings {
                max_concurrent_downloads: 3,
                max_retries: 3,
                proxy: None,
            },
        }
    }
}
