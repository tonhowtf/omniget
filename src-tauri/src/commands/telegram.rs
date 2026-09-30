//! `/telegram` superpower. The engine lives in `omniget_core::core::telegram`
//! (so the CLI can reuse it); this file installs the app host and exposes
//! the connection manager and the command dispatcher to the webview.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use omniget_core::core::telegram::host::{self, TelegramHost};
use omniget_core::core::telegram::net::{doctor, manager, route::Route, NetOpts};

struct AppHost {
    app: AppHandle,
}

impl TelegramHost for AppHost {
    fn emit(&self, event: &str, payload: Value) {
        if matches!(
            event,
            "telegram-download-queued"
                | "telegram-download-progress"
                | "telegram-download-complete"
                | "telegram-watch-saved"
        ) {
            if let Some(tx) = BRIDGE.get() {
                let _ = tx.send((event.to_string(), payload.clone()));
            }
        }
        let _ = self.app.emit(event, payload);
    }

    fn data_dir(&self) -> PathBuf {
        let dir = omniget_core::core::paths::app_data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("telegram");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn settings(&self) -> Value {
        let s = crate::storage::config::load_settings(&self.app).telegram;
        let mut obj = match s.prefs {
            Value::Object(m) => m,
            _ => Default::default(),
        };
        obj.entry("concurrent_downloads")
            .or_insert_with(|| Value::from(s.concurrent_downloads));
        obj.entry("fix_file_extensions")
            .or_insert_with(|| Value::from(s.fix_file_extensions));
        Value::Object(obj)
    }

    fn save_settings(&self, settings: Value) {
        let _ = crate::storage::config::update_settings(&self.app, |all| {
            if let Some(n) = settings
                .get("concurrent_downloads")
                .and_then(|v| v.as_u64())
            {
                all.telegram.concurrent_downloads = n as u32;
            }
            if let Some(b) = settings
                .get("fix_file_extensions")
                .and_then(|v| v.as_bool())
            {
                all.telegram.fix_file_extensions = b;
            }
            all.telegram.prefs = settings;
        });
    }

    fn default_output_dir(&self) -> PathBuf {
        crate::storage::config::load_settings(&self.app)
            .download
            .default_output_dir
    }

    fn toast(&self, kind: &str, message: &str) {
        let _ = self.app.emit(
            "telegram-toast",
            serde_json::json!({ "kind": kind, "message": message }),
        );
    }
}

/// Called once from the app setup.
pub fn install(app: &AppHandle) {
    let (tx, rx) = mpsc::unbounded_channel();
    let _ = BRIDGE.set(tx);
    tauri::async_runtime::spawn(run_bridge(app.clone(), rx));
    host::install(Arc::new(AppHost { app: app.clone() }));
}

// ---- Telegram downloads in the app's Downloads tab ----------------------------
//
// Single downloads and batch rows of the Telegram engine become items of the
// global download queue (`platform: "telegram"`), driven by the engine's own
// `telegram-download-*` events: progress, done, error and "open folder" work
// as for any download, and cancelling there cancels the Telegram job. Events
// go through one channel so they are applied in order.

static BRIDGE: OnceLock<mpsc::UnboundedSender<(String, Value)>> = OnceLock::new();

struct Mirrored {
    queue_id: u64,
    title: String,
    /// Cancelled when the Telegram job ends, to stop the cancel watcher.
    done: CancellationToken,
}

fn show_in_downloads() -> bool {
    host::get()
        .map(|h| h.settings())
        .and_then(|s| s.get("show_in_downloads").and_then(|v| v.as_bool()))
        .unwrap_or(true)
}

async fn run_bridge(app: AppHandle, mut rx: mpsc::UnboundedReceiver<(String, Value)>) {
    let mut rows: HashMap<u64, Mirrored> = HashMap::new();
    while let Some((event, payload)) = rx.recv().await {
        if event == "telegram-watch-saved" {
            let wants = payload
                .get("add_to_watch")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let paths: Vec<PathBuf> = payload
                .get("paths")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.as_str().map(PathBuf::from))
                        .collect()
                })
                .unwrap_or_default();
            if wants && !paths.is_empty() {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let r = add_to_watch(&app, paths).await;
                    tracing::info!("[telegram] auto-save added to Watch: {}", r);
                });
            }
            continue;
        }
        let kind = payload.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        if kind != "single" && kind != "batch" {
            continue;
        }
        let Some(tg_id) = payload.get("id").and_then(|v| v.as_u64()) else {
            continue;
        };
        let Some(state) = app.try_state::<crate::AppState>() else {
            continue;
        };
        let queue = state.download_queue.clone();
        match event.as_str() {
            "telegram-download-queued" => {
                if !show_in_downloads() || rows.contains_key(&tg_id) {
                    continue;
                }
                let title = payload
                    .get("title")
                    .and_then(|v| v.as_str())
                    .filter(|t| !t.is_empty())
                    .unwrap_or("Telegram")
                    .to_string();
                let path = payload
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let total = payload.get("total_bytes").and_then(|v| v.as_u64());
                let cancel = CancellationToken::new();
                let done = CancellationToken::new();
                let queue_id = {
                    let mut q = queue.lock().await;
                    let id = q.next_available_id(crate::core::queue::now_ms());
                    q.items.push(queue_item(
                        id,
                        tg_id,
                        &title,
                        kind,
                        path.as_deref(),
                        total,
                        cancel.clone(),
                    ));
                    crate::core::queue::emit_queue_state(&app, &q);
                    id
                };
                // Cancel in the Downloads tab → cancel the Telegram job.
                let watch_done = done.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            let _ = omniget_core::core::telegram::commands::dispatch(
                                "telegram_cancel_batch".into(),
                                json!({ "batchId": tg_id }),
                            )
                            .await;
                        }
                        _ = watch_done.cancelled() => {}
                    }
                });
                rows.insert(
                    tg_id,
                    Mirrored {
                        queue_id,
                        title,
                        done,
                    },
                );
            }
            "telegram-download-progress" => {
                let Some(row) = rows.get(&tg_id) else {
                    continue;
                };
                let percent = payload.get("percent").and_then(|v| v.as_f64());
                let speed = payload
                    .get("speed_bytes_per_sec")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let downloaded = payload
                    .get("downloaded_bytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let total = payload.get("total_bytes").and_then(|v| v.as_u64());
                let eta = match (total, speed > 0.0) {
                    (Some(t), true) if t > downloaded => {
                        Some(((t - downloaded) as f64 / speed) as u64)
                    }
                    _ => None,
                };
                queue.lock().await.update_progress(
                    row.queue_id,
                    percent,
                    speed,
                    downloaded,
                    total,
                    None,
                    eta,
                );
                let _ = app.emit(
                    "queue-item-progress",
                    crate::core::queue::QueueItemProgress {
                        id: row.queue_id,
                        title: row.title.clone(),
                        platform: "telegram".into(),
                        percent,
                        speed_bytes_per_sec: speed,
                        downloaded_bytes: downloaded,
                        total_bytes: total,
                        phase: "downloading".into(),
                        eta_seconds: eta,
                        ..Default::default()
                    },
                );
            }
            "telegram-download-complete" => {
                let Some(row) = rows.remove(&tg_id) else {
                    continue;
                };
                row.done.cancel();
                let success = payload
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let error = payload
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let file_path = payload
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let size = payload.get("file_size_bytes").and_then(|v| v.as_u64());
                let mut q = queue.lock().await;
                if let Some(count) = payload.get("file_count").and_then(|v| v.as_u64()) {
                    if let Some(item) = q.items.iter_mut().find(|i| i.id == row.queue_id) {
                        item.file_count = Some(count as u32);
                    }
                }
                q.mark_complete(row.queue_id, success, error, file_path, size);
                crate::core::queue::emit_queue_state(&app, &q);
            }
            _ => {}
        }
    }
}

fn queue_item(
    id: u64,
    tg_id: u64,
    title: &str,
    kind: &str,
    path: Option<&str>,
    total: Option<u64>,
    cancel: CancellationToken,
) -> crate::core::queue::QueueItem {
    use crate::core::queue::{QueueItem, QueueKind, QueueStatus};
    let output_dir = match (kind, path) {
        ("single", Some(p)) => Path::new(p)
            .parent()
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_default(),
        (_, Some(p)) => p.to_string(),
        _ => String::new(),
    };
    let item_kind = path
        .map(|p| {
            let ext = Path::new(p)
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            match ext.as_str() {
                "mp4" | "mkv" | "mov" | "webm" | "avi" | "m4v" => QueueKind::Video,
                "mp3" | "m4a" | "ogg" | "opus" | "flac" | "wav" => QueueKind::Audio,
                "jpg" | "jpeg" | "png" | "webp" | "gif" => QueueKind::Image,
                "pdf" => QueueKind::Pdf,
                _ => QueueKind::Generic,
            }
        })
        .unwrap_or(QueueKind::Generic);
    let now = crate::core::queue::now_ms();
    QueueItem {
        id,
        url: format!("telegram://download/{}", tg_id),
        platform: "telegram".into(),
        title: title.to_string(),
        status: QueueStatus::Active,
        cancel_token: cancel,
        output_dir,
        download_mode: None,
        quality: None,
        format_id: None,
        referer: None,
        extra_headers: None,
        page_url: None,
        user_agent: None,
        percent: Some(0.0),
        speed_bytes_per_sec: 0.0,
        downloaded_bytes: 0,
        total_bytes: total,
        file_path: None,
        file_size_bytes: None,
        file_count: None,
        media_info: None,
        downloader: Arc::new(
            omniget_core::core::telegram::client::downloader::TelegramDownloader::new(
                omniget_core::core::telegram::client::engine()
                    .session
                    .clone(),
            ),
        ),
        ytdlp_path: None,
        from_hotkey: false,
        torrent_id: None,
        kind: Some(item_kind),
        thumbnail_url_override: None,
        retry_count: 0,
        max_retries: 0,
        resume_state: None,
        concurrent_segments: None,
        segment_size_bytes: None,
        eta_seconds: None,
        cookie_slug: None,
        custom_ytdlp_args: None,
        torrent_files: None,
        scheduled_at_ms: None,
        stop_at_ms: None,
        phase: Some("downloading".into()),
        current_stream: None,
        streams_done: Vec::new(),
        planned_formats: None,
        fragment_index: None,
        fragment_count: None,
        started_at_ms: Some(now),
        finished_at_ms: None,
        ytdlp_argv_override: None,
    }
}

// ---- /video (Watch) -------------------------------------------------------------

fn collect_videos(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        if let Ok(rd) = std::fs::read_dir(path) {
            let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            entries.sort();
            for p in entries {
                let hidden = p
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with('.'))
                    .unwrap_or(true);
                if !hidden {
                    collect_videos(&p, out);
                }
            }
        }
    } else if path.is_file() && super::video::probe::is_video_path(path) {
        out.push(path.to_path_buf());
    }
}

/// Adds files (folders recursively, videos only) to the /video library:
/// `{added, skipped, errors: [{path, error}]}`.
async fn add_to_watch(app: &AppHandle, paths: Vec<PathBuf>) -> Value {
    let mut files = Vec::new();
    for p in &paths {
        collect_videos(p, &mut files);
    }
    let skipped = paths
        .iter()
        .filter(|p| p.is_file() && !super::video::probe::is_video_path(p))
        .count();
    let mut added = 0usize;
    let mut errors = Vec::new();
    for f in files {
        match super::video::scan::index_loose(&f).await {
            Ok(_) => added += 1,
            Err(e) => errors.push(json!({ "path": f.to_string_lossy(), "error": e })),
        }
    }
    if added > 0 {
        let _ = app.emit("video-library-changed", ());
    }
    json!({ "added": added, "skipped": skipped, "errors": errors })
}

fn emit_status(status: &manager::Status) {
    if let Some(h) = host::get() {
        h.emit(
            "telegram-net-status",
            serde_json::to_value(status).unwrap_or_default(),
        );
    }
}

fn status_now() -> manager::Status {
    let s = manager::get().status();
    emit_status(&s);
    s
}

#[tauri::command]
pub async fn telegram_call(
    app: AppHandle,
    command: String,
    args: Option<Value>,
) -> Result<Value, String> {
    let args = args.unwrap_or(Value::Null);
    // Commands that need the app itself (the /video library).
    if command == "telegram_add_to_watch" {
        let mut paths: Vec<PathBuf> = match args.get("paths").or(args.get("path")) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(PathBuf::from))
                .collect(),
            Some(Value::String(p)) => vec![PathBuf::from(p)],
            _ => Vec::new(),
        };
        if let Some(folder) = args.get("folder").and_then(|v| v.as_str()) {
            paths.push(PathBuf::from(folder));
        }
        if paths.is_empty() {
            return Err("missing 'paths' or 'folder'".into());
        }
        return Ok(add_to_watch(&app, paths).await);
    }
    omniget_core::core::telegram::commands::dispatch(command, args).await
}

#[tauri::command]
pub async fn telegram_net_status() -> Result<manager::Status, String> {
    Ok(manager::get().status())
}

#[tauri::command]
pub async fn telegram_net_scan() -> Result<manager::Status, String> {
    let mgr = manager::get();
    // Tell the screen right away that a scan is running.
    let watcher = {
        let mgr = mgr.clone();
        tokio::spawn(async move {
            for _ in 0..60 {
                tokio::time::sleep(std::time::Duration::from_millis(700)).await;
                let s = mgr.status();
                let scanning = matches!(s.phase, manager::Phase::Scanning);
                emit_status(&s);
                if !scanning {
                    break;
                }
            }
        })
    };
    let status = mgr.scan().await;
    watcher.abort();
    emit_status(&status);
    Ok(status)
}

#[tauri::command]
pub async fn telegram_net_doctor() -> Result<doctor::Report, String> {
    let report = doctor::run(&manager::get()).await;
    emit_status(&report.status);
    Ok(report)
}

#[derive(serde::Serialize)]
pub struct AddLinksResult {
    added: Vec<Route>,
    unsupported: Vec<String>,
}

#[tauri::command]
pub async fn telegram_net_add_links(text: String) -> Result<AddLinksResult, String> {
    let mgr = manager::get();
    let (added, unsupported) = mgr.add_from_text(&text);
    if !added.is_empty() {
        mgr.probe_many(added.clone()).await;
    }
    status_now();
    Ok(AddLinksResult { added, unsupported })
}

#[tauri::command]
pub async fn telegram_net_add_route(route: Route) -> Result<Route, String> {
    let mgr = manager::get();
    let r = mgr.add_route(route);
    mgr.probe_many(vec![r.clone()]).await;
    status_now();
    Ok(r)
}

#[tauri::command]
pub async fn telegram_net_remove_route(id: String) -> Result<(), String> {
    manager::get().remove_route(&id);
    status_now();
    Ok(())
}

#[tauri::command]
pub async fn telegram_net_keep(id: String) -> Result<(), String> {
    manager::get().keep(&id).map_err(|e| e.to_string())?;
    status_now();
    Ok(())
}

#[tauri::command]
pub async fn telegram_net_remove_failed() -> Result<usize, String> {
    let n = manager::get().remove_failed();
    status_now();
    Ok(n)
}

#[tauri::command]
pub async fn telegram_net_test(id: String) -> Result<manager::RouteView, String> {
    let v = manager::get()
        .test_route(&id)
        .await
        .map_err(|e| e.to_string())?;
    status_now();
    Ok(v)
}

#[tauri::command]
pub async fn telegram_net_set_mode(
    mode: manager::Mode,
    fixed_route: Option<String>,
) -> Result<(), String> {
    manager::get().set_mode(mode, fixed_route);
    status_now();
    Ok(())
}

#[tauri::command]
pub async fn telegram_net_set_opts(opts: NetOpts, use_rescue: Option<bool>) -> Result<(), String> {
    manager::get().set_opts(opts, use_rescue);
    status_now();
    Ok(())
}

#[tauri::command]
pub async fn telegram_net_add_subscription(
    url: String,
    name: Option<String>,
) -> Result<manager::SubscriptionEntry, String> {
    let entry = manager::get()
        .add_subscription(url.trim(), name.as_deref().unwrap_or(""))
        .await
        .map_err(|e| format!("{e:#}"))?;
    status_now();
    Ok(entry)
}

#[tauri::command]
pub async fn telegram_net_remove_subscription(url: String) -> Result<(), String> {
    manager::get().remove_subscription(&url);
    status_now();
    Ok(())
}

#[tauri::command]
pub async fn telegram_net_refresh_subscriptions() -> Result<Value, String> {
    let res = manager::get().refresh_subscriptions().await;
    status_now();
    Ok(serde_json::to_value(res).unwrap_or_default())
}

// ---- optional mihomo core (VLESS/Hysteria2/VMess/TUIC… nodes) ----

use omniget_core::core::telegram::net::mihomo::MihomoStatus;

#[tauri::command]
pub async fn telegram_net_mihomo_status() -> Result<MihomoStatus, String> {
    Ok(manager::get().mihomo_status())
}

#[tauri::command]
pub async fn telegram_net_mihomo_set(on: bool) -> Result<MihomoStatus, String> {
    let st = manager::get()
        .set_use_mihomo(on)
        .await
        .map_err(|e| format!("{e:#}"))?;
    status_now();
    Ok(st)
}

#[tauri::command]
pub async fn telegram_net_mihomo_restart() -> Result<MihomoStatus, String> {
    let st = manager::get()
        .mihomo_restart()
        .await
        .map_err(|e| format!("{e:#}"))?;
    status_now();
    Ok(st)
}

#[tauri::command]
pub async fn telegram_net_mihomo_remove_node(key: String) -> Result<(), String> {
    manager::get().mihomo_remove_node(&key).await;
    status_now();
    Ok(())
}
