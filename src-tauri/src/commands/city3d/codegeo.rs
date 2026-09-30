//! Geometry by code and the code-generated kit (agent E, sessão M5).
//!
//! - `city3d_kit {style}`: the style's kit GLB, one node per module key,
//!   built from `omniget-codegeo/kit/` and cached by content hash.
//! - `city3d_codegeo_run {script, class}`: runs a Rhai script in the sandbox
//!   and returns the canonical GLB with its anchors, or the errors.
//! - `city3d_typologies` / `city3d_typology_save` / `city3d_typology_delete`:
//!   saved scripts a building can use as `codegeo:<hash>`.
//!
//! Cache: `<app_data>/city3d/codegeo/`; typologies: `<app_data>/city3d/typologies/`.

use base64::Engine as _;
use omniget_codegeo as cg;
use serde::Serialize;
use std::path::PathBuf;

fn city_dir() -> Result<PathBuf, String> {
    crate::core::paths::app_data_dir()
        .map(|d| d.join("city3d"))
        .ok_or_else(|| "ERR_CITY3D_DATA_DIR: could not resolve the app data directory".to_string())
}

pub fn codegeo_dir() -> Result<PathBuf, String> {
    Ok(city_dir()?.join("codegeo"))
}

pub fn typologies_dir() -> Result<PathBuf, String> {
    Ok(city_dir()?.join("typologies"))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("ERR_CITY3D_CODEGEO: {e}"))
}

#[derive(Serialize)]
pub struct RunResult {
    /// Empty when `errors` is not.
    pub glb_base64: String,
    pub hash: String,
    pub typology: String,
    pub anchors: Vec<cg::Anchor>,
    pub anchor_points: Vec<cg::AnchorPoint>,
    pub triangles: usize,
    pub footprint: Option<cg::Footprint>,
    pub slots: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<cg::ScriptError>,
    pub ms: f64,
}

impl RunResult {
    fn from(r: Result<cg::Output, Vec<cg::ScriptError>>) -> Self {
        match r {
            Ok(o) => RunResult {
                glb_base64: base64::engine::general_purpose::STANDARD.encode(&o.glb),
                hash: o.hash,
                typology: o.typology,
                anchors: o.anchors,
                anchor_points: o.anchor_points,
                triangles: o.triangles,
                footprint: Some(o.footprint),
                slots: o.slots,
                warnings: o.warnings,
                errors: vec![],
                ms: o.ms,
            },
            Err(errors) => RunResult {
                glb_base64: String::new(),
                hash: String::new(),
                typology: String::new(),
                anchors: vec![],
                anchor_points: vec![],
                triangles: 0,
                footprint: None,
                slots: vec![],
                warnings: vec![],
                errors,
                ms: 0.0,
            },
        }
    }
}

/// The kit GLB of a style (`vila-colonial`, `vila-praiana`, `vila-arida`).
#[tauri::command]
pub async fn city3d_kit(style: String) -> Result<tauri::ipc::Response, String> {
    let dir = codegeo_dir()?;
    let bytes = blocking(move || cg::kit::kit_glb_cached(&dir, &style)).await??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Per-module triangle counts, build times and budget checks of a style.
#[tauri::command]
pub async fn city3d_kit_stats(style: String) -> Result<serde_json::Value, String> {
    blocking(move || {
        let kit = cg::kit::build_kit(&style)?;
        Ok(serde_json::json!({
            "style": kit.style,
            "hash": kit.hash,
            "modules": kit.modules.iter().map(|m| &m.stats).collect::<Vec<_>>(),
            "aliases": kit.aliases,
            "missing": kit.missing,
            "errors": kit.errors.iter().map(|(k, e)| serde_json::json!({ "key": k, "error": e })).collect::<Vec<_>>(),
        }))
    })
    .await?
}

/// Runs a script. `class`: `prop`, `building` or `module`, optionally with a
/// kind (`prop:bench`). Script errors come back in `errors`, not as `Err`.
#[tauri::command]
pub async fn city3d_codegeo_run(
    script: String,
    class: Option<String>,
) -> Result<RunResult, String> {
    let dir = codegeo_dir()?;
    let class = class.unwrap_or_else(|| "prop".into());
    blocking(move || RunResult::from(cg::run_cached(&dir, &script, &class))).await
}

#[tauri::command]
pub async fn city3d_typologies() -> Result<Vec<cg::typology::Typology>, String> {
    let dir = typologies_dir()?;
    blocking(move || cg::typology::list(&dir)).await
}

#[derive(Serialize)]
pub struct SaveResult {
    pub typology: Option<cg::typology::Typology>,
    pub errors: Vec<cg::ScriptError>,
}

/// Validates and stores a script as a typology.
#[tauri::command]
pub async fn city3d_typology_save(
    name: String,
    script: String,
    class: Option<String>,
) -> Result<SaveResult, String> {
    let dir = typologies_dir()?;
    let class = class.unwrap_or_else(|| "building".into());
    blocking(
        move || match cg::typology::save(&dir, &name, &script, &class) {
            Ok(t) => SaveResult {
                typology: Some(t),
                errors: vec![],
            },
            Err(errors) => SaveResult {
                typology: None,
                errors,
            },
        },
    )
    .await
}

#[tauri::command]
pub async fn city3d_typology_delete(id: String) -> Result<bool, String> {
    let dir = typologies_dir()?;
    blocking(move || cg::typology::delete(&dir, &id)).await
}

/// Ready examples (fonte, quiosque, sobrado) for the editor.
#[tauri::command]
pub fn city3d_codegeo_examples() -> Vec<cg::Example> {
    cg::examples()
}

/// The kit documentation (the same text the LLM gets).
#[tauri::command]
pub fn city3d_codegeo_reference() -> String {
    cg::kit_reference()
}
