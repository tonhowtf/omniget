use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::json;

const PROGRESS_THROTTLE: Duration = Duration::from_millis(250);

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);

pub fn emit(event: &str, payload: serde_json::Value) {
    if let Some(h) = crate::core::telegram::host::get() {
        h.emit(event, payload);
    }
}

pub fn next_id() -> u64 {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let bump = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    now_ms.wrapping_mul(1000).wrapping_add(bump % 1000)
}

pub struct DownloadReporter {
    pub id: u64,
    title: String,
    kind: &'static str,
    batch_id: Option<u64>,
    total_bytes: Option<u64>,
    file_path: Option<String>,
    started: Instant,
    last_emit: Option<Instant>,
    last_bytes: u64,
    last_at: Instant,
    speed: f64,
}

impl DownloadReporter {
    pub fn new(
        id: u64,
        title: &str,
        kind: &'static str,
        batch_id: Option<u64>,
        total_bytes: Option<u64>,
        file_path: Option<String>,
    ) -> Self {
        emit(
            "telegram-download-queued",
            json!({
                "id": id,
                "title": title,
                "kind": kind,
                "batch_id": batch_id,
                "total_bytes": total_bytes,
                "file_path": file_path,
            }),
        );
        let now = Instant::now();
        Self {
            id,
            title: title.to_string(),
            kind,
            batch_id,
            total_bytes,
            file_path,
            started: now,
            last_emit: None,
            last_bytes: 0,
            last_at: now,
            speed: 0.0,
        }
    }

    pub fn progress(&mut self, percent: f64) {
        let percent = percent.clamp(0.0, 100.0);
        let now = Instant::now();
        let edge = percent <= 0.0 || percent >= 100.0;
        if !edge {
            if let Some(last) = self.last_emit {
                if now.duration_since(last) < PROGRESS_THROTTLE {
                    return;
                }
            }
        }
        let bytes = self
            .total_bytes
            .map(|t| ((percent / 100.0) * t as f64) as u64)
            .unwrap_or(0);
        let dt = now.duration_since(self.last_at).as_secs_f64();
        if dt > 0.05 && bytes >= self.last_bytes {
            let inst = (bytes - self.last_bytes) as f64 / dt;
            self.speed = if self.speed == 0.0 {
                inst
            } else {
                self.speed * 0.7 + inst * 0.3
            };
            self.last_bytes = bytes;
            self.last_at = now;
        }
        self.last_emit = Some(now);
        emit(
            "telegram-download-progress",
            json!({
                "id": self.id,
                "title": self.title,
                "kind": self.kind,
                "batch_id": self.batch_id,
                "percent": percent,
                "downloaded_bytes": bytes,
                "total_bytes": self.total_bytes,
                "speed_bytes_per_sec": self.speed,
                "file_path": self.file_path,
            }),
        );
    }

    pub fn complete(
        &self,
        success: bool,
        error: Option<String>,
        file_path: Option<String>,
        file_size_bytes: Option<u64>,
        file_count: Option<u32>,
        deleted_count: Option<u32>,
    ) {
        self.complete_with(
            success,
            error,
            file_path,
            file_size_bytes,
            file_count,
            deleted_count,
            serde_json::Value::Null,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_with(
        &self,
        success: bool,
        error: Option<String>,
        file_path: Option<String>,
        file_size_bytes: Option<u64>,
        file_count: Option<u32>,
        deleted_count: Option<u32>,
        extra: serde_json::Value,
    ) {
        let mut payload = json!({
                "id": self.id,
                "title": self.title,
                "kind": self.kind,
                "batch_id": self.batch_id,
                "success": success,
                "error": error,
                "file_path": file_path.or_else(|| self.file_path.clone()),
                "file_size_bytes": file_size_bytes,
                "file_count": file_count,
                "deleted_count": deleted_count,
                "elapsed_ms": self.started.elapsed().as_millis() as u64,
        });
        if let (Some(obj), Some(more)) = (payload.as_object_mut(), extra.as_object()) {
            for (k, v) in more {
                obj.insert(k.clone(), v.clone());
            }
        }
        emit("telegram-download-complete", payload);
    }
}

pub fn batch_file_status(
    batch_id: u64,
    message_id: i32,
    status: &str,
    percent: f64,
    error: Option<String>,
) {
    emit(
        "telegram-batch-file-status",
        json!({
            "batch_id": batch_id,
            "message_id": message_id,
            "status": status,
            "percent": percent,
            "error": error,
        }),
    );
}

pub fn batch_file_duplicate(batch_id: u64, message_id: i32, existing: &str) {
    emit(
        "telegram-batch-file-status",
        json!({
            "batch_id": batch_id,
            "message_id": message_id,
            "status": "duplicate",
            "percent": 100.0,
            "error": null,
            "duplicate_of": existing,
        }),
    );
}
