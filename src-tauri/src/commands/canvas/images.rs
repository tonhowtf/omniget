use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use omniget_core::core::canvas::assets::{self, AssetRef, ThumbInfo};
use omniget_core::core::canvas::store::CanvasStore;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::canvas::doc::host;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageResolved {
    pub asset: Option<String>,
    pub path: Option<String>,
    pub original: String,
    pub thumb: String,
    pub w: u32,
    pub h: u32,
    pub bytes: Option<u64>,
    pub imported: bool,
    pub source_url: Option<String>,
}

pub enum Source {
    Asset(String),
    Url(String),
    Base64(String),
    Path(String),
}

fn store(app: &AppHandle) -> Arc<CanvasStore> {
    host().bind(app).store().clone()
}

pub fn ws_dir(s: &CanvasStore, ws: &str) -> Result<PathBuf, String> {
    s.workspace(ws)?;
    Ok(s.workspace_dir(ws))
}

pub fn allow_thumbs(app: &AppHandle, ws_dir: &Path) {
    static DONE: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    let dir = assets::thumbs_dir(ws_dir);
    let mut done = DONE
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if done.insert(dir.clone()) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = app.asset_protocol_scope().allow_directory(&dir, true);
    }
}

pub fn web_allowed(app: &AppHandle) -> Result<(), String> {
    if crate::canvas::prefs::get(app).images.download_web {
        Ok(())
    } else {
        Err(
            "ASSET_URL_OFF: downloading images from the web is off (Settings → Canvas → Images)"
                .into(),
        )
    }
}

pub async fn import_source(
    app: &AppHandle,
    ws_dir: &Path,
    src: Source,
) -> Result<(AssetRef, Option<String>), String> {
    match src {
        Source::Asset(id) => {
            let d = ws_dir.to_path_buf();
            Ok((assets::blocking(move || assets::find(&d, &id)).await?, None))
        }
        Source::Url(u) => {
            web_allowed(app)?;
            let (r, _) = assets::put_url(ws_dir.to_path_buf(), &u).await?;
            Ok((r, Some(u)))
        }
        Source::Base64(b) => {
            let d = ws_dir.to_path_buf();
            let r = assets::blocking(move || {
                let bytes = assets::decode_base64(&b)?;
                assets::put_bytes(&d, &bytes)
            })
            .await?;
            Ok((r, None))
        }
        Source::Path(p) => {
            let d = ws_dir.to_path_buf();
            let r = assets::blocking(move || {
                let abs = assets::resolve_path(&d, &p)
                    .ok_or_else(|| format!("ASSET_NOT_FOUND: '{p}' is not a usable path"))?;
                if assets::inside(&assets::assets_dir(&d), &abs) {
                    if let Ok(r) = assets::find(&d, &abs.to_string_lossy()) {
                        return Ok(r);
                    }
                }
                assets::put_path(&d, &abs)
            })
            .await?;
            Ok((r, None))
        }
    }
}

pub async fn thumb_of(ws_dir: &Path, r: &AssetRef) -> Result<ThumbInfo, String> {
    let d = ws_dir.to_path_buf();
    let r = r.clone();
    assets::blocking(move || assets::thumb_for_asset(&d, &r)).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn canvas_image_thumb(
    app: AppHandle,
    workspace_id: String,
    asset: Option<String>,
    path: Option<String>,
    url: Option<String>,
    base64: Option<String>,
    import: Option<bool>,
) -> Result<ImageResolved, String> {
    let s = store(&app);
    let dir = ws_dir(&s, &workspace_id)?;
    allow_thumbs(&app, &dir);
    let keep_original = import == Some(false);
    if keep_original {
        if let Some(p) = path.as_deref().filter(|p| !p.trim().is_empty()) {
            let abs = assets::resolve_path(&dir, p)
                .ok_or_else(|| format!("ASSET_NOT_FOUND: '{p}' is not a usable path"))?;
            let d = dir.clone();
            let a = abs.clone();
            let t = assets::blocking(move || assets::thumb_for_file(&d, &a)).await?;
            return Ok(ImageResolved {
                asset: None,
                path: None,
                original: abs.to_string_lossy().into_owned(),
                thumb: t.thumb,
                w: t.w,
                h: t.h,
                bytes: None,
                imported: false,
                source_url: None,
            });
        }
    }
    let src = if let Some(a) = asset.filter(|a| !a.trim().is_empty()) {
        Source::Asset(a)
    } else if let Some(b) = base64.filter(|b| !b.trim().is_empty()) {
        Source::Base64(b)
    } else if let Some(p) = path.filter(|p| !p.trim().is_empty()) {
        Source::Path(p)
    } else if let Some(u) = url.filter(|u| !u.trim().is_empty()) {
        Source::Url(u)
    } else {
        return Err("ASSET_MISSING: pass asset, path, url or base64".into());
    };
    let was_asset = matches!(src, Source::Asset(_));
    let (r, source_url) = import_source(&app, &dir, src).await?;
    let t = thumb_of(&dir, &r).await?;
    Ok(ImageResolved {
        asset: Some(r.id.clone()),
        path: Some(r.path.clone()),
        original: r.abs.clone(),
        thumb: t.thumb,
        w: t.w,
        h: t.h,
        bytes: Some(r.bytes),
        imported: !was_asset,
        source_url,
    })
}

#[tauri::command]
pub async fn canvas_image_thumbs_clear(app: AppHandle) -> Result<u64, String> {
    let root = store(&app).root().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || assets::clear_thumbs(&root))
        .await
        .map_err(|e| format!("ASSET_IO: {e}"))
}

pub async fn file_preview(app: &AppHandle, ws: &str, path: &str) -> Result<Option<String>, String> {
    let s = store(app);
    let dir = ws_dir(&s, ws)?;
    let Some(abs) = assets::resolve_path(&dir, path) else {
        return Ok(None);
    };
    allow_thumbs(app, &dir);
    let d = dir.clone();
    match assets::blocking(move || assets::thumb_for_file(&d, &abs)).await {
        Ok(t) => Ok(Some(t.thumb)),
        Err(_) => Ok(None),
    }
}
