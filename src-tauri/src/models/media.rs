use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::platforms::Platform;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MediaType {
    Video,
    Audio,
    Photo,
    Gif,
    Carousel,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    pub url: String,
    pub media_type: MediaType,
    pub thumbnail_url: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenericDownloadResult {
    pub platform: Platform,
    pub title: String,
    pub author: String,
    pub files: Vec<DownloadedFile>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadedFile {
    pub path: PathBuf,
    pub media_type: MediaType,
    pub size_bytes: u64,
}
