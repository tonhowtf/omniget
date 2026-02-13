use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub title: String,
    pub author: String,
    pub platform: String,
    pub duration_seconds: Option<f64>,
    pub thumbnail_url: Option<String>,
    pub available_qualities: Vec<VideoQuality>,
    pub media_type: MediaType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MediaType {
    Video,
    Audio,
    Playlist,
    Course,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoQuality {
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub url: String,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadOptions {
    pub quality: Option<String>,
    pub output_dir: PathBuf,
    pub filename_template: Option<String>,
    pub download_subtitles: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadResult {
    pub file_path: PathBuf,
    pub file_size_bytes: u64,
    pub duration_seconds: f64,
}
