use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

use omniget_core::core::asr;
use omniget_core::core::studio::{self, job, Probe, RenderOutcome, RenderRequest, Tools};

static JOBS: Mutex<Option<HashMap<String, CancellationToken>>> = Mutex::new(None);

const TEXT_LIMIT: u64 = 8 * 1024 * 1024;

fn subtitle_only(path: &str) -> Result<(), String> {
    if studio::is_subtitle_path(path) {
        Ok(())
    } else {
        Err("Studio only reads and writes subtitle files (.srt, .vtt or .ass).".to_string())
    }
}

fn register(job_id: &str) -> CancellationToken {
    let token = CancellationToken::new();
    if let Ok(mut map) = JOBS.lock() {
        map.get_or_insert_with(HashMap::new)
            .insert(job_id.to_string(), token.clone());
    }
    token
}

fn unregister(job_id: &str) {
    if let Ok(mut map) = JOBS.lock() {
        if let Some(m) = map.as_mut() {
            m.remove(job_id);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct RenderProgress<'a> {
    job_id: &'a str,
    percent: f64,
    stage: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct TranscribeProgress<'a> {
    job_id: &'a str,
    percent: f64,
}

#[derive(Debug, Clone, Serialize)]
struct AsrProgress {
    model: String,
    id: String,
    stage: String,
    done: u64,
    total: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StudioSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StudioTranscript {
    pub lang: String,
    pub model: String,
    pub segments: Vec<StudioSegment>,
}

#[tauri::command]
pub async fn studio_tools() -> Tools {
    omniget_core::core::ffmpeg::reset_ffmpeg_available_cache();
    if !omniget_core::core::ffmpeg::is_ffmpeg_available().await {
        return Tools::default();
    }
    studio::tools().await
}

#[tauri::command]
pub async fn studio_probe(app: AppHandle, path: String) -> Result<Probe, String> {
    let probe = studio::probe(Path::new(&path)).await?;
    let _ = app.asset_protocol_scope().allow_file(&path);
    Ok(probe)
}

#[tauri::command]
pub fn studio_asr_status() -> asr::Status {
    asr::status()
}

#[tauri::command]
pub async fn studio_asr_install(
    app: AppHandle,
    model: Option<String>,
) -> Result<asr::Status, String> {
    let status = asr::status();
    if !status.runtime_installed && !status.runtime_downloadable {
        return Err(
            "Local transcription needs the ONNX Runtime, which has no build for this system."
                .to_string(),
        );
    }
    let id = asr::models::pick(model.as_deref())
        .map(|m| m.id.to_string())
        .map_err(|e| e.to_string())?;
    let progress: omniget_core::core::tools::ProgressFn = Arc::new(move |p| {
        let _ = app.emit(
            "studio-asr-progress",
            AsrProgress {
                model: id.clone(),
                id: p.id,
                stage: p.stage,
                done: p.done,
                total: p.total,
            },
        );
    });
    asr::install(model.as_deref(), &progress)
        .await
        .map_err(|e| {
            format!(
                "Could not download the Whisper model: {e}. Check your connection and try again."
            )
        })
}

#[tauri::command]
pub async fn studio_transcribe(
    app: AppHandle,
    job_id: String,
    path: String,
    lang: Option<String>,
    model: Option<String>,
    duration_ms: Option<u64>,
) -> Result<StudioTranscript, String> {
    let lang = lang.filter(|l| !l.trim().is_empty() && l != "auto");
    let emit_app = app.clone();
    let emit_id = job_id.clone();
    let transcript = asr::transcribe_input(
        std::ffi::OsStr::new(&path),
        duration_ms.filter(|d| *d > 0),
        lang.as_deref(),
        model.as_deref(),
        move |fraction| {
            let _ = emit_app.emit(
                "studio-transcribe-progress",
                TranscribeProgress {
                    job_id: &emit_id,
                    percent: (f64::from(fraction) * 100.0).clamp(0.0, 100.0).floor(),
                },
            );
        },
    )
    .await
    .map_err(|e| {
        let text = e.to_string();
        if text.contains("no audio") {
            "This video has no sound to transcribe. Write the captions by hand or import a subtitle file.".to_string()
        } else {
            format!("Transcription failed: {text}")
        }
    })?;
    Ok(StudioTranscript {
        lang: transcript.lang,
        model: transcript.model,
        segments: transcript
            .segments
            .into_iter()
            .map(|s| StudioSegment {
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                text: s.text,
            })
            .collect(),
    })
}

#[tauri::command]
pub async fn studio_render(
    app: AppHandle,
    job_id: String,
    request: RenderRequest,
) -> Result<RenderOutcome, String> {
    let token = register(&job_id);
    let last = Mutex::new(-1.0f64);
    let report = |fraction: f64, stage: &'static str| {
        let percent = (fraction * 100.0).clamp(0.0, 100.0).floor();
        let changed = last
            .lock()
            .map(|mut l| {
                if percent > *l || stage != "rendering" {
                    *l = percent;
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if changed {
            let _ = app.emit(
                "studio-render-progress",
                RenderProgress {
                    job_id: &job_id,
                    percent,
                    stage,
                },
            );
        }
    };
    let result = job::render(&request, &token, &report).await;
    unregister(&job_id);
    if let Ok(outcome) = &result {
        let _ = app.asset_protocol_scope().allow_file(&outcome.output);
    }
    result
}

#[tauri::command]
pub fn studio_cancel(job_id: String) {
    if let Ok(map) = JOBS.lock() {
        if let Some(token) = map.as_ref().and_then(|m| m.get(&job_id)) {
            token.cancel();
        }
    }
}

#[tauri::command]
pub async fn studio_save_text(path: String, contents: String) -> Result<(), String> {
    subtitle_only(&path)?;
    tokio::fs::write(&path, contents)
        .await
        .map_err(|e| format!("Could not save the file: {e}."))
}

#[tauri::command]
pub async fn studio_read_text(path: String) -> Result<String, String> {
    subtitle_only(&path)?;
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|_| "The file could not be opened.".to_string())?;
    if meta.len() > TEXT_LIMIT {
        return Err(
            "This subtitle file is larger than 8 MB, so it is probably not a subtitle file."
                .to_string(),
        );
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("Could not read the file: {e}."))?;
    Ok(String::from_utf8_lossy(&bytes)
        .trim_start_matches('\u{feff}')
        .to_string())
}
