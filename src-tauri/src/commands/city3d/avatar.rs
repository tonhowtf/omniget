//! Avatars and residents of the 3D city: catalog, genomes, the local avatar
//! record and deterministic residents (crate `omniget-avatar`).

use base64::Engine;
use omniget_avatar::store::{self, AvatarRecord};
use omniget_avatar::{catalog, catalog_hash, Genome};
use serde::Serialize;
use serde_json::Value;

fn avatar_file() -> Result<std::path::PathBuf, String> {
    omniget_core::core::paths::app_data_dir()
        .map(|d| store::avatar_path(&d))
        .ok_or_else(|| "app data dir unavailable".to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn fresh_seed() -> u64 {
    now_ms() ^ (std::process::id() as u64).rotate_left(32)
}

#[derive(Serialize)]
pub struct CatalogPayload {
    pub hash: String,
    pub catalog: Value,
    pub schema: Value,
}

#[tauri::command]
pub fn city3d_avatar_catalog() -> CatalogPayload {
    CatalogPayload {
        hash: catalog_hash(),
        catalog: omniget_avatar::catalog::catalog_json(),
        schema: omniget_avatar::identity_prompt_schema(),
    }
}

#[derive(Serialize)]
pub struct GenomePayload {
    pub genome: Genome,
    pub catalog_hash: String,
}

#[tauri::command]
pub fn city3d_genome_random(seed: Option<u64>) -> GenomePayload {
    GenomePayload {
        genome: Genome::random(seed.unwrap_or_else(fresh_seed)),
        catalog_hash: catalog_hash(),
    }
}

#[tauri::command]
pub fn city3d_genome_from_traits(
    traits: Value,
    seed: Option<u64>,
) -> Result<GenomePayload, String> {
    omniget_avatar::from_traits(&traits, seed.unwrap_or_else(fresh_seed))
        .map(|genome| GenomePayload {
            genome,
            catalog_hash: catalog_hash(),
        })
        .map_err(|errors| errors.join("; "))
}

#[tauri::command]
pub fn city3d_avatar_get() -> Result<Option<AvatarRecord>, String> {
    store::load(&avatar_file()?)
}

#[tauri::command]
pub fn city3d_avatar_set(record: AvatarRecord) -> Result<AvatarRecord, String> {
    let errors = record.genome.validate(catalog());
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    let record = AvatarRecord {
        catalog_hash: catalog_hash(),
        updated_ms: now_ms(),
        ..record
    };
    store::save(&avatar_file()?, &record)?;
    Ok(record)
}

#[tauri::command]
pub fn city3d_npc_identities(
    city_id: String,
    count: usize,
    seed: Option<u64>,
) -> Vec<omniget_avatar::Resident> {
    omniget_avatar::npc_identities(&city_id, count.min(5000), seed.unwrap_or(0))
}

/// Writes an exported `.vrm` (base64 from the frontend exporter) to a path the
/// user picked in the save dialog.
#[tauri::command]
pub fn city3d_avatar_export_vrm(path: String, data_base64: String) -> Result<u64, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.as_bytes())
        .map_err(|e| e.to_string())?;
    if bytes.len() < 12 || &bytes[0..4] != b"glTF" {
        return Err("not a GLB/VRM file".into());
    }
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(bytes.len() as u64)
}
