use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

use omniget_core::core::dependencies::find_tool;
use omniget_core::core::paths::app_data_dir;
use omniget_core::models::progress::ProgressUpdate;

const SPINNER_STYLE: &str = "{spinner:.cyan} {msg:.dim}";
const PROGRESS_STYLE: &str = "{spinner:.cyan} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, ETA {eta})";

pub struct CliReporter {
    pb: ProgressBar,
    json_mode: bool,
}

impl CliReporter {
    pub fn new(json_mode: bool) -> Self {
        let pb = ProgressBar::new(100);
        pb.set_style(ProgressStyle::default_bar().template(SPINNER_STYLE).unwrap());
        Self { pb, json_mode }
    }

    pub fn message(&self, msg: &str) {
        if self.json_mode {
            println!(r#"{{"type":"message","content":"{}"}}"#, msg.replace('"', "\\\""));
        } else {
            self.pb.set_message(msg.to_string());
        }
    }

    pub fn update(&self, update: &ProgressUpdate) {
        if self.json_mode {
            let json = serde_json::json!({
                "type": "progress",
                "percent": update.percent,
                "downloaded_bytes": update.downloaded_bytes,
                "total_bytes": update.total_bytes,
                "speed_bps": update.speed_bps,
                "eta_seconds": update.eta_seconds,
            });
            println!("{}", json);
        } else {
            if update.percent >= 0.0 {
                self.pb.set_position(update.percent.clamp(0.0, 100.0).round() as u64);
                self.pb.set_style(
                    ProgressStyle::default_bar()
                        .template(PROGRESS_STYLE)
                        .unwrap(),
                );
            }
        }
    }

    pub fn finish(&self, success: bool, message: &str) {
        if self.json_mode {
            let json = serde_json::json!({
                "type": "complete",
                "success": success,
                "message": message,
            });
            println!("{}", json);
        } else {
            if success {
                self.pb.finish_with_message(format!("✓ {}", message));
            } else {
                self.pb.finish_with_message(format!("✗ {}", message));
            }
        }
    }
}

pub async fn find_yt_dlp() -> Result<PathBuf> {
    find_tool("yt-dlp")
        .await
        .ok_or_else(|| anyhow::anyhow!("yt-dlp not found in PATH or app data dir"))
}

pub fn default_output_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

pub fn default_cookie_path() -> Option<PathBuf> {
    let app_dir = app_data_dir()?;
    let cookie_file = app_dir.join("cookies").join("cookies.txt");
    if cookie_file.exists() {
        Some(cookie_file)
    } else {
        None
    }
}
