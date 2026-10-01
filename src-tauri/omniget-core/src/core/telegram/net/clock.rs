use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static OFFSET_SECS: AtomicI64 = AtomicI64::new(0);
static KNOWN: AtomicI64 = AtomicI64::new(0);

pub fn system_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn now_unix() -> i64 {
    system_unix() + OFFSET_SECS.load(Ordering::Relaxed)
}

pub fn observe_server_time(server_unix: i64) {
    let off = server_unix - system_unix();
    KNOWN.store(1, Ordering::Relaxed);
    OFFSET_SECS.store(if off.abs() <= 2 { 0 } else { off }, Ordering::Relaxed);
}

pub fn offset() -> Option<i64> {
    (KNOWN.load(Ordering::Relaxed) == 1).then(|| OFFSET_SECS.load(Ordering::Relaxed))
}
