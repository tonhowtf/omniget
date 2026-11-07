use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use omniget_core::core::convert::{
    self, external, formats, Category, ConvertOptions, ConvertOutcome, FormatInfo, InspectResult,
    ToolStatus,
};

static BATCHES: Mutex<Option<HashMap<String, CancellationToken>>> = Mutex::new(None);

const MEDIA_SLOTS: usize = 2;
const OTHER_SLOTS: usize = 4;

#[derive(Debug, Clone, Deserialize)]
pub struct ConvertItem {
    pub path: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize)]
struct ConverterProgress<'a> {
    batch_id: &'a str,
    index: usize,
    path: &'a str,
    percent: f64,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<&'a ConvertOutcome>,
}

fn emit(
    app: &AppHandle,
    batch_id: &str,
    index: usize,
    path: &str,
    percent: f64,
    state: &'static str,
    outcome: Option<&ConvertOutcome>,
) {
    let _ = app.emit(
        "converter-progress",
        ConverterProgress {
            batch_id,
            index,
            path,
            percent,
            state,
            outcome,
        },
    );
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

fn is_media(path: &Path, target: &str) -> bool {
    let media = |c: Option<Category>| matches!(c, Some(Category::Audio | Category::Video));
    media(formats::category_of(target)) || media(formats::detect(path).map(|f| f.category))
}

#[tauri::command]
pub async fn converter_tools() -> ToolStatus {
    external::reset_cache();
    convert::detect_tools().await
}

#[tauri::command]
pub fn converter_formats() -> Vec<FormatInfo> {
    convert::FORMATS.to_vec()
}

#[tauri::command]
pub async fn converter_inspect(paths: Vec<String>) -> Vec<InspectResult> {
    let tools = convert::detect_tools().await;
    let jobs = paths.iter().map(|p| {
        let tools = tools.clone();
        async move { convert::inspect(Path::new(p), &tools).await }
    });
    futures::future::join_all(jobs).await
}

#[tauri::command]
pub async fn converter_run(
    app: AppHandle,
    batch_id: String,
    items: Vec<ConvertItem>,
    out_dir: Option<String>,
    options: ConvertOptions,
) -> Vec<ConvertOutcome> {
    let token = register(&batch_id);
    let tools = convert::detect_tools().await;
    let out_dir: Option<PathBuf> = out_dir.filter(|d| !d.trim().is_empty()).map(PathBuf::from);
    let pairs: Vec<(PathBuf, String)> = items
        .iter()
        .map(|i| (PathBuf::from(&i.path), i.target.clone()))
        .collect();
    let planned = convert::plan_outputs(
        &pairs,
        out_dir.as_deref(),
        options.overwrite.unwrap_or(false),
    );
    let media_slots = Arc::new(Semaphore::new(MEDIA_SLOTS));
    let other_slots = Arc::new(Semaphore::new(OTHER_SLOTS));

    for (index, item) in items.iter().enumerate() {
        emit(&app, &batch_id, index, &item.path, 0.0, "queued", None);
    }

    let jobs = items.iter().enumerate().map(|(index, item)| {
        let app = app.clone();
        let batch_id = batch_id.as_str();
        let token = token.clone();
        let tools = tools.clone();
        let options = options.clone();
        let out_dir = out_dir.clone();
        let planned = planned.get(index).cloned().flatten();
        let media_slots = media_slots.clone();
        let other_slots = other_slots.clone();
        async move {
            let input = PathBuf::from(&item.path);
            let slots = if is_media(&input, &item.target) {
                media_slots
            } else {
                other_slots
            };
            let permit = tokio::select! {
                p = slots.acquire_owned() => p.ok(),
                _ = token.cancelled() => None,
            };
            let outcome = if permit.is_none() || token.is_cancelled() {
                ConvertOutcome {
                    input: item.path.clone(),
                    output: None,
                    size_bytes: 0,
                    route: None,
                    ok: false,
                    error: Some("Cancelled.".to_string()),
                    skipped: false,
                }
            } else {
                emit(&app, batch_id, index, &item.path, 0.0, "running", None);
                let last = Mutex::new(-1.0f64);
                let report = |fraction: f64| {
                    let percent = (fraction * 100.0).clamp(0.0, 100.0).floor();
                    let changed = last
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
                        emit(&app, batch_id, index, &item.path, percent, "running", None);
                    }
                };
                match planned.as_deref() {
                    Some(output) => {
                        convert::run_to(
                            &input,
                            &item.target,
                            output,
                            &options,
                            &tools,
                            token.clone(),
                            report,
                        )
                        .await
                    }
                    None => {
                        convert::run(
                            &input,
                            &item.target,
                            out_dir.as_deref(),
                            &options,
                            &tools,
                            token.clone(),
                            report,
                        )
                        .await
                    }
                }
            };
            drop(permit);
            let state = if outcome.skipped {
                "skipped"
            } else if outcome.ok {
                "done"
            } else {
                "failed"
            };
            let percent = if outcome.ok { 100.0 } else { 0.0 };
            emit(
                &app,
                batch_id,
                index,
                &item.path,
                percent,
                state,
                Some(&outcome),
            );
            outcome
        }
    });
    let outcomes = futures::future::join_all(jobs).await;
    unregister(&batch_id);
    outcomes
}

#[tauri::command]
pub fn converter_cancel(batch_id: String) {
    if let Ok(map) = BATCHES.lock() {
        if let Some(token) = map.as_ref().and_then(|m| m.get(&batch_id)) {
            token.cancel();
        }
    }
}
