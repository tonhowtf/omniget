use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, FilePath};

use omniget_core::core::board::{
    self, names, store, BoardConfig, BoardEntry, BoardError, SavedInfo,
};

static OPENED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct BoardLibrary {
    pub folder: String,
    pub is_default: bool,
    pub boards: Vec<BoardEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BoardFile {
    pub path: String,
    pub name: String,
    pub contents: String,
    pub external: bool,
    pub modified_ms: u64,
}

enum Place {
    Library(PathBuf, PathBuf),
    External(PathBuf, PathBuf),
}

impl Place {
    fn path(&self) -> &Path {
        match self {
            Place::Library(_, p) | Place::External(_, p) => p,
        }
    }

    fn dir(&self) -> &Path {
        match self {
            Place::Library(d, _) | Place::External(d, _) => d,
        }
    }
}

fn err(e: BoardError) -> String {
    e.to_string()
}

fn folder() -> Result<(PathBuf, bool), String> {
    let (dir, is_default) = board::active_folder().map_err(err)?;
    if is_default {
        store::ensure_dir(&dir).map_err(err)?;
    } else if !dir.is_dir() {
        return Err(err(BoardError::NoFolder));
    }
    let canon = std::fs::canonicalize(&dir).map_err(|_| err(BoardError::NoFolder))?;
    Ok((canon, is_default))
}

async fn blocking<T: Send + 'static>(
    run: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(run)
        .await
        .map_err(|e| e.to_string())?
}

fn remember(path: &Path) {
    if let Ok(canon) = std::fs::canonicalize(path) {
        if let Ok(mut set) = OPENED.lock() {
            set.get_or_insert_with(HashSet::new).insert(canon);
        }
    }
}

fn forget(path: &Path) {
    if let Ok(mut set) = OPENED.lock() {
        if let Some(s) = set.as_mut() {
            s.remove(path);
        }
    }
}

fn was_opened(path: &Path) -> bool {
    OPENED
        .lock()
        .map(|set| set.as_ref().is_some_and(|s| s.contains(path)))
        .unwrap_or(false)
}

fn place(path: &str) -> Result<Place, String> {
    let raw = PathBuf::from(path);
    if let Ok((dir, _)) = folder() {
        if let Ok(resolved) = store::resolve(&dir, &raw) {
            return Ok(Place::Library(dir, resolved));
        }
    }
    let canon = std::fs::canonicalize(&raw).map_err(|_| err(BoardError::NotFound))?;
    if was_opened(&canon) {
        let parent = canon
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| err(BoardError::Outside))?;
        return Ok(Place::External(parent, canon));
    }
    Err(err(BoardError::Outside))
}

fn library() -> Result<BoardLibrary, String> {
    let (dir, is_default) = folder()?;
    Ok(BoardLibrary {
        folder: dir.to_string_lossy().into_owned(),
        is_default,
        boards: store::list(&dir).map_err(err)?,
    })
}

fn into_paths(picked: Option<Vec<FilePath>>) -> Vec<PathBuf> {
    picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .collect()
}

async fn ask<T: Send + 'static>(
    run: impl FnOnce(Box<dyn FnOnce(T) + Send>) + Send + 'static,
) -> Option<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    run(Box::new(move |value| {
        let _ = tx.send(value);
    }));
    rx.await.ok()
}

fn decode(data: &str) -> Result<Vec<u8>, String> {
    let raw = data.split_once(',').map(|(_, b)| b).unwrap_or(data);
    base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .map_err(|_| "BOARD_BAD_DATA".to_string())
}

fn read_file(path: &Path, external: bool) -> Result<BoardFile, String> {
    let contents = store::read(path).map_err(err)?;
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = names::stem_of(&file).unwrap_or(&file).to_string();
    let modified_ms = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok(BoardFile {
        path: path.to_string_lossy().into_owned(),
        name,
        contents,
        external,
        modified_ms,
    })
}

#[tauri::command]
pub async fn board_library() -> Result<BoardLibrary, String> {
    blocking(library).await
}

#[tauri::command]
pub async fn board_choose_folder(
    app: AppHandle,
    title: Option<String>,
) -> Result<Option<BoardLibrary>, String> {
    let current = blocking(|| {
        Ok(folder()
            .ok()
            .map(|(dir, _)| dir)
            .or_else(board::default_folder))
    })
    .await?;
    let picked = ask(move |done| {
        let mut builder = app.dialog().file();
        if let Some(dir) = current.filter(|d| d.is_dir()) {
            builder = builder.set_directory(dir);
        }
        if let Some(t) = title {
            builder = builder.set_title(t);
        }
        builder.pick_folder(done);
    })
    .await
    .flatten()
    .and_then(|p| p.into_path().ok());
    let Some(dir) = picked else {
        return Ok(None);
    };
    blocking(move || {
        if !dir.is_dir() {
            return Err(err(BoardError::NoFolder));
        }
        board::save_config(&BoardConfig { folder: Some(dir) }).map_err(err)?;
        library().map(Some)
    })
    .await
}

#[tauri::command]
pub async fn board_reset_folder() -> Result<BoardLibrary, String> {
    blocking(|| {
        board::save_config(&BoardConfig::default()).map_err(err)?;
        library()
    })
    .await
}

#[tauri::command]
pub async fn board_create(name: Option<String>, fallback: String) -> Result<BoardEntry, String> {
    blocking(move || {
        let (dir, _) = folder()?;
        store::create(&dir, name.as_deref(), &fallback).map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn board_read(path: String) -> Result<BoardFile, String> {
    blocking(move || {
        let at = place(&path)?;
        read_file(at.path(), matches!(at, Place::External(..)))
    })
    .await
}

#[tauri::command]
pub async fn board_open_file(
    app: AppHandle,
    title: Option<String>,
) -> Result<Option<BoardFile>, String> {
    let picked = ask(move |done| {
        let mut builder = app
            .dialog()
            .file()
            .add_filter("Excalidraw", &[names::EXTENSION]);
        if let Some(t) = title {
            builder = builder.set_title(t);
        }
        builder.pick_file(done);
    })
    .await
    .flatten()
    .and_then(|p| p.into_path().ok());
    let Some(path) = picked else {
        return Ok(None);
    };
    blocking(move || {
        let file = read_file(&path, true)?;
        remember(&path);
        let canon = std::fs::canonicalize(&path).unwrap_or(path);
        let external = folder()
            .map(|(dir, _)| store::resolve(&dir, &canon).is_err())
            .unwrap_or(true);
        Ok(Some(BoardFile {
            path: canon.to_string_lossy().into_owned(),
            external,
            ..file
        }))
    })
    .await
}

#[tauri::command]
pub async fn board_save(
    path: String,
    contents: String,
    thumbnail: Option<String>,
) -> Result<SavedInfo, String> {
    blocking(move || {
        let at = place(&path)?;
        let info = store::save(at.path(), &contents).map_err(err)?;
        if let (Place::Library(dir, file), Some(data)) = (&at, thumbnail) {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .and_then(|n| names::stem_of(&n).map(str::to_string));
            if let (Some(name), Ok(png)) = (name, decode(&data)) {
                let _ = store::write_thumb(dir, &name, &png);
            }
        }
        Ok(info)
    })
    .await
}

#[tauri::command]
pub async fn board_rename(path: String, new_name: String) -> Result<BoardEntry, String> {
    blocking(move || {
        let at = place(&path)?;
        let entry = store::rename(at.dir(), at.path(), &new_name).map_err(err)?;
        if let Place::External(_, old) = &at {
            forget(old);
            remember(Path::new(&entry.path));
        }
        Ok(entry)
    })
    .await
}

#[tauri::command]
pub async fn board_duplicate(path: String, suffix: String) -> Result<BoardEntry, String> {
    blocking(move || match place(&path)? {
        Place::Library(lib, file) => store::duplicate(&lib, &file, &suffix).map_err(err),
        Place::External(_, file) => {
            let (dir, _) = folder()?;
            store::import(&dir, &file).map_err(err)
        }
    })
    .await
}

#[tauri::command]
pub async fn board_delete(path: String) -> Result<store::Deleted, String> {
    blocking(move || match place(&path)? {
        Place::Library(dir, file) => store::delete(&dir, &file, true).map_err(err),
        Place::External(..) => Err(err(BoardError::Outside)),
    })
    .await
}

#[tauri::command]
pub async fn board_restore(restore_path: String) -> Result<BoardEntry, String> {
    blocking(move || {
        let (dir, _) = folder()?;
        store::restore(&dir, Path::new(&restore_path)).map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn board_import(
    app: AppHandle,
    title: Option<String>,
) -> Result<Vec<BoardEntry>, String> {
    let picked = ask(move |done| {
        let mut builder = app
            .dialog()
            .file()
            .add_filter("Excalidraw", &[names::EXTENSION]);
        if let Some(t) = title {
            builder = builder.set_title(t);
        }
        builder.pick_files(done);
    })
    .await
    .flatten();
    let paths = into_paths(picked);
    blocking(move || {
        let (dir, _) = folder()?;
        let mut out = Vec::new();
        let mut first_error = None;
        for path in paths {
            match store::import(&dir, &path) {
                Ok(entry) => out.push(entry),
                Err(e) => first_error = first_error.or(Some(err(e))),
            }
        }
        match (out.is_empty(), first_error) {
            (true, Some(e)) => Err(e),
            _ => Ok(out),
        }
    })
    .await
}

#[tauri::command]
pub async fn board_thumbnail(path: String) -> Result<Option<String>, String> {
    blocking(move || {
        let Place::Library(dir, file) = place(&path)? else {
            return Ok(None);
        };
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .and_then(|n| names::stem_of(&n).map(str::to_string));
        Ok(name.and_then(|n| store::read_thumb(&dir, &n)).map(|png| {
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(png)
            )
        }))
    })
    .await
}

#[tauri::command]
pub async fn board_export(
    app: AppHandle,
    file_name: String,
    kind: String,
    data: String,
    title: Option<String>,
) -> Result<Option<String>, String> {
    let (label, ext, binary) = match kind.as_str() {
        "png" => ("PNG", "png", true),
        "svg" => ("SVG", "svg", false),
        "excalidraw" => ("Excalidraw", names::EXTENSION, false),
        _ => return Err("BOARD_BAD_DATA".into()),
    };
    let bytes = if binary {
        decode(&data)?
    } else {
        data.into_bytes()
    };
    if kind == "excalidraw" {
        store::validate_scene(&bytes).map_err(err)?;
    }
    let stem = names::validate(&file_name).unwrap_or_else(|_| "board".into());
    let suggested = format!("{stem}.{ext}");
    let picked = ask(move |done| {
        let mut builder = app
            .dialog()
            .file()
            .add_filter(label, &[ext])
            .set_file_name(suggested);
        if let Some(t) = title {
            builder = builder.set_title(t);
        }
        builder.save_file(done);
    })
    .await
    .flatten()
    .and_then(|p| p.into_path().ok());
    let Some(mut target) = picked else {
        return Ok(None);
    };
    if target.extension().is_none() {
        target.set_extension(ext);
    }
    blocking(move || {
        store::atomic_write(&target, &bytes).map_err(err)?;
        Ok(Some(target.to_string_lossy().into_owned()))
    })
    .await
}

#[tauri::command]
pub async fn board_copy_png(app: AppHandle, data: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    blocking(move || {
        let png = decode(&data)?;
        let rgba = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .map_err(|e| format!("BOARD_CLIPBOARD: {e}"))?
            .to_rgba8();
        let (w, h) = rgba.dimensions();
        let img = tauri::image::Image::new_owned(rgba.into_raw(), w, h);
        app.clipboard()
            .write_image(&img)
            .map_err(|e| format!("BOARD_CLIPBOARD: {e}"))
    })
    .await
}
