use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use once_cell::sync::Lazy;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use omniget_core::core::music::tags;
use omniget_core::core::stems::audio::{StemFormat, StemTags};
use omniget_core::core::stems::separate::{Cancelled, Control, OrtSeparator, SAMPLE_RATE};
use omniget_core::core::stems::{models, sets, Output};

use super::{db, scan};

const PROGRESS_EVENT: &str = "music-stems-progress";
const MODEL_EVENT: &str = "music-stems-model-progress";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Preparing,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct StemOut {
    pub stem: String,
    pub path: String,
    pub track_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: u64,
    pub track_id: Option<i64>,
    pub source: String,
    pub title: String,
    pub outputs: Vec<StemOut>,
    pub state: JobState,
    pub progress: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Labels {
    #[serde(default)]
    pub vocals: String,
    #[serde(default)]
    pub instrumental: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StemsRequest {
    #[serde(default)]
    pub track_ids: Vec<i64>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub format: StemFormat,
    #[serde(default)]
    pub out_dir: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub labels: Labels,
}

#[derive(Debug, Clone, Serialize)]
pub struct StemSetView {
    pub source: String,
    pub source_id: Option<i64>,
    pub vocals: String,
    pub vocals_id: Option<i64>,
    pub instrumental: String,
    pub instrumental_id: Option<i64>,
    pub model: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
struct ModelProgress {
    id: String,
    stage: String,
    done: u64,
    total: Option<u64>,
}

struct Registry {
    jobs: HashMap<u64, Job>,
    tokens: HashMap<u64, CancellationToken>,
    next: u64,
}

static REG: Lazy<Mutex<Registry>> = Lazy::new(|| {
    Mutex::new(Registry {
        jobs: HashMap::new(),
        tokens: HashMap::new(),
        next: 1,
    })
});
static SLOTS: Lazy<Arc<Semaphore>> = Lazy::new(|| Arc::new(Semaphore::new(1)));
static INSTALL: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static SETS: Lazy<std::sync::Mutex<()>> = Lazy::new(|| std::sync::Mutex::new(()));

async fn update(app: &AppHandle, id: u64, f: impl FnOnce(&mut Job)) {
    let snapshot = {
        let mut r = REG.lock().await;
        let Some(j) = r.jobs.get_mut(&id) else { return };
        f(j);
        j.clone()
    };
    let _ = app.emit(PROGRESS_EVENT, snapshot);
}

fn label(given: &str, fallback: &str) -> String {
    let t = given.trim();
    if t.is_empty() {
        fallback.to_string()
    } else {
        t.to_string()
    }
}

fn track_id_for(path: &str) -> Option<i64> {
    db::with(|c| {
        c.query_row(
            "SELECT id FROM tracks WHERE path = ?1",
            params![path],
            |r| r.get(0),
        )
        .optional()
    })
    .ok()
    .flatten()
}

fn sets_file() -> Option<PathBuf> {
    db::music_dir().map(|d| sets::index_file(&d))
}

fn read_sets() -> Vec<sets::StemSet> {
    let _g = SETS.lock();
    let Some(file) = sets_file() else {
        return Vec::new();
    };
    let mut all = sets::load(&file);
    if sets::prune(&mut all) {
        let _ = sets::save(&file, &all);
    }
    all
}

fn remember(set: sets::StemSet) {
    let _g = SETS.lock();
    let Some(file) = sets_file() else { return };
    let mut all = sets::load(&file);
    sets::upsert(&mut all, set);
    if let Err(e) = sets::save(&file, &all) {
        tracing::warn!("[stems] could not save the stems index: {e}");
    }
}

struct Source {
    path: PathBuf,
    track_id: Option<i64>,
    title: String,
    tags: StemTags,
    duration_s: Option<f64>,
}

fn source_from_track(t: &db::TrackRow) -> Source {
    Source {
        path: PathBuf::from(&t.path),
        track_id: Some(t.id),
        title: if t.title.is_empty() {
            t.path.clone()
        } else {
            t.title.clone()
        },
        tags: StemTags {
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            album_artist: t.album_artist.clone(),
            genre: t.genre.clone(),
            year: t.year.and_then(|y| u32::try_from(y).ok()),
        },
        duration_s: (t.duration_ms > 0).then(|| t.duration_ms as f64 / 1000.0),
    }
}

fn source_from_path(p: &str) -> Source {
    let path = PathBuf::from(p);
    if let Some(id) = track_id_for(p) {
        if let Ok(t) = db::track(id) {
            return source_from_track(&t);
        }
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string());
    let meta = tags::read(&path).map(|(m, _)| m).ok();
    let title = meta
        .as_ref()
        .map(|m| m.title.clone())
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| stem.clone());
    Source {
        track_id: None,
        tags: StemTags {
            title: title.clone(),
            artist: meta.as_ref().map(|m| m.artist.clone()).unwrap_or_default(),
            album: meta.as_ref().map(|m| m.album.clone()).unwrap_or_default(),
            album_artist: meta
                .as_ref()
                .map(|m| m.album_artist.clone())
                .unwrap_or_default(),
            genre: meta.as_ref().map(|m| m.genre.clone()).unwrap_or_default(),
            year: meta.as_ref().and_then(|m| m.year),
        },
        duration_s: meta
            .as_ref()
            .map(|m| m.duration_ms as f64 / 1000.0)
            .filter(|d| *d > 0.0),
        title,
        path,
    }
}

struct Naming {
    out_dir: Option<PathBuf>,
    vocals: String,
    instrumental: String,
    format: StemFormat,
}

fn plan_outputs(spec: &models::ModelSpec, src: &Source, naming: &Naming) -> Vec<Output> {
    spec.files
        .iter()
        .map(|f| {
            let l = if f.stem == "vocals" {
                &naming.vocals
            } else {
                &naming.instrumental
            };
            let mut t = src.tags.clone();
            t.title = format!("{} ({l})", src.title);
            Output {
                path: omniget_core::core::stems::output_path(
                    &src.path,
                    naming.out_dir.as_deref(),
                    l,
                    naming.format,
                ),
                tags: t,
            }
        })
        .collect()
}

pub async fn start(app: AppHandle, req: StemsRequest) -> Result<Vec<Job>, String> {
    let spec = models::pick(req.model.as_deref()).map_err(|e| e.to_string())?;
    let mut sources: Vec<Source> = db::tracks_by_ids(&req.track_ids)?
        .iter()
        .map(source_from_track)
        .collect();
    for p in req.paths.iter().filter(|p| !p.trim().is_empty()) {
        let path = Path::new(p);
        if !path.is_absolute() || !path.is_file() {
            return Err(format!("file not found: {p}"));
        }
        sources.push(source_from_path(p));
    }
    if sources.is_empty() {
        return Err("pick at least one song or file".into());
    }
    let out_dir = Some(req.out_dir.trim())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    if let Some(d) = &out_dir {
        if !d.is_absolute() || d.is_file() {
            return Err(format!("invalid folder: {}", d.display()));
        }
    }
    let naming = Arc::new(Naming {
        out_dir,
        vocals: label(&req.labels.vocals, "Vocals"),
        instrumental: label(&req.labels.instrumental, "Instrumental"),
        format: req.format,
    });
    let mut created = Vec::new();
    for src in sources {
        let outputs = plan_outputs(spec, &src, &naming);
        let job = {
            let mut r = REG.lock().await;
            let id = r.next;
            r.next += 1;
            let job = Job {
                id,
                track_id: src.track_id,
                source: src.path.to_string_lossy().to_string(),
                title: src.title.clone(),
                outputs: spec
                    .files
                    .iter()
                    .zip(&outputs)
                    .map(|(f, o)| StemOut {
                        stem: f.stem.to_string(),
                        path: o.path.to_string_lossy().to_string(),
                        track_id: None,
                    })
                    .collect(),
                state: JobState::Queued,
                progress: 0.0,
                error: None,
            };
            r.jobs.insert(id, job.clone());
            r.tokens.insert(id, CancellationToken::new());
            job
        };
        created.push(job.clone());
        let app = app.clone();
        let naming = naming.clone();
        tauri::async_runtime::spawn(async move {
            run_job(app, job.id, spec, src, naming).await;
        });
    }
    Ok(created)
}

async fn ensure_model(app: &AppHandle, spec: &'static models::ModelSpec) -> Result<(), String> {
    let _g = INSTALL.lock().await;
    if models::is_downloaded(spec) && omniget_core::core::onnxrt::is_installed() {
        return Ok(());
    }
    let app = app.clone();
    let id = spec.id.to_string();
    let progress: omniget_core::core::tools::ProgressFn = Arc::new(move |p| {
        let _ = app.emit(
            MODEL_EVENT,
            ModelProgress {
                id: id.clone(),
                stage: p.stage,
                done: p.done,
                total: p.total,
            },
        );
    });
    models::install(spec, &progress)
        .await
        .map(|_| ())
        .map_err(|e| format!("could not download the separation model: {e}"))
}

fn finish_outputs(
    src: &Source,
    outputs: &[Output],
    spec: &models::ModelSpec,
    lyrics: Option<&str>,
) -> Vec<Option<i64>> {
    let cover = tags::read(&src.path).ok().and_then(|(_, c)| c);
    let mut ids = Vec::new();
    for (f, o) in spec.files.iter().zip(outputs) {
        if let Some(c) = &cover {
            let _ = tags::write(
                &o.path,
                &tags::TagPatch::default(),
                tags::CoverChange::Set(c.data.clone()),
            );
        }
        if f.stem != "vocals" {
            if let Some(l) = lyrics {
                let lrc = o.path.with_extension("lrc");
                if !lrc.exists() {
                    let _ = std::fs::write(&lrc, l);
                }
            }
        }
        ids.push(scan::index_file(&o.path).ok());
    }
    ids
}

async fn run_job(
    app: AppHandle,
    id: u64,
    spec: &'static models::ModelSpec,
    src: Source,
    naming: Arc<Naming>,
) {
    let token = { REG.lock().await.tokens.get(&id).cloned() };
    let Some(token) = token else { return };
    let permit = tokio::select! {
        p = SLOTS.clone().acquire_owned() => p.ok(),
        _ = token.cancelled() => None,
    };
    if permit.is_none() {
        update(&app, id, |j| j.state = JobState::Cancelled).await;
        return;
    }
    update(&app, id, |j| j.state = JobState::Preparing).await;
    let ready = tokio::select! {
        r = ensure_model(&app, spec) => Some(r),
        _ = token.cancelled() => None,
    };
    match ready {
        None => {
            update(&app, id, |j| j.state = JobState::Cancelled).await;
            return;
        }
        Some(Err(e)) => {
            update(&app, id, |j| {
                j.state = JobState::Failed;
                j.error = Some(e);
            })
            .await;
            return;
        }
        Some(Ok(())) => {}
    }
    if token.is_cancelled() {
        update(&app, id, |j| j.state = JobState::Cancelled).await;
        return;
    }
    let outputs = plan_outputs(spec, &src, &naming);
    let format = naming.format;
    update(&app, id, |j| {
        for (o, planned) in j.outputs.iter_mut().zip(&outputs) {
            o.path = planned.path.to_string_lossy().to_string();
        }
    })
    .await;
    let duration = match src.duration_s {
        Some(d) => Some(d),
        None => omniget_core::core::ffmpeg::probe(&src.path)
            .await
            .ok()
            .map(|p| p.duration_seconds)
            .filter(|d| *d > 0.0),
    };
    let total = duration.map(|d| d * SAMPLE_RATE as f64);
    let lyrics = match src.track_id {
        Some(t) => super::music_lyrics_get(t)
            .await
            .ok()
            .filter(|l| l.source != "none" && !l.raw.trim().is_empty())
            .map(|l| l.raw),
        None => None,
    };
    update(&app, id, |j| j.state = JobState::Running).await;

    let app2 = app.clone();
    let tok = token.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut sep = OrtSeparator::load(spec)?;
        let last = std::sync::Mutex::new(std::time::Instant::now());
        let handle = tauri::async_runtime::handle();
        let progress = |done: u64| {
            let Some(total) = total else { return };
            let Ok(mut l) = last.lock() else { return };
            if l.elapsed().as_millis() < 250 {
                return;
            }
            *l = std::time::Instant::now();
            let p = (done as f64 / total).clamp(0.0, 0.99);
            let app = app2.clone();
            handle.spawn(async move {
                update(&app, id, |j| {
                    if j.state == JobState::Running {
                        j.progress = p;
                    }
                })
                .await
            });
        };
        let cancelled = || tok.is_cancelled();
        let ctl = Control {
            cancelled: &cancelled,
            progress: &progress,
        };
        omniget_core::core::stems::separate_file(&src.path, &mut sep, &outputs, format, &ctl)?;
        let ids = finish_outputs(&src, &outputs, spec, lyrics.as_deref());
        Ok::<_, anyhow::Error>((src, outputs, ids))
    })
    .await;

    match result {
        Ok(Ok((src, outputs, ids))) => {
            let path_of = |stem: &str| {
                spec.files
                    .iter()
                    .zip(&outputs)
                    .find(|(f, _)| f.stem == stem)
                    .map(|(_, o)| o.path.to_string_lossy().to_string())
                    .unwrap_or_default()
            };
            remember(sets::StemSet {
                source: src.path.to_string_lossy().to_string(),
                vocals: path_of("vocals"),
                instrumental: path_of("instrumental"),
                model: spec.id.to_string(),
                created_at: db::now_ms(),
            });
            update(&app, id, |j| {
                for (o, tid) in j.outputs.iter_mut().zip(ids) {
                    o.track_id = tid;
                }
                j.state = JobState::Done;
                j.progress = 1.0;
            })
            .await;
        }
        Ok(Err(e)) if e.downcast_ref::<Cancelled>().is_some() => {
            update(&app, id, |j| j.state = JobState::Cancelled).await;
        }
        Ok(Err(e)) => {
            update(&app, id, |j| {
                j.state = JobState::Failed;
                j.error = Some(format!("{e:#}"));
            })
            .await;
        }
        Err(e) => {
            update(&app, id, |j| {
                j.state = JobState::Failed;
                j.error = Some(e.to_string());
            })
            .await;
        }
    }
}

#[tauri::command]
pub async fn music_stems_status() -> models::Status {
    models::status()
}

#[tauri::command]
pub async fn music_stems_install_model(
    app: AppHandle,
    model: Option<String>,
) -> Result<models::Status, String> {
    let spec = models::pick(model.as_deref()).map_err(|e| e.to_string())?;
    ensure_model(&app, spec).await?;
    Ok(models::status())
}

#[tauri::command]
pub async fn music_stems_remove_model(model: Option<String>) -> Result<models::Status, String> {
    let spec = models::pick(model.as_deref()).map_err(|e| e.to_string())?;
    let _g = INSTALL.lock().await;
    models::remove(spec).map_err(|e| e.to_string())?;
    Ok(models::status())
}

#[tauri::command]
pub async fn music_stems_start(app: AppHandle, request: StemsRequest) -> Result<Vec<Job>, String> {
    start(app, request).await
}

#[tauri::command]
pub async fn music_stems_cancel(id: Option<u64>) {
    let r = REG.lock().await;
    for (jid, t) in &r.tokens {
        if id.map(|i| i == *jid).unwrap_or(true) {
            t.cancel();
        }
    }
}

#[tauri::command]
pub async fn music_stems_jobs() -> Vec<Job> {
    let r = REG.lock().await;
    let mut v: Vec<Job> = r.jobs.values().cloned().collect();
    v.sort_by_key(|j| j.id);
    v
}

#[tauri::command]
pub async fn music_stems_clear() {
    let mut r = REG.lock().await;
    let done: Vec<u64> = r
        .jobs
        .values()
        .filter(|j| {
            matches!(
                j.state,
                JobState::Done | JobState::Failed | JobState::Cancelled
            )
        })
        .map(|j| j.id)
        .collect();
    for id in done {
        r.jobs.remove(&id);
        r.tokens.remove(&id);
    }
}

#[tauri::command]
pub async fn music_stems_sets() -> Result<Vec<StemSetView>, String> {
    tokio::task::spawn_blocking(|| {
        read_sets()
            .into_iter()
            .rev()
            .map(|s| StemSetView {
                source_id: track_id_for(&s.source),
                vocals_id: track_id_for(&s.vocals),
                instrumental_id: track_id_for(&s.instrumental),
                source: s.source,
                vocals: s.vocals,
                instrumental: s.instrumental,
                model: s.model,
                created_at: s.created_at,
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}
