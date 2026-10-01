pub mod accounts;
pub mod api;
pub mod auth;
pub mod bandwidth;
pub mod batch;
pub mod cdn;
pub mod clone;
pub mod dedupe;
pub mod downloader;
pub mod events;
pub mod export;
pub mod history;
pub mod index;
pub mod links;
pub mod parallel_download;
pub mod player;
pub mod preview_cache;
pub mod realtime;
pub mod resume;
pub mod stream_server;
pub mod sync_state;
pub mod template;
pub mod thumbnail_cache;
pub mod transcribe;
pub mod upload;
pub mod watch;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Once, OnceLock};

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::host;
use auth::{TelegramSessionHandle, TelegramState};
use stream_server::StreamServerInfo;

pub type ActiveDownloads = Arc<Mutex<HashMap<u64, (String, CancellationToken)>>>;
pub type StreamSlot = Arc<Mutex<Option<Arc<Mutex<StreamServerInfo>>>>>;

pub struct Engine {
    pub session: TelegramSessionHandle,
    pub active: ActiveDownloads,
    pub stream_server: StreamSlot,
    started: Once,
}

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();

pub fn engine() -> Arc<Engine> {
    let e = ENGINE
        .get_or_init(|| {
            Arc::new(Engine {
                session: Arc::new(Mutex::new(TelegramState::new())),
                active: Arc::new(Mutex::new(HashMap::new())),
                stream_server: Arc::new(Mutex::new(None)),
                started: Once::new(),
            })
        })
        .clone();
    if tokio::runtime::Handle::try_current().is_ok() {
        let me = e.clone();
        e.started.call_once(move || me.start_background());
    }
    e
}

impl Engine {
    fn start_background(self: Arc<Self>) {
        migrate_legacy_data();
        thumbnail_cache::init_disk_cache(host::data_dir().join("previews"));
        apply_settings(&settings());

        let session = self.session.clone();
        let slot = self.stream_server.clone();
        tokio::spawn(async move {
            match stream_server::start(session).await {
                Ok(info) => *slot.lock().await = Some(info),
                Err(e) => tracing::error!("[tg-stream] failed to start streaming server: {}", e),
            }
        });

        sync_state::spawn_background_loop(
            self.session.clone(),
            tokio::runtime::Handle::current(),
            CancellationToken::new(),
        );

        watch::spawn_background_loop(self.session.clone(), self.active.clone());
        realtime::spawn(self.session.clone(), self.active.clone());
    }
}

static LAST_RESTORE_FAIL: AtomicI64 = AtomicI64::new(0);

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub async fn restore_session(handle: &auth::TelegramSessionHandle) {
    if handle.lock().await.client.is_some() {
        return;
    }
    let has_file = auth::session_file_path()
        .map(|p| p.is_file())
        .unwrap_or(false);
    if !has_file || now_secs() - LAST_RESTORE_FAIL.load(Ordering::Relaxed) < 20 {
        return;
    }
    static RESTORING: OnceLock<Mutex<()>> = OnceLock::new();
    let _one_at_a_time = RESTORING.get_or_init(|| Mutex::new(())).lock().await;
    if handle.lock().await.client.is_some() {
        return;
    }
    match auth::check_session(handle).await {
        Ok(_) => tracing::info!("[telegram] saved session restored"),
        Err(e) => {
            LAST_RESTORE_FAIL.store(now_secs(), Ordering::Relaxed);
            tracing::debug!("[telegram] session restore failed: {}", e);
        }
    }
}

pub async fn stream_endpoint() -> Result<(String, String), String> {
    let slot = engine().stream_server.clone();
    for _ in 0..50 {
        let info = slot.lock().await.clone();
        if let Some(info) = info {
            let g = info.lock().await;
            return Ok((format!("http://127.0.0.1:{}", g.port), g.token.clone()));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Err("stream server not ready".into())
}

pub fn settings() -> serde_json::Value {
    host::get()
        .map(|h| h.settings())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}))
}

pub fn setting(key: &str) -> serde_json::Value {
    settings()
        .get(key)
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

pub fn save_setting(key: &str, value: serde_json::Value) {
    let Some(h) = host::get() else { return };
    let mut current = settings();
    if let Some(obj) = current.as_object_mut() {
        obj.insert(key.to_string(), value);
    }
    h.save_settings(current);
}

pub fn fix_file_extensions() -> bool {
    setting("fix_file_extensions").as_bool().unwrap_or(false)
}

pub fn concurrent_downloads() -> usize {
    (setting("concurrent_downloads").as_u64().unwrap_or(3) as usize).clamp(1, 10)
}

pub fn download_dir() -> PathBuf {
    if let Some(dir) = setting("download_dir")
        .as_str()
        .filter(|s| !s.trim().is_empty())
    {
        return PathBuf::from(dir);
    }
    host::get()
        .map(|h| h.default_output_dir())
        .or_else(dirs::download_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("Telegram")
}

fn apply_settings(saved: &serde_json::Value) {
    if let Some(n) = saved.get("max_threads").and_then(|v| v.as_u64()) {
        parallel_download::set_max_threads_cap(n as usize);
    }
    if let Some(v) = saved.get("sync_enabled").and_then(|v| v.as_bool()) {
        sync_state::set_enabled(v);
    }
    if let Some(n) = saved.get("sync_interval_min").and_then(|v| v.as_u64()) {
        sync_state::set_interval_min(n as u32);
    }
}

pub fn migrate_legacy_data() {
    static DONE: Once = Once::new();
    DONE.call_once(|| {
        let dest = host::data_dir();
        let session_dest = dest.join("telegram.session");
        if !session_dest.exists() {
            let mut candidates: Vec<PathBuf> = Vec::new();
            if let Some(base) = dirs::data_dir() {
                candidates.push(base.join("omniget").join("telegram.session"));
                candidates.push(base.join("wtf.tonho.omniget").join("telegram.session"));
            }
            if let Some(app) = crate::core::paths::app_data_dir() {
                candidates.push(app.join("telegram.session"));
            }
            if let Some(src) = candidates.into_iter().find(|p| p.is_file()) {
                match std::fs::copy(&src, &session_dest) {
                    Ok(_) => {
                        for suffix in ["-wal", "-shm"] {
                            let side = PathBuf::from(format!("{}{}", src.display(), suffix));
                            if side.is_file() {
                                let _ = std::fs::copy(
                                    &side,
                                    PathBuf::from(format!("{}{}", session_dest.display(), suffix)),
                                );
                            }
                        }
                        tracing::info!("[tg-migrate] session copied from {}", src.display());
                    }
                    Err(e) => tracing::warn!(
                        "[tg-migrate] session copy from {} failed: {}",
                        src.display(),
                        e
                    ),
                }
            }
        }

        let mut plugin_dirs: Vec<PathBuf> = Vec::new();
        if let Some(app) = crate::core::paths::app_data_dir() {
            plugin_dirs.push(app.join("plugins").join("telegram").join("data"));
        }
        if let Some(base) = dirs::data_dir() {
            plugin_dirs.push(
                base.join("wtf.tonho.omniget")
                    .join("plugins")
                    .join("telegram")
                    .join("data"),
            );
        }
        if let Some(src) = plugin_dirs.into_iter().find(|p| p.is_dir()) {
            for name in [
                "accounts.json",
                "bandwidth.json",
                "accounts",
                "backups",
                "clone_sessions",
                "resume",
            ] {
                let from = src.join(name);
                let to = dest.join(name);
                if from.exists() && !to.exists() {
                    if let Err(e) = copy_recursive(&from, &to) {
                        tracing::warn!("[tg-migrate] copying {} failed: {}", from.display(), e);
                    }
                }
            }
        }
    });
}

fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)?.flatten() {
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}
