//! The local player's avatar record, kept as `<app_data>/city3d/avatar.json`.

use crate::genome::Genome;
use crate::identity::Identity;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarRecord {
    pub genome: Genome,
    #[serde(default)]
    pub identity: Option<Identity>,
    /// Creator-only extras beyond the 32 bytes (free RGB, all local changes);
    /// opaque to Rust.
    #[serde(default)]
    pub extended: Option<serde_json::Value>,
    #[serde(default)]
    pub catalog_hash: String,
    #[serde(default)]
    pub updated_ms: u64,
}

pub fn avatar_path(app_data: &Path) -> PathBuf {
    app_data.join("city3d").join("avatar.json")
}

pub fn load(path: &Path) -> Result<Option<AvatarRecord>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("avatar.json is invalid: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn save(path: &Path, record: &AvatarRecord) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(record).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}
