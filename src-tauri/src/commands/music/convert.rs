use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use omniget_core::core::music::convert::{self, ConvertOptions, Preset};
use omniget_core::core::music::{tags, template};

use super::{db, scan};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: u64,
    pub track_id: i64,
    pub title: String,
    pub output: String,
    pub state: JobState,
    pub progress: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConvertRequest {
    pub track_ids: Vec<i64>,
    pub preset: Preset,
    #[serde(default)]
    pub options: ConvertOptions,
    #[serde(default)]
    pub out_dir: String,
    #[serde(default)]
    pub template: String,
    #[serde(default)]
    pub bake_replaygain: bool,
    #[serde(default)]
    pub add_to_library: bool,
    #[serde(default)]
    pub overwrite: bool,
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
static SLOTS: Lazy<Arc<Semaphore>> = Lazy::new(|| Arc::new(Semaphore::new(2)));

async fn update(app: &AppHandle, id: u64, f: impl FnOnce(&mut Job)) {
    let snapshot = {
        let mut r = REG.lock().await;
        let Some(j) = r.jobs.get_mut(&id) else { return };
        f(j);
        j.clone()
    };
    let _ = app.emit("music-convert-progress", snapshot);
}

fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    for i in 2..1000 {
        let p = dir.join(format!("{stem} ({i}).{ext}"));
        if !p.exists() {
            return p;
        }
    }
    path
}

fn output_for(src: &Path, req: &ConvertRequest, meta: &tags::TrackMeta) -> PathBuf {
    let dir = if req.out_dir.trim().is_empty() {
        src.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        PathBuf::from(req.out_dir.trim())
    };
    let rel = if req.template.trim().is_empty() {
        src.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "track".into())
    } else {
        template::render(&req.template, meta)
    };
    let mut out = dir;
    for part in rel.split('/') {
        out.push(part);
    }
    out.set_extension(req.preset.extension());
    out
}

pub async fn start(app: AppHandle, req: ConvertRequest) -> Result<Vec<Job>, String> {
    let tracks = db::tracks_by_ids(&req.track_ids)?;
    let mut created = Vec::new();
    for t in tracks {
        let src = PathBuf::from(&t.path);
        let meta = tags::read(&src).map(|(m, _)| m).unwrap_or_default();
        let mut out = output_for(&src, &req, &meta);
        if out == src {
            let stem = out
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            out.set_file_name(format!("{stem} (converted).{}", req.preset.extension()));
        }
        if !req.overwrite {
            out = unique(out);
        }
        let job = {
            let mut r = REG.lock().await;
            let id = r.next;
            r.next += 1;
            let job = Job {
                id,
                track_id: t.id,
                title: if t.title.is_empty() {
                    t.path.clone()
                } else {
                    t.title.clone()
                },
                output: out.to_string_lossy().to_string(),
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
        let req = req.clone();
        let gain = if req.bake_replaygain {
            t.rg_track_gain
        } else {
            None
        };
        let duration = t.duration_ms as f64 / 1000.0;
        tauri::async_runtime::spawn(async move {
            run_job(app, job.id, src, out, req, gain, duration).await;
        });
    }
    Ok(created)
}

async fn run_job(
    app: AppHandle,
    id: u64,
    src: PathBuf,
    out: PathBuf,
    req: ConvertRequest,
    gain: Option<f64>,
    duration: f64,
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
    update(&app, id, |j| j.state = JobState::Running).await;
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut opts = req.options.clone();
    if gain.is_some() {
        opts.gain_db = gain;
    }
    let part = {
        let stem = out
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        out.with_file_name(format!("{stem}.part.{}", req.preset.extension()))
    };
    let args = convert::ffmpeg_args(
        &src.to_string_lossy(),
        &part.to_string_lossy(),
        req.preset,
        &opts,
        Some(duration),
    );
    let span = match (opts.start_s, opts.end_s) {
        (s, Some(e)) => e - s.unwrap_or(0.0),
        (Some(s), None) => duration - s,
        _ => duration,
    }
    .max(0.1);
    let mut cmd = omniget_core::core::process::command("ffmpeg");
    cmd.args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            update(&app, id, |j| {
                j.state = JobState::Failed;
                j.error = Some(format!("ffmpeg: {e}"));
            })
            .await;
            return;
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let err_tail = Arc::new(Mutex::new(String::new()));
    if let Some(se) = stderr {
        let tail = err_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(se).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let mut t = tail.lock().await;
                *t = l;
            }
        });
    }
    let app2 = app.clone();
    let reader = async move {
        if let Some(so) = stdout {
            let mut lines = BufReader::new(so).lines();
            let mut last = std::time::Instant::now();
            while let Ok(Some(l)) = lines.next_line().await {
                if let Some(t) = convert::parse_progress_line(&l) {
                    if last.elapsed().as_millis() > 200 {
                        last = std::time::Instant::now();
                        let p = (t / span).clamp(0.0, 0.99);
                        update(&app2, id, |j| j.progress = p).await;
                    }
                }
            }
        }
    };
    let status = tokio::select! {
        _ = reader => child.wait().await.ok(),
        _ = token.cancelled() => {
            let _ = child.kill().await;
            None
        }
    };
    let status = match status {
        Some(s) => Some(s),
        None if !token.is_cancelled() => child.wait().await.ok(),
        None => None,
    };
    match status {
        Some(s) if s.success() => {
            if let Err(e) = std::fs::rename(&part, &out) {
                let _ = std::fs::remove_file(&part);
                update(&app, id, |j| {
                    j.state = JobState::Failed;
                    j.error = Some(e.to_string());
                })
                .await;
                return;
            }
            if req.add_to_library {
                let o = out.clone();
                let _ = tokio::task::spawn_blocking(move || scan::index_file(&o)).await;
            }
            update(&app, id, |j| {
                j.state = JobState::Done;
                j.progress = 1.0;
            })
            .await;
        }
        _ if token.is_cancelled() => {
            let _ = std::fs::remove_file(&part);
            update(&app, id, |j| j.state = JobState::Cancelled).await;
        }
        _ => {
            let _ = std::fs::remove_file(&part);
            let tail = err_tail.lock().await.clone();
            update(&app, id, |j| {
                j.state = JobState::Failed;
                j.error = Some(if tail.is_empty() {
                    "ffmpeg failed".into()
                } else {
                    tail
                });
            })
            .await;
        }
    }
}

pub async fn cancel(id: Option<u64>) {
    let r = REG.lock().await;
    for (jid, t) in &r.tokens {
        if id.map(|i| i == *jid).unwrap_or(true) {
            t.cancel();
        }
    }
}

pub async fn jobs() -> Vec<Job> {
    let r = REG.lock().await;
    let mut v: Vec<Job> = r.jobs.values().cloned().collect();
    v.sort_by_key(|j| j.id);
    v
}

pub async fn clear_finished() {
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
