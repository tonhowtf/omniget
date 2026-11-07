use std::path::PathBuf;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use omniget_core::core::icons::{
    cache::{write_atomic, Cache},
    library::{self, ExportFile},
    service, CollectionsResult, ErrorKind, IconsError, IconsResult, Library, SearchResult,
    API_BASE,
};

const MAX_NAMES: usize = 400;
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
const MAX_START: u32 = 4096;

static LIBRARY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn no_data_dir() -> IconsError {
    IconsError::new(
        ErrorKind::Io,
        "no_data_dir",
        "The app data folder is not available.",
    )
}

fn join_error(e: tokio::task::JoinError) -> IconsError {
    IconsError::io(e.to_string())
}

fn cache() -> Result<Cache, IconsError> {
    Cache::open_default().ok_or_else(no_data_dir)
}

fn library_path() -> Result<PathBuf, IconsError> {
    library::library_path().ok_or_else(no_data_dir)
}

#[tauri::command]
pub async fn icons_collections(refresh: Option<bool>) -> Result<CollectionsResult, IconsError> {
    let client = service::client()?;
    service::collections(&client, &cache()?, API_BASE, refresh.unwrap_or(false)).await
}

#[tauri::command]
pub async fn icons_search(
    query: String,
    prefixes: Option<Vec<String>>,
    start: Option<u32>,
) -> Result<SearchResult, IconsError> {
    let client = service::client()?;
    service::search(
        &client,
        &cache()?,
        API_BASE,
        &query,
        &prefixes.unwrap_or_default(),
        start.unwrap_or(0).min(MAX_START),
    )
    .await
}

#[tauri::command]
pub async fn icons_get(prefix: String, names: Vec<String>) -> Result<IconsResult, IconsError> {
    if names.len() > MAX_NAMES {
        return Err(IconsError::invalid(
            "too_many_names",
            format!("Ask for at most {MAX_NAMES} icons at a time."),
        )
        .with("max", MAX_NAMES));
    }
    let client = service::client()?;
    service::icons(&client, &cache()?, API_BASE, &prefix, &names).await
}

#[tauri::command]
pub async fn icons_library() -> Result<Library, IconsError> {
    let path = library_path()?;
    tokio::task::spawn_blocking(move || {
        let _guard = LIBRARY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        library::load_from(&path)
    })
    .await
    .map_err(join_error)
}

#[tauri::command]
pub async fn icons_library_save(library: Library) -> Result<Library, IconsError> {
    let path = library_path()?;
    tokio::task::spawn_blocking(move || {
        let _guard = LIBRARY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        library::save_to(&path, library)
    })
    .await
    .map_err(join_error)?
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheInfo {
    pub bytes: u64,
}

#[tauri::command]
pub async fn icons_cache_info() -> Result<CacheInfo, IconsError> {
    let cache = cache()?;
    tokio::task::spawn_blocking(move || CacheInfo {
        bytes: cache.size_bytes(),
    })
    .await
    .map_err(join_error)
}

#[tauri::command]
pub async fn icons_cache_clear() -> Result<(), IconsError> {
    let cache = cache()?;
    tokio::task::spawn_blocking(move || cache.clear())
        .await
        .map_err(join_error)?
        .map_err(IconsError::from)
}

async fn ask_save_path(
    app: &AppHandle,
    file_name: &str,
    label: &str,
    ext: &str,
) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(file_name)
        .add_filter(label, &[ext])
        .save_file(move |picked| {
            let _ = tx.send(picked.and_then(|p| p.into_path().ok()));
        });
    rx.await.ok().flatten()
}

async fn ask_folder(app: &AppHandle) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |picked| {
        let _ = tx.send(picked.and_then(|p| p.into_path().ok()));
    });
    rx.await.ok().flatten()
}

fn target_path(picked: PathBuf, ext: &str) -> Result<PathBuf, IconsError> {
    let ok = picked
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext));
    if ok {
        return Ok(picked);
    }
    let mut name = picked
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".{ext}"));
    let mut path = picked;
    path.set_file_name(&name);
    if path.exists() {
        return Err(IconsError::invalid(
            "file_exists",
            format!(
                "A file named {} already exists there. Pick another name.",
                name.to_string_lossy()
            ),
        )
        .with("name", name.to_string_lossy()));
    }
    Ok(path)
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SaveKind {
    Svg,
    Png,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavedFile {
    pub path: String,
    pub count: usize,
}

#[tauri::command]
pub async fn icons_save_file(
    app: AppHandle,
    kind: SaveKind,
    file_name: String,
    text: Option<String>,
    base64: Option<String>,
) -> Result<Option<SavedFile>, IconsError> {
    let bytes = match kind {
        SaveKind::Svg => text
            .ok_or_else(|| IconsError::invalid("no_svg", "There is no SVG to save."))?
            .into_bytes(),
        SaveKind::Png => {
            let raw =
                base64.ok_or_else(|| IconsError::invalid("no_png", "There is no PNG to save."))?;
            let data = raw.split_once(',').map(|(_, d)| d).unwrap_or(&raw);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.trim())
                .map_err(|e| {
                    IconsError::invalid("png_damaged", format!("The PNG data is damaged: {e}"))
                })?;
            if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
                return Err(IconsError::invalid("not_png", "The image is not a PNG."));
            }
            bytes
        }
    };
    if bytes.is_empty() || bytes.len() > MAX_FILE_BYTES {
        return Err(IconsError::invalid(
            "file_size",
            "The file is empty or too large.",
        ));
    }
    let (label, ext) = match kind {
        SaveKind::Svg => ("SVG", "svg"),
        SaveKind::Png => ("PNG", "png"),
    };
    let stem = library::safe_file_stem(&file_name);
    let Some(path) = ask_save_path(&app, &format!("{stem}.{ext}"), label, ext).await else {
        return Ok(None);
    };
    let path = target_path(path, ext)?;
    let out = path.clone();
    tokio::task::spawn_blocking(move || write_atomic(&out, &bytes))
        .await
        .map_err(join_error)??;
    Ok(Some(SavedFile {
        path: path.to_string_lossy().into_owned(),
        count: 1,
    }))
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ExportMode {
    Folder,
    Sprite,
}

#[tauri::command]
pub async fn icons_export(
    app: AppHandle,
    mode: ExportMode,
    name: String,
    files: Vec<ExportFile>,
    sprite: Option<String>,
) -> Result<Option<SavedFile>, IconsError> {
    let total: usize =
        files.iter().map(|f| f.svg.len()).sum::<usize>() + sprite.as_ref().map_or(0, |s| s.len());
    if total > MAX_FILE_BYTES {
        return Err(IconsError::invalid(
            "export_too_large",
            "The export is too large.",
        ));
    }
    match mode {
        ExportMode::Sprite => {
            let svg = sprite
                .ok_or_else(|| IconsError::invalid("no_sprite", "There is no sprite to save."))?;
            let stem = library::safe_file_stem(&name);
            let Some(path) = ask_save_path(&app, &format!("{stem}-sprite.svg"), "SVG", "svg").await
            else {
                return Ok(None);
            };
            let path = target_path(path, "svg")?;
            let out = path.clone();
            tokio::task::spawn_blocking(move || write_atomic(&out, svg.as_bytes()))
                .await
                .map_err(join_error)??;
            Ok(Some(SavedFile {
                path: path.to_string_lossy().into_owned(),
                count: files.len(),
            }))
        }
        ExportMode::Folder => {
            let Some(dir) = ask_folder(&app).await else {
                return Ok(None);
            };
            let (out, count) =
                tokio::task::spawn_blocking(move || library::write_folder(&dir, &name, &files))
                    .await
                    .map_err(join_error)??;
            Ok(Some(SavedFile {
                path: out.to_string_lossy().into_owned(),
                count,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_picked_path_when_the_extension_matches() {
        assert_eq!(
            target_path(PathBuf::from("/a/b.SVG"), "svg").unwrap(),
            PathBuf::from("/a/b.SVG")
        );
    }

    #[test]
    fn adds_the_extension_only_when_nothing_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let picked = dir.path().join("b");
        assert_eq!(
            target_path(picked.clone(), "svg").unwrap(),
            dir.path().join("b.svg")
        );
        std::fs::write(dir.path().join("b.svg"), "x").unwrap();
        let err = target_path(picked, "svg").unwrap_err();
        assert_eq!(err.code, "file_exists");
        assert_eq!(
            target_path(dir.path().join("b.txt"), "png").unwrap(),
            dir.path().join("b.txt.png")
        );
    }
}
