use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

use omniget_core::core::photos::files::{self, FileReport, RemoveMode};
use omniget_core::core::photos::preview::{self, Preview};
use omniget_core::core::photos::{
    export, index, similar, DupGroup, Edits, ExportOptions, ExportOutcome, Library, Photo,
    Progress, Stage,
};

const INDEX_EVENT: &str = "photos-index-progress";
const EXPORT_EVENT: &str = "photos-export-progress";
const EXPORT_PARALLEL: usize = 2;
const REPORT_EVERY: Duration = Duration::from_millis(150);

static INDEX_TOKEN: Mutex<Option<CancellationToken>> = Mutex::new(None);
static RERUN: AtomicBool = AtomicBool::new(false);
static LAST: Mutex<Option<Progress>> = Mutex::new(None);
static BATCHES: Mutex<Option<HashMap<String, CancellationToken>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct Folder {
    pub path: String,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Folders {
    pub folders: Vec<Folder>,
    pub chosen: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ExportProgress<'a> {
    batch_id: &'a str,
    index: usize,
    path: &'a str,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<&'a ExportOutcome>,
}

fn library() -> Result<Library, String> {
    omniget_core::core::paths::app_data_dir()
        .map(|d| Library::new(d.join("photos")))
        .ok_or_else(|| "OmniGet couldn't find its data folder.".to_string())
}

fn default_folders(app: &AppHandle) -> Vec<PathBuf> {
    let downloads = crate::storage::config::load_settings(app)
        .download
        .default_output_dir;
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in [Some(downloads), dirs::picture_dir()].into_iter().flatten() {
        if dir.is_dir() && !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

fn roots(app: &AppHandle, lib: &Library) -> Result<(Vec<PathBuf>, bool), String> {
    Ok(match lib.folders()? {
        Some(chosen) => (chosen, true),
        None => (default_folders(app), false),
    })
}

fn folders_of(roots: (Vec<PathBuf>, bool)) -> Folders {
    Folders {
        folders: roots
            .0
            .into_iter()
            .map(|p| Folder {
                exists: p.is_dir(),
                path: p.to_string_lossy().into_owned(),
            })
            .collect(),
        chosen: roots.1,
    }
}

fn allow_thumbs(app: &AppHandle, lib: &Library) {
    let dir = lib.thumbs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = app.asset_protocol_scope().allow_directory(&dir, true);
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("The photo task stopped unexpectedly: {e}"))?
}

fn format_for(lib: &Library, path: &str) -> Result<String, String> {
    lib.format_of(path)?
        .ok_or_else(|| "This photo is not in the library anymore.".to_string())
}

#[tauri::command]
pub async fn photos_folders(app: AppHandle) -> Result<Folders, String> {
    let lib = library()?;
    Ok(folders_of(roots(&app, &lib)?))
}

#[tauri::command]
pub async fn photos_set_folders(app: AppHandle, folders: Vec<String>) -> Result<Folders, String> {
    let lib = library()?;
    let mut clean: Vec<PathBuf> = Vec::new();
    for f in folders.iter().filter(|f| !f.trim().is_empty()) {
        let p = PathBuf::from(f);
        if !p.is_absolute() {
            return Err("Choose a folder with its full path.".to_string());
        }
        if !clean.contains(&p) {
            clean.push(p);
        }
    }
    lib.set_folders(&clean)?;
    if let Ok(guard) = INDEX_TOKEN.lock() {
        if let Some(token) = guard.as_ref() {
            RERUN.store(true, Ordering::SeqCst);
            token.cancel();
        }
    }
    Ok(folders_of(roots(&app, &lib)?))
}

#[tauri::command]
pub fn photos_index_status() -> Progress {
    LAST.lock()
        .ok()
        .and_then(|l| l.clone())
        .unwrap_or_else(Progress::idle)
}

#[tauri::command]
pub async fn photos_index_start(app: AppHandle) -> Result<Progress, String> {
    let lib = library()?;
    let (mut folders, _) = roots(&app, &lib)?;
    let mut token = {
        let mut guard = INDEX_TOKEN.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Ok(photos_index_status());
        }
        RERUN.store(false, Ordering::SeqCst);
        let token = CancellationToken::new();
        *guard = Some(token.clone());
        token
    };
    allow_thumbs(&app, &lib);
    let first = Progress {
        stage: Stage::Scanning,
        ..Progress::idle()
    };
    if let Ok(mut last) = LAST.lock() {
        *last = Some(first.clone());
    }
    let emitter = app.clone();
    tokio::task::spawn_blocking(move || {
        let sent = Mutex::new(Instant::now() - REPORT_EVERY);
        let report = |p: &Progress| {
            if let Ok(mut last) = LAST.lock() {
                *last = Some(p.clone());
            }
            let due = sent
                .lock()
                .map(|mut s| {
                    let due = !p.running() || s.elapsed() >= REPORT_EVERY;
                    if due {
                        *s = Instant::now();
                    }
                    due
                })
                .unwrap_or(true);
            if due {
                let _ = emitter.emit(INDEX_EVENT, p);
            }
        };
        loop {
            index::run(&lib, &folders, &token, &report);
            let Ok(mut guard) = INDEX_TOKEN.lock() else {
                break;
            };
            let again = RERUN.swap(false, Ordering::SeqCst);
            let next = again.then(|| roots(&emitter, &lib).ok()).flatten();
            match next {
                Some((next_folders, _)) => {
                    token = CancellationToken::new();
                    *guard = Some(token.clone());
                    folders = next_folders;
                }
                None => {
                    *guard = None;
                    break;
                }
            }
        }
    });
    Ok(first)
}

#[tauri::command]
pub fn photos_index_cancel() {
    if let Ok(guard) = INDEX_TOKEN.lock() {
        if let Some(token) = guard.as_ref() {
            token.cancel();
        }
    }
}

#[tauri::command]
pub async fn photos_list(app: AppHandle) -> Result<Vec<Photo>, String> {
    let lib = library()?;
    allow_thumbs(&app, &lib);
    blocking(move || lib.list()).await
}

#[tauri::command]
pub async fn photos_duplicates(max_distance: Option<u32>) -> Result<Vec<DupGroup>, String> {
    let lib = library()?;
    blocking(move || {
        let mut items = lib.hashed()?;
        similar::fill_content(&lib, &mut items)?;
        Ok(similar::group(
            &items,
            max_distance.unwrap_or(similar::DEFAULT_DISTANCE),
        ))
    })
    .await
}

#[tauri::command]
pub async fn photos_remove(paths: Vec<String>, mode: RemoveMode) -> Result<FileReport, String> {
    let lib = library()?;
    blocking(move || Ok(files::remove(&lib, &paths, mode))).await
}

#[tauri::command]
pub async fn photos_move(
    app: AppHandle,
    paths: Vec<String>,
    dest: String,
) -> Result<FileReport, String> {
    let lib = library()?;
    let (folders, _) = roots(&app, &lib)?;
    blocking(move || Ok(files::move_to(&lib, &paths, Path::new(&dest), &folders))).await
}

#[tauri::command]
pub async fn photos_edits_get(path: String) -> Result<Option<Edits>, String> {
    let lib = library()?;
    blocking(move || lib.edits(&path)).await
}

#[tauri::command]
pub async fn photos_edits_set(path: String, edits: Option<Edits>) -> Result<(), String> {
    let lib = library()?;
    blocking(move || {
        format_for(&lib, &path)?;
        lib.set_edits(&path, edits.as_ref())
    })
    .await
}

#[tauri::command]
pub async fn photos_preview(path: String, edits: Edits) -> Result<Preview, String> {
    let lib = library()?;
    blocking(move || {
        let format = format_for(&lib, &path)?;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            preview::render(Path::new(&path), &format, &edits)
        }))
        .unwrap_or_else(|_| {
            Err("This photo could not be opened. It may be damaged or use a variant OmniGet can't open yet.".to_string())
        })
    })
    .await
}

#[tauri::command]
pub fn photos_release() {
    preview::release();
}

fn register(batch_id: &str) -> CancellationToken {
    let token = CancellationToken::new();
    if let Ok(mut map) = BATCHES.lock() {
        map.get_or_insert_with(HashMap::new)
            .insert(batch_id.to_string(), token.clone());
    }
    token
}

fn unregister(batch_id: &str) {
    if let Ok(mut map) = BATCHES.lock() {
        if let Some(m) = map.as_mut() {
            m.remove(batch_id);
        }
    }
}

fn emit_export(
    app: &AppHandle,
    batch_id: &str,
    index: usize,
    path: &str,
    state: &'static str,
    outcome: Option<&ExportOutcome>,
) {
    let _ = app.emit(
        EXPORT_EVENT,
        ExportProgress {
            batch_id,
            index,
            path,
            state,
            outcome,
        },
    );
}

#[tauri::command]
pub async fn photos_export(
    app: AppHandle,
    batch_id: String,
    paths: Vec<String>,
    options: ExportOptions,
) -> Result<Vec<ExportOutcome>, String> {
    export::validate(&options)?;
    let lib = library()?;
    let token = register(&batch_id);
    let taken = Arc::new(Mutex::new(HashSet::<PathBuf>::new()));
    for (i, p) in paths.iter().enumerate() {
        emit_export(&app, &batch_id, i, p, "queued", None);
    }
    let jobs = paths.into_iter().enumerate().map(|(i, path)| {
        let app = app.clone();
        let lib = lib.clone();
        let token = token.clone();
        let taken = taken.clone();
        let options = options.clone();
        let batch_id = batch_id.clone();
        async move {
            let outcome = if token.is_cancelled() {
                ExportOutcome::failed(&path, "Cancelled.")
            } else {
                emit_export(&app, &batch_id, i, &path, "running", None);
                let job_path = path.clone();
                blocking(move || {
                    let format = format_for(&lib, &job_path)?;
                    let edits = if options.apply_edits {
                        lib.edits(&job_path)?
                    } else {
                        None
                    };
                    let input = Path::new(&job_path);
                    let bytes = export::render_safely(input, &format, edits.as_ref(), &options)?;
                    let mut taken = taken.lock().map_err(|e| e.to_string())?;
                    Ok(export::save(input, &bytes, &options, &mut taken))
                })
                .await
                .unwrap_or_else(|e| ExportOutcome::failed(&path, e))
            };
            let state = if outcome.ok { "done" } else { "failed" };
            emit_export(&app, &batch_id, i, &path, state, Some(&outcome));
            outcome
        }
    });
    let outcomes = futures::stream::iter(jobs)
        .buffered(EXPORT_PARALLEL)
        .collect()
        .await;
    unregister(&batch_id);
    Ok(outcomes)
}

#[tauri::command]
pub fn photos_export_cancel(batch_id: String) {
    if let Ok(map) = BATCHES.lock() {
        if let Some(token) = map.as_ref().and_then(|m| m.get(&batch_id)) {
            token.cancel();
        }
    }
}
