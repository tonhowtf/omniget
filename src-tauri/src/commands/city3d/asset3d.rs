//! Import normalizer and exports of the 3D city (agent F, sessão M6).
//!
//! Every import goes through `omniget_asset3d::normalize` (nine steps, on a
//! worker thread with a deadline) and lands in
//! `<app_data>/city3d/assets/<sha256>/{asset.glb, meta.json, thumb.png}`.

use base64::Engine;
use omniget_asset3d::store::{AssetMeta, AssetSource, Store};
use omniget_asset3d::{AssetClass, ImageMode, Input, License, NormalizeOptions};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Largest file the import reads (the normalizer's own limit is the same).
const MAX_IMPORT_BYTES: u64 = 64 << 20;

pub(super) fn store() -> Result<Store, String> {
    let root = crate::core::paths::app_data_dir()
        .ok_or("no app data folder")?
        .join("city3d")
        .join("assets");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(Store::new(root))
}

pub(super) fn options(
    name: &str,
    class: AssetClass,
    license: &License,
    fit_style: bool,
    image_mode: Option<ImageMode>,
    palette: Option<BTreeMap<String, String>>,
    provenance: String,
) -> NormalizeOptions {
    let mut o = NormalizeOptions::new(name, class);
    o.fit_style = fit_style;
    o.image_mode = image_mode.unwrap_or_default();
    o.palette = palette;
    o.license = Some(license.clone());
    o.provenance = Some(provenance);
    o
}

fn display_name(file_name: &str) -> String {
    let stem = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let stem = stem.rsplit_once('.').map(|(a, _)| a).unwrap_or(stem);
    let s: String = stem.chars().filter(|c| !c.is_control()).take(64).collect();
    if s.trim().is_empty() {
        "asset".into()
    } else {
        s.trim().to_string()
    }
}

async fn run_import(
    input: Input,
    opts: NormalizeOptions,
    license: License,
    source_kind: &str,
) -> Result<AssetMeta, String> {
    let file_name = input.file_name.clone();
    let fit_style = opts.fit_style;
    let asset = tokio::task::spawn_blocking(move || omniget_asset3d::normalize(input, opts))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let source = AssetSource {
        kind: source_kind.into(),
        name: file_name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_string(),
        format: asset.report.format.clone(),
        source_hash: asset.source_hash.clone(),
        provider: None,
    };
    store()?.save(&asset, license, source, fit_style)
}

/// Imports a file the user picked. `.gltf` and `.obj` may read their
/// buffers, textures and `.mtl` from the same folder.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn city3d_import(
    path: String,
    class: AssetClass,
    license: License,
    fit_style: bool,
    image_mode: Option<ImageMode>,
    palette: Option<BTreeMap<String, String>>,
) -> Result<AssetMeta, String> {
    let p = PathBuf::from(&path);
    let meta = std::fs::metadata(&p).map_err(|_| "file not found".to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(format!(
            "the file has {} MB (max {} MB)",
            meta.len() >> 20,
            MAX_IMPORT_BYTES >> 20
        ));
    }
    let bytes = tokio::fs::read(&p).await.map_err(|e| e.to_string())?;
    let file_name = p
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut input = Input::bytes(bytes, &file_name);
    if let Some(dir) = p.parent() {
        input.resolver = Some(omniget_asset3d::folder_resolver(
            dir.to_path_buf(),
            MAX_IMPORT_BYTES as usize,
        ));
    }
    let opts = options(
        &display_name(&file_name),
        class,
        &license,
        fit_style,
        image_mode,
        palette,
        format!("file:{file_name}"),
    );
    run_import(input, opts, license, "file").await
}

/// Imports bytes the front already has (drag and drop, a generator result).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn city3d_import_bytes(
    name: String,
    bytes_base64: String,
    class: AssetClass,
    license: License,
    fit_style: bool,
    image_mode: Option<ImageMode>,
    palette: Option<BTreeMap<String, String>>,
) -> Result<AssetMeta, String> {
    if bytes_base64.len() as u64 > MAX_IMPORT_BYTES * 4 / 3 + 16 {
        return Err(format!(
            "the file is larger than {} MB",
            MAX_IMPORT_BYTES >> 20
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_base64.trim())
        .map_err(|_| "bad base64".to_string())?;
    let input = Input::bytes(bytes, &name);
    let opts = options(
        &display_name(&name),
        class,
        &license,
        fit_style,
        image_mode,
        palette,
        format!("bytes:{name}"),
    );
    run_import(input, opts, license, "bytes").await
}

#[tauri::command]
pub async fn city3d_assets_list() -> Result<Vec<AssetMeta>, String> {
    Ok(store()?.list())
}

/// Normalized GLB (meshopt) of a stored asset.
#[tauri::command]
pub async fn city3d_asset_glb(id: String) -> Result<tauri::ipc::Response, String> {
    Ok(tauri::ipc::Response::new(store()?.glb(&id)?))
}

/// 256 px PNG thumbnail of a stored asset.
#[tauri::command]
pub async fn city3d_asset_thumb(id: String) -> Result<tauri::ipc::Response, String> {
    Ok(tauri::ipc::Response::new(store()?.thumbnail(&id)?))
}

#[tauri::command]
pub async fn city3d_asset_delete(id: String) -> Result<(), String> {
    store()?.delete(&id)
}

#[derive(Serialize, Deserialize)]
pub struct VoxExport {
    /// Present when no `path` was given.
    pub vox_base64: Option<String>,
    pub path: Option<String>,
    pub voxel_mm: u32,
    pub voxels: usize,
    pub size: [u32; 3],
}

fn check_export_path(path: &str, allowed: &[&str]) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !allowed.contains(&ext.as_str()) {
        return Err(format!("the file must end in .{}", allowed.join(" or .")));
    }
    if !p.is_absolute() {
        return Err("the export path must be absolute".into());
    }
    Ok(p)
}

/// MagicaVoxel `.vox` of a stored asset. With `path` (from the save dialog)
/// the file is written there; without it the bytes come back in base64.
#[tauri::command]
pub async fn city3d_export_vox(
    id: String,
    voxel_mm: u32,
    path: Option<String>,
) -> Result<VoxExport, String> {
    let glb = store()?.glb(&id)?;
    let r = tokio::task::spawn_blocking(move || omniget_asset3d::export_vox(&glb, voxel_mm))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    if let Some(path) = path {
        let p = check_export_path(&path, &["vox"])?;
        tokio::fs::write(&p, &r.bytes)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(VoxExport {
            vox_base64: None,
            path: Some(path),
            voxel_mm: r.voxel_mm,
            voxels: r.voxels,
            size: r.size,
        });
    }
    Ok(VoxExport {
        vox_base64: Some(base64::engine::general_purpose::STANDARD.encode(&r.bytes)),
        path: None,
        voxel_mm: r.voxel_mm,
        voxels: r.voxels,
        size: r.size,
    })
}

/// Writes an export the front built (GLB of a building, a city cut or a
/// character) to the path the user chose in the save dialog.
#[tauri::command]
pub async fn city3d_export_write(path: String, bytes_base64: String) -> Result<(), String> {
    let p = check_export_path(&path, &["glb", "vrm", "vox"])?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_base64.trim())
        .map_err(|_| "bad base64".to_string())?;
    tokio::fs::write(&p, bytes).await.map_err(|e| e.to_string())
}

/// Copies a stored asset's GLB to a path from the save dialog. VRM assets
/// whose `meta` forbids export are refused.
#[tauri::command]
pub async fn city3d_asset_export(id: String, path: String) -> Result<(), String> {
    let s = store()?;
    let meta = s.meta(&id)?;
    if let Some(v) = &meta.vrm {
        if !v.permissions.export {
            return Err("this VRM's licence does not allow exporting a modified copy".into());
        }
    }
    let p = check_export_path(&path, &["glb", "vrm"])?;
    tokio::fs::write(&p, s.glb(&id)?)
        .await
        .map_err(|e| e.to_string())
}
