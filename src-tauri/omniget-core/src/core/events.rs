use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum QueueStatus {
    Queued,
    Active,
    Paused,
    Complete { success: bool },
    Error { message: String },
}

#[derive(Clone, Serialize)]
pub struct QueueItemInfo {
    pub id: u64,
    pub url: String,
    pub platform: String,
    pub title: String,
    pub status: QueueStatus,
    pub percent: f64,
    pub speed_bytes_per_sec: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub file_path: Option<String>,
    pub file_size_bytes: Option<u64>,
    pub file_count: Option<u32>,
}

#[derive(Clone, Serialize)]
pub struct QueueItemProgress {
    pub id: u64,
    pub title: String,
    pub platform: String,
    pub percent: f64,
    pub speed_bytes_per_sec: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub phase: String,
}

pub trait EventEmitter: Send + Sync + Clone + 'static {
    fn emit_queue_state(&self, items: &[QueueItemInfo]);
    fn emit_progress(&self, progress: &QueueItemProgress);
}
