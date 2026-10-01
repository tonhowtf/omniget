pub mod ledger;
pub mod schema;
pub mod transport;

use std::sync::{Arc, OnceLock};

pub use ledger::{Actor, Assessment, Budget, Intent, Ledger, Quote, Reserve};
pub use schema::{GenRequest, JobStatus, ModelSchema, ModelSummary, ParamSpec};
pub use transport::{CliTransport, MediaTransport};

pub const DB_FILE: &str = "canvas-media.sqlite";

static LEDGER: OnceLock<Arc<Ledger>> = OnceLock::new();

pub fn ledger() -> Result<Arc<Ledger>, String> {
    if let Some(l) = LEDGER.get() {
        return Ok(l.clone());
    }
    let dir =
        crate::core::paths::app_data_dir().ok_or("PROVIDER_STORAGE_ERROR: no app data folder")?;
    let l = Arc::new(Ledger::open(&dir.join(DB_FILE))?);
    Ok(LEDGER.get_or_init(|| l).clone())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

pub struct Fetched {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub sha256: String,
}

pub async fn fetch_result(url: &str) -> Result<Fetched, String> {
    let c = crate::core::assist::media_retrieval::collect_bytes(url).await?;
    if !c.mime.starts_with("image/") {
        return Err("PROVIDER_RESULT_NOT_AN_IMAGE".into());
    }
    Ok(Fetched {
        bytes: c.bytes,
        mime: c.mime,
        sha256: c.digest,
    })
}

pub fn ext_for(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "bin",
    }
}

pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}
