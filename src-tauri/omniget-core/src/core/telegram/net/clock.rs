//! Server-corrected clock. Fake-TLS proxies reject hellos whose timestamp
//! is off by more than a few seconds, and MTProto rejects message ids from
//! a clock that runs ahead ("msg_id too high"): a wrong system clock is one
//! of the most common "stuck connecting" causes in the issue trackers. Every
//! successful probe learns the server time from its `msg_id` and we keep the
//! offset here.

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

/// Records the server's clock (seconds). Small drifts are ignored.
pub fn observe_server_time(server_unix: i64) {
    let off = server_unix - system_unix();
    KNOWN.store(1, Ordering::Relaxed);
    OFFSET_SECS.store(if off.abs() <= 2 { 0 } else { off }, Ordering::Relaxed);
}

/// Offset from the system clock, when a server has told us the time.
pub fn offset() -> Option<i64> {
    (KNOWN.load(Ordering::Relaxed) == 1).then(|| OFFSET_SECS.load(Ordering::Relaxed))
}
