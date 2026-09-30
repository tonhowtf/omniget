use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::auth::TelegramSessionHandle;

const DEFAULT_INTERVAL_MIN: u32 = 30;
const MIN_INTERVAL_MIN: u32 = 5;
const MAX_INTERVAL_MIN: u32 = 360;

static ENABLED: AtomicBool = AtomicBool::new(true);
static INTERVAL_MIN: AtomicU32 = AtomicU32::new(DEFAULT_INTERVAL_MIN);
static LAST_SUCCESS_AT: AtomicI64 = AtomicI64::new(0);
static LAST_DURATION_MS: AtomicU32 = AtomicU32::new(0);
static LAST_UPDATED_COUNT: AtomicU32 = AtomicU32::new(0);
static IS_SYNCING: AtomicBool = AtomicBool::new(false);

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(v: bool) {
    ENABLED.store(v, Ordering::Relaxed);
}

pub fn interval_min() -> u32 {
    INTERVAL_MIN.load(Ordering::Relaxed)
}

pub fn set_interval_min(v: u32) {
    let clamped = v.clamp(MIN_INTERVAL_MIN, MAX_INTERVAL_MIN);
    INTERVAL_MIN.store(clamped, Ordering::Relaxed);
}

#[derive(Debug, Serialize, Clone)]
pub struct SyncState {
    pub enabled: bool,
    pub interval_min: u32,
    pub last_success_at: i64,
    pub last_duration_ms: u32,
    pub last_updated_count: u32,
    pub is_syncing: bool,
}

pub fn snapshot() -> SyncState {
    SyncState {
        enabled: is_enabled(),
        interval_min: interval_min(),
        last_success_at: LAST_SUCCESS_AT.load(Ordering::Relaxed),
        last_duration_ms: LAST_DURATION_MS.load(Ordering::Relaxed),
        last_updated_count: LAST_UPDATED_COUNT.load(Ordering::Relaxed),
        is_syncing: IS_SYNCING.load(Ordering::Relaxed),
    }
}

#[derive(Serialize)]
struct SyncEvent {
    stage: &'static str,
    state: SyncState,
}

fn emit(stage: &'static str) {
    let ev = SyncEvent {
        stage,
        state: snapshot(),
    };
    super::events::emit(
        "telegram-sync-state",
        serde_json::to_value(&ev).unwrap_or_default(),
    );
}

pub async fn run_once(handle: &TelegramSessionHandle) -> anyhow::Result<u32> {
    if IS_SYNCING.swap(true, Ordering::AcqRel) {
        return Ok(0);
    }
    emit("started");
    let started = std::time::Instant::now();
    let result = super::api::refresh_all_dialogs(handle).await;
    let elapsed = started.elapsed().as_millis() as u32;
    IS_SYNCING.store(false, Ordering::Release);

    match result {
        Ok(count) => {
            LAST_SUCCESS_AT.store(now_unix(), Ordering::Relaxed);
            LAST_DURATION_MS.store(elapsed, Ordering::Relaxed);
            LAST_UPDATED_COUNT.store(count as u32, Ordering::Relaxed);
            emit("completed");
            Ok(count as u32)
        }
        Err(e) => {
            emit("error");
            Err(e)
        }
    }
}

pub fn spawn_background_loop(
    handle: TelegramSessionHandle,
    runtime_handle: tokio::runtime::Handle,
    cancel: CancellationToken,
) {
    runtime_handle.spawn(async move {
        loop {
            let interval_secs = (interval_min() as u64).saturating_mul(60).max(60);
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {}
            }
            if !is_enabled() {
                continue;
            }
            let auth_ok = {
                let g = handle.lock().await;
                g.client.is_some()
            };
            if !auth_ok {
                continue;
            }
            if let Err(e) = run_once(&handle).await {
                tracing::warn!("[tg-sync] background sync failed: {}", e);
            }
        }
    });
}
