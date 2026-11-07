pub mod names;
pub mod store;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use store::{BoardEntry, SavedInfo};

pub const THUMBS_DIR: &str = ".thumbs";
pub const CONFIG_FILE: &str = "board.json";
pub const MAX_SCENE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_THUMB_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum BoardError {
    #[error("BOARD_NAME_EMPTY")]
    NameEmpty,
    #[error("BOARD_NAME_INVALID")]
    NameInvalid,
    #[error("BOARD_NAME_TOO_LONG")]
    NameTooLong,
    #[error("BOARD_EXISTS")]
    Exists,
    #[error("BOARD_NOT_FOUND")]
    NotFound,
    #[error("BOARD_OUTSIDE")]
    Outside,
    #[error("BOARD_NOT_SCENE")]
    NotScene,
    #[error("BOARD_TOO_LARGE")]
    TooLarge,
    #[error("BOARD_NO_FOLDER")]
    NoFolder,
    #[error("BOARD_IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("BOARD_TRASH: {0}")]
    Trash(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoardConfig {
    #[serde(default)]
    pub folder: Option<PathBuf>,
}

pub fn default_folder() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("board"))
}

fn config_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(CONFIG_FILE))
}

pub fn load_config() -> BoardConfig {
    config_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_config(config: &BoardConfig) -> Result<(), BoardError> {
    let path = config_path().ok_or(BoardError::NoFolder)?;
    let bytes = serde_json::to_vec_pretty(config).map_err(std::io::Error::other)?;
    store::atomic_write(&path, &bytes)
}

pub fn active_folder() -> Result<(PathBuf, bool), BoardError> {
    let config = load_config();
    if let Some(folder) = config.folder.filter(|f| f.is_absolute()) {
        return Ok((folder, false));
    }
    default_folder()
        .map(|f| (f, true))
        .ok_or(BoardError::NoFolder)
}
