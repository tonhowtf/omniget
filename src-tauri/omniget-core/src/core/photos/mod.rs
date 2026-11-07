pub mod adjust;
pub mod export;
pub mod files;
pub mod index;
pub mod library;
pub mod meta;
pub mod preview;
pub mod similar;
pub mod thumb;

#[cfg(test)]
mod tests;

use serde::Serialize;

pub use adjust::{Crop, Edits};
pub use export::{ExportOptions, ExportOutcome};
pub use index::{Progress, Stage};
pub use library::Library;
pub use similar::{DupGroup, DupKind};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Photo {
    pub path: String,
    pub name: String,
    pub folder: String,
    pub format: String,
    pub size: u64,
    pub modified_at: i64,
    pub taken_at: Option<i64>,
    pub width: u32,
    pub height: u32,
    pub thumb: Option<String>,
    pub error: Option<String>,
    pub edited: bool,
}

pub const GALLERY_FORMATS: &[&str] = &[
    "jpeg", "png", "webp", "gif", "bmp", "tiff", "tga", "heic", "jxl", "psd", "raw",
];

pub fn format_of(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    let info = crate::core::convert::formats::by_extension(ext)?;
    GALLERY_FORMATS.contains(&info.id).then_some(info.id)
}
