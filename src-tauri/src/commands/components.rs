use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use omniget_core::core::components::{
    self, cache,
    fetch::{self, FetchError, FetchResult},
    project::{self, ProjectInfo},
    runner::{self, RunOutcome},
    BuiltinRegistry,
};

static RUNS: Mutex<Option<HashMap<String, CancellationToken>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
struct OutputLine<'a> {
    run_id: &'a str,
    stream: &'static str,
    line: String,
}

#[tauri::command]
pub fn components_registries() -> Vec<BuiltinRegistry> {
    components::BUILTIN_REGISTRIES.to_vec()
}

#[tauri::command]
pub async fn components_fetch(url: String, force: Option<bool>) -> Result<FetchResult, FetchError> {
    let dir = cache::default_dir();
    fetch::fetch_json(&url, force.unwrap_or(false), dir.as_deref()).await
}

#[tauri::command]
pub fn components_cache_clear() -> Result<usize, String> {
    match cache::default_dir() {
        Some(dir) => cache::clear(&dir).map_err(|e| e.to_string()),
        None => Ok(0),
    }
}

#[tauri::command]
pub fn components_inspect_project(path: String) -> Result<ProjectInfo, String> {
    project::inspect(&PathBuf::from(path)).map_err(String::from)
}

#[tauri::command]
pub fn components_read_css(project: String, rel: String) -> Result<String, String> {
    project::read_css(&PathBuf::from(project), &rel).map_err(String::from)
}

#[tauri::command]
pub fn components_write_css(
    project: String,
    rel: String,
    expected: String,
    content: String,
) -> Result<String, String> {
    project::write_css(&PathBuf::from(project), &rel, &expected, &content).map_err(String::from)
}

#[tauri::command]
pub async fn components_run(
    app: AppHandle,
    run_id: String,
    cwd: String,
    program: String,
    args: Vec<String>,
) -> RunOutcome {
    let token = CancellationToken::new();
    if let Ok(mut map) = RUNS.lock() {
        map.get_or_insert_with(HashMap::new)
            .insert(run_id.clone(), token.clone());
    }
    let id = run_id.clone();
    let outcome = runner::run(
        &PathBuf::from(cwd),
        &program,
        &args,
        token,
        move |stream, line| {
            let _ = app.emit(
                "components-output",
                OutputLine {
                    run_id: &id,
                    stream,
                    line,
                },
            );
        },
    )
    .await;
    if let Ok(mut map) = RUNS.lock() {
        if let Some(m) = map.as_mut() {
            m.remove(&run_id);
        }
    }
    outcome
}

#[tauri::command]
pub fn components_cancel(run_id: String) {
    if let Ok(map) = RUNS.lock() {
        if let Some(token) = map.as_ref().and_then(|m| m.get(&run_id)) {
            token.cancel();
        }
    }
}
