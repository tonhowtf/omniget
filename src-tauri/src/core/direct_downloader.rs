use std::path::Path;

use anyhow::anyhow;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

pub async fn download_direct(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: mpsc::Sender<f64>,
) -> anyhow::Result<u64> {
    download_direct_with_headers(client, url, output, progress_tx, None).await
}

pub async fn download_direct_with_headers(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: mpsc::Sender<f64>,
    headers: Option<reqwest::header::HeaderMap>,
) -> anyhow::Result<u64> {
    let mut request = client.get(url);
    if let Some(h) = headers {
        request = request.headers(h);
    }
    let response = request.send().await?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "HTTP {} ao baixar {}",
            response.status(),
            url
        ));
    }

    let total_size = response.content_length();

    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut file = tokio::fs::File::create(output).await?;
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        if let Some(total) = total_size {
            if total > 0 {
                let percent = (downloaded as f64 / total as f64) * 100.0;
                let _ = progress_tx.send(percent).await;
            }
        }
    }

    file.flush().await?;
    let _ = progress_tx.send(100.0).await;

    Ok(downloaded)
}
