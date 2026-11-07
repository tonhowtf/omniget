use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use image::codecs::jpeg::JpegEncoder;
use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter};

use omniget_core::core::pdf::{
    self, files, Control, ImageLayout, PageSpec, PdfError, PdfFailure, PdfInfo, Preset,
};

const PROGRESS_EVENT: &str = "pdf-progress";
const THUMB_SIDE: u32 = 320;

static JOBS: Mutex<Option<HashMap<String, Arc<AtomicBool>>>> = Mutex::new(None);

#[derive(Debug, Clone, Deserialize)]
pub struct OutputPlan {
    pub path: String,
    pub pages: Vec<PageSpec>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavedFile {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompressOutcome {
    pub path: Option<String>,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub images: usize,
}

#[derive(Debug, Clone, Serialize)]
struct Progress<'a> {
    job_id: &'a str,
    done: usize,
    total: usize,
}

struct Job {
    id: String,
    cancel: Arc<AtomicBool>,
}

impl Job {
    fn start(id: &str) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut map) = JOBS.lock() {
            map.get_or_insert_with(HashMap::new)
                .insert(id.to_string(), cancel.clone());
        }
        Self {
            id: id.to_string(),
            cancel,
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if let Ok(mut map) = JOBS.lock() {
            if let Some(m) = map.as_mut() {
                m.remove(&self.id);
            }
        }
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, PdfError> + Send + 'static,
) -> Result<T, PdfFailure> {
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result.map_err(PdfFailure::from),
        Err(e) => Err(PdfFailure {
            code: "io",
            message: e.to_string(),
        }),
    }
}

fn saved(path: PathBuf, size_bytes: u64) -> SavedFile {
    SavedFile {
        path: path.to_string_lossy().into_owned(),
        size_bytes,
    }
}

#[tauri::command]
pub async fn pdf_inspect(paths: Vec<String>) -> Vec<PdfInfo> {
    tokio::task::spawn_blocking(move || paths.iter().map(|p| pdf::inspect(Path::new(p))).collect())
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn pdf_bytes(path: String) -> Result<Response, PdfFailure> {
    let bytes = blocking(move || files::read_pdf_bytes(Path::new(&path))).await?;
    Ok(Response::new(bytes))
}

#[tauri::command]
pub async fn pdf_save(
    sources: Vec<String>,
    outputs: Vec<OutputPlan>,
    overwrite: bool,
) -> Result<Vec<SavedFile>, PdfFailure> {
    blocking(move || {
        let sources: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
        let outputs: Vec<(PathBuf, Vec<PageSpec>)> = outputs
            .into_iter()
            .map(|o| (PathBuf::from(o.path), o.pages))
            .collect();
        let written = pdf::write_outputs(&sources, &outputs, overwrite)?;
        Ok(written.into_iter().map(|(p, s)| saved(p, s)).collect())
    })
    .await
}

#[tauri::command]
pub async fn pdf_compress(
    app: AppHandle,
    job_id: String,
    path: String,
    preset: Preset,
    out_path: String,
) -> Result<CompressOutcome, PdfFailure> {
    let job = Job::start(&job_id);
    blocking(move || {
        let bytes = files::read_pdf_bytes(Path::new(&path))?;
        let before_bytes = bytes.len() as u64;
        let progress = |done: usize, total: usize| {
            let _ = app.emit(
                PROGRESS_EVENT,
                Progress {
                    job_id: &job.id,
                    done,
                    total,
                },
            );
        };
        let control = Control {
            cancel: &job.cancel,
            progress: &progress,
        };
        let (out, stats) = pdf::compress_bytes(bytes, preset, &control)?;
        let after_bytes = out.len() as u64;
        if after_bytes as f64 > before_bytes as f64 * 0.97 {
            return Ok(CompressOutcome {
                path: None,
                before_bytes,
                after_bytes: before_bytes,
                images: stats.images,
            });
        }
        let target = pdf::unique_path(Path::new(&out_path));
        files::write_atomic(&out, &target)?;
        Ok(CompressOutcome {
            path: Some(target.to_string_lossy().into_owned()),
            before_bytes,
            after_bytes,
            images: stats.images,
        })
    })
    .await
}

#[tauri::command]
pub async fn pdf_expand_images(paths: Vec<String>) -> Vec<String> {
    tokio::task::spawn_blocking(move || {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        pdf::expand_images(&paths)
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
pub async fn pdf_image_thumb(path: String) -> Result<Response, PdfFailure> {
    let bytes = blocking(move || {
        let failed = || PdfError::Image(path.clone());
        if !pdf::is_image(Path::new(&path)) {
            return Err(failed());
        }
        let img = image::ImageReader::open(&path)
            .map_err(|_| failed())?
            .with_guessed_format()
            .map_err(|_| failed())?;
        let mut decoder = img.into_decoder().map_err(|_| failed())?;
        let orientation = image::ImageDecoder::orientation(&mut decoder)
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut img = image::DynamicImage::from_decoder(decoder).map_err(|_| failed())?;
        img.apply_orientation(orientation);
        let thumb = img.thumbnail(THUMB_SIDE, THUMB_SIDE).to_rgb8();
        let mut out = Cursor::new(Vec::new());
        JpegEncoder::new_with_quality(&mut out, 80)
            .encode_image(&thumb)
            .map_err(|_| failed())?;
        Ok(out.into_inner())
    })
    .await?;
    Ok(Response::new(bytes))
}

#[tauri::command]
pub async fn pdf_from_images(
    app: AppHandle,
    job_id: String,
    paths: Vec<String>,
    layout: ImageLayout,
    out_path: String,
) -> Result<SavedFile, PdfFailure> {
    let job = Job::start(&job_id);
    blocking(move || {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let progress = |done: usize, total: usize| {
            let _ = app.emit(
                PROGRESS_EVENT,
                Progress {
                    job_id: &job.id,
                    done,
                    total,
                },
            );
        };
        let control = Control {
            cancel: &job.cancel,
            progress: &progress,
        };
        let doc = pdf::images_to_pdf(&paths, layout, &control)?;
        let (path, size) = pdf::save(&doc, Path::new(&out_path), false)?;
        Ok(saved(path, size))
    })
    .await
}

#[tauri::command]
pub fn pdf_cancel(job_id: String) {
    if let Ok(map) = JOBS.lock() {
        if let Some(flag) = map.as_ref().and_then(|m| m.get(&job_id)) {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
