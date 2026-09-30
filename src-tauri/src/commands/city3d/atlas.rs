//! The kit baked into sprites for the city's lite mode (the 2D renderer).
//!
//! - `city3d_kit_atlas {style}`: `{style, manifest, pages}` where `manifest`
//!   is the atlas JSON `src/lib/world/render/atlas.ts` reads (plus a
//!   `city3d` block: key → rotation → slot layers) and `pages` are its PNG
//!   pages in base64, in `manifest.pages` order.
//!
//! Cache: `<app_data>/city3d/atlas/`, named by kit hash and bake version.

use base64::Engine as _;
use omniget_codegeo as cg;
use serde::Serialize;
use std::path::PathBuf;

fn atlas_dir() -> Result<PathBuf, String> {
    crate::core::paths::app_data_dir()
        .map(|d| d.join("city3d").join("atlas"))
        .ok_or_else(|| "ERR_CITY3D_DATA_DIR: could not resolve the app data directory".to_string())
}

#[derive(Serialize)]
pub struct KitAtlas {
    pub style: String,
    pub manifest: serde_json::Value,
    pub pages: Vec<String>,
}

/// Sprite atlas of a style's kit (`vila-colonial`, `vila-praiana`,
/// `vila-arida`), 4 rotations per key, one grey layer per palette slot.
#[tauri::command]
pub async fn city3d_kit_atlas(style: String) -> Result<KitAtlas, String> {
    let dir = atlas_dir()?;
    tokio::task::spawn_blocking(move || {
        let baked = cg::sprites::atlas_cached(&dir, &style)
            .map_err(|e| format!("ERR_CITY3D_ATLAS: {e}"))?;
        let b64 = base64::engine::general_purpose::STANDARD;
        Ok(KitAtlas {
            style,
            manifest: baked.manifest,
            pages: baked.pages.iter().map(|p| b64.encode(p)).collect(),
        })
    })
    .await
    .map_err(|e| format!("ERR_CITY3D_ATLAS: {e}"))?
}
