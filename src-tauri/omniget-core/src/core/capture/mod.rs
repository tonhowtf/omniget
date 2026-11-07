pub mod args;
pub mod devices;
pub mod errors;
pub mod history;
pub mod naming;
pub mod probe;
pub mod runner;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Macos,
    Windows,
    Linux,
    Other,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::Macos
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "linux") {
            Platform::Linux
        } else {
            Platform::Other
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Screen,
    Window,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sources {
    pub platform: Platform,
    pub ffmpeg: bool,
    pub screens: Vec<Source>,
    pub audio: Vec<Source>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    #[default]
    Mp4,
    Mov,
    Mkv,
}

impl Container {
    pub fn ext(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mov => "mov",
            Container::Mkv => "mkv",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    High,
    #[default]
    Balanced,
    Small,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordOptions {
    pub screen: String,
    #[serde(default)]
    pub region: Option<Region>,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default)]
    pub mic: Option<String>,
    #[serde(default = "default_true")]
    pub cursor: bool,
    #[serde(default)]
    pub container: Container,
    #[serde(default)]
    pub quality: Quality,
    #[serde(default)]
    pub out_dir: Option<String>,
}

fn default_fps() -> u32 {
    30
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
}

impl Segment {
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportMode {
    #[default]
    Merge,
    Separate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRequest {
    pub path: String,
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub mode: ExportMode,
    #[serde(default)]
    pub precise: bool,
    #[serde(default)]
    pub strip_audio: bool,
    #[serde(default)]
    pub out_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub outputs: Vec<String>,
    pub size_bytes: u64,
}
