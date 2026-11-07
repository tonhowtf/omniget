use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

use omniget_core::core::capture::errors::{self, code_message};
use omniget_core::core::capture::history::{self, EntryKind, HistoryEntry};
use omniget_core::core::capture::probe::{self, ProbeResult};
use omniget_core::core::capture::runner::{self, RecordOutcome, Recorder};
use omniget_core::core::capture::{
    args, devices, naming, ExportMode, ExportRequest, ExportResult, Platform, RecordOptions,
    Segment, Sources,
};

use crate::storage::config;

const RECORDING_EVENT: &str = "capture-recording";
const EXPORT_EVENT: &str = "capture-export-progress";

struct Active {
    id: String,
    path: PathBuf,
    started_at: i64,
    recorder: Arc<Recorder>,
}

static ACTIVE: tokio::sync::Mutex<Option<Active>> = tokio::sync::Mutex::const_new(None);
static ENDED: Mutex<Option<RecordOutcome>> = Mutex::new(None);
static JOBS: Mutex<Option<HashMap<String, CancellationToken>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct RecordingInfo {
    pub id: String,
    pub path: String,
    pub started_at: i64,
}

#[derive(Debug, Clone, Serialize)]
struct RecordingEvent<'a> {
    id: &'a str,
    state: &'static str,
    outcome: &'a RecordOutcome,
}

#[derive(Debug, Clone, Serialize)]
struct ExportEvent<'a> {
    job_id: &'a str,
    percent: f64,
    step: usize,
    steps: usize,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

fn history_file() -> Result<PathBuf, String> {
    history::default_file().ok_or_else(|| "Could not find the app data folder.".to_string())
}

fn record_history(entry: HistoryEntry) {
    if let Ok(file) = history_file() {
        let _ = history::add(&file, entry);
    }
}

fn default_dir(app: &AppHandle) -> PathBuf {
    config::load_settings(app).download.default_output_dir
}

fn resolve_out_dir(
    app: &AppHandle,
    chosen: Option<&str>,
    fallback: Option<&Path>,
) -> Result<PathBuf, String> {
    let dir = chosen
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| fallback.map(Path::to_path_buf))
        .unwrap_or_else(|| default_dir(app));
    std::fs::create_dir_all(&dir)
        .map_err(|e| code_message(errors::OUT_DIR, &format!("{}: {e}", dir.to_string_lossy())))?;
    Ok(dir)
}

fn finalize(id: &str, outcome: &RecordOutcome, app: &AppHandle) {
    if outcome.ok {
        record_history(HistoryEntry::new(
            EntryKind::Recording,
            Path::new(&outcome.path),
            Some(outcome.duration).filter(|d| *d > 0.0),
            None,
        ));
    }
    let _ = app.emit(
        RECORDING_EVENT,
        RecordingEvent {
            id,
            state: if outcome.ok { "done" } else { "failed" },
            outcome,
        },
    );
}

#[tauri::command]
pub async fn capture_sources() -> Sources {
    omniget_core::core::ffmpeg::reset_ffmpeg_available_cache();
    devices::list_sources().await
}

#[tauri::command]
pub fn capture_default_dir(app: AppHandle) -> String {
    default_dir(&app).to_string_lossy().to_string()
}

#[tauri::command]
pub async fn capture_status() -> Option<RecordingInfo> {
    let guard = ACTIVE.lock().await;
    guard.as_ref().map(|a| RecordingInfo {
        id: a.id.clone(),
        path: a.path.to_string_lossy().to_string(),
        started_at: a.started_at,
    })
}

#[tauri::command]
pub async fn capture_start(
    app: AppHandle,
    options: RecordOptions,
) -> Result<RecordingInfo, String> {
    let mut guard = ACTIVE.lock().await;
    if guard.as_ref().is_some_and(|a| !a.recorder.finished()) {
        return Err(errors::ALREADY_RECORDING.to_string());
    }
    let dir = resolve_out_dir(&app, options.out_dir.as_deref(), None)?;
    let name = naming::recording_name(&chrono::Local::now().naive_local(), options.container.ext());
    let output = naming::unique_path(&dir, &name);
    let platform = Platform::current();
    let argv = args::build_record_args(&options, platform, &output)?;
    if let Ok(mut ended) = ENDED.lock() {
        *ended = None;
    }
    let with_mic = options.mic.as_deref().is_some_and(|m| !m.trim().is_empty());
    let recorder =
        Arc::new(runner::start_recording(argv, output.clone(), platform, with_mic).await?);
    let id = uuid::Uuid::new_v4().to_string();
    let started_at = chrono::Utc::now().timestamp_millis();
    *guard = Some(Active {
        id: id.clone(),
        path: output.clone(),
        started_at,
        recorder: recorder.clone(),
    });
    drop(guard);

    let watch_id = id.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = recorder.wait().await;
        if outcome.stopped_by_user {
            return;
        }
        let mut guard = ACTIVE.lock().await;
        if guard.as_ref().is_some_and(|a| a.id == watch_id) {
            *guard = None;
            if let Ok(mut ended) = ENDED.lock() {
                *ended = Some(outcome.clone());
            }
        }
        drop(guard);
        finalize(&watch_id, &outcome, &app);
    });

    Ok(RecordingInfo {
        id,
        path: output.to_string_lossy().to_string(),
        started_at,
    })
}

#[tauri::command]
pub async fn capture_stop(app: AppHandle) -> Result<RecordOutcome, String> {
    let active = ACTIVE.lock().await.take();
    let Some(active) = active else {
        if let Some(ended) = ENDED.lock().ok().and_then(|mut e| e.take()) {
            return Ok(ended);
        }
        return Err(errors::NOT_RECORDING.to_string());
    };
    let outcome = active.recorder.stop().await;
    if outcome.stopped_by_user {
        finalize(&active.id, &outcome, &app);
    }
    Ok(outcome)
}

#[tauri::command]
pub async fn capture_probe(app: AppHandle, path: String) -> Result<ProbeResult, String> {
    let p = PathBuf::from(&path);
    let result = probe::probe(&p).await?;
    let _ = app.asset_protocol_scope().allow_file(&p);
    Ok(result)
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

#[tauri::command]
pub fn capture_cancel(job_id: String) {
    if let Ok(map) = JOBS.lock() {
        if let Some(token) = map.as_ref().and_then(|m| m.get(&job_id)) {
            token.cancel();
        }
    }
}

struct Progress<'a> {
    app: &'a AppHandle,
    job_id: &'a str,
    total: f64,
    steps: usize,
    last: Mutex<f64>,
}

impl Progress<'_> {
    fn emit(&self, percent: f64, step: usize, state: &'static str, error: Option<&str>) {
        let _ = self.app.emit(
            EXPORT_EVENT,
            ExportEvent {
                job_id: self.job_id,
                percent,
                step,
                steps: self.steps,
                state,
                error,
            },
        );
    }

    fn running(&self, done_secs: f64, step: usize) {
        let percent = if self.total > 0.0 {
            (done_secs / self.total * 100.0).clamp(0.0, 99.0).floor()
        } else {
            0.0
        };
        let changed = self
            .last
            .lock()
            .map(|mut l| {
                if percent > *l {
                    *l = percent;
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if changed {
            self.emit(percent, step, "running", None);
        }
    }
}

async fn export_inner(
    input: &Path,
    segments: &[Segment],
    req: &ExportRequest,
    dir: &Path,
    token: &CancellationToken,
    progress: &Progress<'_>,
    created: &mut Vec<PathBuf>,
) -> Result<Vec<(PathBuf, f64)>, String> {
    let stem = naming::stem_of(input);
    let ext = naming::ext_of(input);
    let merge = req.mode == ExportMode::Merge && segments.len() > 1;
    let weight = if merge { 0.9 } else { 1.0 };
    let mut done = 0.0;
    if !merge {
        let mut outputs = Vec::new();
        for (i, seg) in segments.iter().enumerate() {
            let name = if segments.len() == 1 && req.mode == ExportMode::Merge {
                naming::merged_name(&stem, &ext)
            } else {
                naming::segment_name(&stem, seg.start, seg.end, &ext)
            };
            let out = naming::unique_path(dir, &name);
            created.push(out.clone());
            let argv = args::build_cut_args(input, seg, &out, req.precise, req.strip_audio);
            let base = done;
            runner::run_job(&argv, token, &|t| {
                progress.running(base + t.min(seg.duration()), i + 1)
            })
            .await?;
            done += seg.duration();
            outputs.push((out, seg.duration()));
        }
        return Ok(outputs);
    }
    let work = dir.join(format!(
        ".omniget-capture-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&work).map_err(|e| code_message(errors::OUT_DIR, &e.to_string()))?;
    let result = async {
        let mut parts = Vec::new();
        for (i, seg) in segments.iter().enumerate() {
            let part = work.join(format!("part-{:03}.{ext}", i + 1));
            let argv = args::build_cut_args(input, seg, &part, req.precise, req.strip_audio);
            let base = done;
            runner::run_job(&argv, token, &|t| {
                progress.running((base + t.min(seg.duration())) * weight, i + 1)
            })
            .await?;
            done += seg.duration();
            parts.push(part);
        }
        let list = work.join("list.txt");
        std::fs::write(&list, args::concat_list(&parts))
            .map_err(|e| code_message(errors::EXPORT_FAILED, &e.to_string()))?;
        let out = naming::unique_path(dir, &naming::merged_name(&stem, &ext));
        created.push(out.clone());
        let total = progress.total;
        runner::run_job(&args::build_concat_args(&list, &out), token, &|t| {
            progress.running(total * weight + t * (1.0 - weight), segments.len() + 1)
        })
        .await?;
        Ok(vec![(out, total)])
    }
    .await;
    let _ = std::fs::remove_dir_all(&work);
    result
}

#[tauri::command]
pub async fn capture_export(
    app: AppHandle,
    job_id: String,
    request: ExportRequest,
) -> Result<ExportResult, String> {
    let input = PathBuf::from(&request.path);
    if !input.is_file() {
        return Err(code_message(errors::NOT_FOUND, &request.path));
    }
    let segments = args::clean_segments(&request.segments, None);
    if segments.is_empty() {
        return Err(errors::NO_SEGMENTS.to_string());
    }
    let dir = resolve_out_dir(&app, request.out_dir.as_deref(), input.parent())?;
    let token = register(&job_id);
    let merge = request.mode == ExportMode::Merge && segments.len() > 1;
    let progress = Progress {
        app: &app,
        job_id: &job_id,
        total: segments.iter().map(Segment::duration).sum(),
        steps: segments.len() + usize::from(merge),
        last: Mutex::new(-1.0),
    };
    progress.emit(0.0, 1, "running", None);
    let mut created = Vec::new();
    let result = export_inner(
        &input,
        &segments,
        &request,
        &dir,
        &token,
        &progress,
        &mut created,
    )
    .await;
    unregister(&job_id);
    match result {
        Ok(outputs) => {
            let mut size = 0;
            for (out, secs) in &outputs {
                let entry = HistoryEntry::new(EntryKind::Export, out, Some(*secs), Some(&input));
                size += entry.size_bytes;
                record_history(entry);
            }
            progress.emit(100.0, progress.steps, "done", None);
            Ok(ExportResult {
                outputs: outputs
                    .iter()
                    .map(|(p, _)| p.to_string_lossy().to_string())
                    .collect(),
                size_bytes: size,
            })
        }
        Err(e) => {
            for p in &created {
                let _ = std::fs::remove_file(p);
            }
            let state = if e == errors::CANCELLED {
                "cancelled"
            } else {
                "failed"
            };
            progress.emit(0.0, progress.steps, state, Some(&e));
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn capture_snapshot(
    app: AppHandle,
    path: String,
    at: f64,
    out_dir: Option<String>,
) -> Result<String, String> {
    let input = PathBuf::from(&path);
    if !input.is_file() {
        return Err(code_message(errors::NOT_FOUND, &path));
    }
    let dir = resolve_out_dir(&app, out_dir.as_deref(), input.parent())?;
    let out = naming::unique_path(&dir, &naming::snapshot_name(&naming::stem_of(&input), at));
    let argv = args::build_snapshot_args(&input, at, &out);
    let token = CancellationToken::new();
    if let Err(e) = runner::run_job(&argv, &token, &|_| {}).await {
        let _ = std::fs::remove_file(&out);
        return Err(e);
    }
    record_history(HistoryEntry::new(
        EntryKind::Snapshot,
        &out,
        None,
        Some(&input),
    ));
    Ok(out.to_string_lossy().to_string())
}

#[tauri::command]
pub fn capture_history() -> Result<Vec<HistoryEntry>, String> {
    Ok(history::list(&history_file()?))
}

#[tauri::command]
pub fn capture_history_remove(id: String) -> Result<Vec<HistoryEntry>, String> {
    history::remove(&history_file()?, &id)
}

#[tauri::command]
pub fn capture_open_privacy(kind: String) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        let pane = if kind == "mic" {
            "Privacy_Microphone"
        } else {
            "Privacy_ScreenCapture"
        };
        omniget_core::core::process::std_command("open")
            .arg(format!(
                "x-apple.systempreferences:com.apple.preference.security?{pane}"
            ))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}
