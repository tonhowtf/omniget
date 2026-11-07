use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use omniget_core::core::cutout::{
    self, output, source, Background, Click, Edge, Format, Preview, Status,
};

const INSPECT_PARALLEL: usize = 3;

static BATCHES: Mutex<Option<HashMap<String, CancellationToken>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
struct InstallProgress {
    model: String,
    id: String,
    stage: String,
    done: u64,
    total: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Inspected {
    pub path: String,
    pub name: String,
    pub info: Option<source::Info>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CutItem {
    pub path: String,
    #[serde(default)]
    pub frame_ms: Option<u64>,
    #[serde(default)]
    pub clicks: Vec<Click>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CutOptions {
    pub model: String,
    #[serde(default)]
    pub edge: Edge,
    #[serde(default)]
    pub background: Background,
    #[serde(default)]
    pub format: Format,
    #[serde(default)]
    pub out_dir: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CutOutcome {
    pub input: String,
    pub frame_ms: Option<u64>,
    pub output: Option<String>,
    pub size_bytes: u64,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct CutProgress<'a> {
    batch_id: &'a str,
    index: usize,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<&'a CutOutcome>,
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

fn model_error(e: anyhow::Error) -> String {
    let text = e.to_string();
    if text.contains("has not been downloaded") || text.contains("not installed") {
        "The cutout model is not downloaded yet. Download it above and try again.".to_string()
    } else {
        format!("The picture could not be cut out: {text}")
    }
}

#[tauri::command]
pub fn cutout_status() -> Status {
    cutout::status()
}

#[tauri::command]
pub async fn cutout_install(app: AppHandle, model: String) -> Result<Status, String> {
    let status = cutout::status();
    if !status.runtime_installed && !status.runtime_downloadable {
        return Err(
            "Cutting out needs the ONNX Runtime, which has no ready build for this system."
                .to_string(),
        );
    }
    let name = model.clone();
    let progress: omniget_core::core::tools::ProgressFn = Arc::new(move |p| {
        let _ = app.emit(
            "cutout-install-progress",
            InstallProgress {
                model: name.clone(),
                id: p.id,
                stage: p.stage,
                done: p.done,
                total: p.total,
            },
        );
    });
    cutout::install(&model, &progress).await.map_err(|e| {
        format!("Could not download the model: {e}. Check your connection and try again.")
    })?;
    Ok(cutout::status())
}

#[tauri::command]
pub async fn cutout_inspect(paths: Vec<String>) -> Vec<Inspected> {
    let jobs = paths.into_iter().map(|path| async move {
        let p = PathBuf::from(&path);
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        match source::inspect(&p).await {
            Ok(info) => Inspected {
                path,
                name,
                info: Some(info),
                error: None,
            },
            Err(e) => Inspected {
                path,
                name,
                info: None,
                error: Some(e),
            },
        }
    });
    futures::stream::iter(jobs)
        .buffered(INSPECT_PARALLEL)
        .collect()
        .await
}

#[tauri::command]
pub async fn cutout_preview(
    path: String,
    frame_ms: Option<u64>,
    model: String,
    clicks: Vec<Click>,
    edge: Edge,
    background: Background,
) -> Result<Preview, String> {
    let p = PathBuf::from(&path);
    let work = cutout::working_picture(&p, frame_ms).await?;
    let fill = cutout::preview_fill(&background).await?;
    let key = source::key(&p, frame_ms);
    tokio::task::spawn_blocking(move || {
        let raw = cutout::mask(&key, &work, &model, &clicks).map_err(model_error)?;
        cutout::preview(&work, &raw, edge, &fill)
    })
    .await
    .map_err(|e| format!("The preview stopped unexpectedly: {e}"))?
}

async fn render_one(
    item: &CutItem,
    options: &CutOptions,
    fill: &cutout::Fill,
) -> Result<Vec<u8>, String> {
    let input = PathBuf::from(&item.path);
    let full = source::load(&input, item.frame_ms).await?;
    let key = source::key(&input, item.frame_ms);
    let model = options.model.clone();
    let clicks = item.clicks.clone();
    let edge = options.edge;
    let format = options.format;
    let fill = fill.clone();
    tokio::task::spawn_blocking(move || {
        let work = cutout::shrink(&full, cutout::WORK_SIDE);
        let raw = cutout::mask(&key, &work, &model, &clicks).map_err(model_error)?;
        output::encode(&cutout::cut(&full, &raw, edge, &fill), format)
    })
    .await
    .map_err(|e| format!("The cutout stopped unexpectedly: {e}"))?
}

#[tauri::command]
pub async fn cutout_run(
    app: AppHandle,
    batch_id: String,
    items: Vec<CutItem>,
    options: CutOptions,
) -> Result<Vec<CutOutcome>, String> {
    let fill = cutout::fill_for(&options.background).await?;
    let token = register(&batch_id);
    let out_dir = options
        .out_dir
        .as_deref()
        .filter(|d| !d.trim().is_empty())
        .map(PathBuf::from);
    let pairs: Vec<(PathBuf, Option<u64>)> = items
        .iter()
        .map(|i| (PathBuf::from(&i.path), i.frame_ms))
        .collect();
    let planned = output::plan(
        &pairs,
        out_dir.as_deref(),
        options.format,
        options.overwrite,
    );
    let emit = |index: usize, state: &'static str, outcome: Option<&CutOutcome>| {
        let _ = app.emit(
            "cutout-progress",
            CutProgress {
                batch_id: &batch_id,
                index,
                state,
                outcome,
            },
        );
    };
    for index in 0..items.len() {
        emit(index, "queued", None);
    }
    let mut outcomes = Vec::with_capacity(items.len());
    for (index, (item, output_path)) in items.iter().zip(planned.iter()).enumerate() {
        let result = if token.is_cancelled() {
            Err("Cancelled.".to_string())
        } else {
            emit(index, "running", None);
            let rendered = tokio::select! {
                r = render_one(item, &options, &fill) => r,
                _ = token.cancelled() => Err("Cancelled.".to_string()),
            };
            match rendered {
                Ok(bytes) => output::save(output_path, &bytes).await,
                Err(e) => Err(e),
            }
        };
        let outcome = match result {
            Ok(size_bytes) => CutOutcome {
                input: item.path.clone(),
                frame_ms: item.frame_ms,
                output: Some(output_path.to_string_lossy().into_owned()),
                size_bytes,
                ok: true,
                error: None,
            },
            Err(error) => CutOutcome {
                input: item.path.clone(),
                frame_ms: item.frame_ms,
                output: None,
                size_bytes: 0,
                ok: false,
                error: Some(error),
            },
        };
        emit(
            index,
            if outcome.ok { "done" } else { "failed" },
            Some(&outcome),
        );
        outcomes.push(outcome);
    }
    unregister(&batch_id);
    Ok(outcomes)
}

#[tauri::command]
pub fn cutout_cancel(batch_id: String) {
    if let Ok(map) = BATCHES.lock() {
        if let Some(token) = map.as_ref().and_then(|m| m.get(&batch_id)) {
            token.cancel();
        }
    }
}

#[tauri::command]
pub fn cutout_release() {
    cutout::release();
}
