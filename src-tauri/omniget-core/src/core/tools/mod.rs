pub mod ai_keys;
pub mod github;
pub mod onnx;
pub mod pricing;
pub mod usage;

use std::sync::Arc;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ToolProgress {
    pub id: String,
    pub stage: String,
    pub done: u64,
    pub total: Option<u64>,
    pub message: Option<String>,
}

pub type ProgressFn = Arc<dyn Fn(ToolProgress) + Send + Sync>;

pub fn noop_progress() -> ProgressFn {
    Arc::new(|_| {})
}

pub fn report(
    p: &ProgressFn,
    id: &str,
    stage: &str,
    done: u64,
    total: Option<u64>,
    message: Option<String>,
) {
    p(ToolProgress {
        id: id.to_string(),
        stage: stage.to_string(),
        done,
        total,
        message,
    });
}

pub fn tools_dir() -> Option<std::path::PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("tools"))
}

pub fn temp_dir() -> std::path::PathBuf {
    let base = tools_dir().unwrap_or_else(std::env::temp_dir).join("tmp");
    let _ = std::fs::create_dir_all(&base);
    base
}

pub fn client() -> anyhow::Result<reqwest::Client> {
    use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
        ),
    );
    Ok(
        crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(600))
            .build()?,
    )
}

pub async fn download_to(
    client: &reqwest::Client,
    url: &str,
    dest: &std::path::Path,
    progress: &ProgressFn,
    id: &str,
) -> anyhow::Result<u64> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("download of {} failed: HTTP {}", url, resp.status());
    }
    let total = resp.content_length();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&part).await?;
    let mut stream = resp.bytes_stream();
    let mut done: u64 = 0;
    let mut last = std::time::Instant::now();
    report(progress, id, "started", 0, total, None);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        if last.elapsed() > std::time::Duration::from_millis(200) {
            report(progress, id, "progress", done, total, None);
            last = std::time::Instant::now();
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&part, dest).await?;
    report(progress, id, "done", done, total, None);
    Ok(done)
}
