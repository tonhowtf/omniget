use std::path::Path;
use std::time::Duration;

use anyhow::anyhow;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const CHUNK_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn download_direct(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: mpsc::Sender<f64>,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<u64> {
    download_direct_with_headers(client, url, output, progress_tx, None, cancel).await
}

pub async fn download_direct_with_headers(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: mpsc::Sender<f64>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
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

    // Check for HTML responses (expired/invalid URLs)
    if let Some(ct) = response.headers().get("content-type") {
        if let Ok(ct_str) = ct.to_str() {
            if ct_str.contains("text/html") {
                return Err(anyhow!("Servidor retornou HTML em vez de mídia — URL pode ter expirado"));
            }
        }
    }

    let total_size = response.content_length();

    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut file = tokio::fs::File::create(output).await?;
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    loop {
        if let Some(token) = cancel {
            if token.is_cancelled() {
                drop(file);
                let _ = tokio::fs::remove_file(output).await;
                return Err(anyhow!("Download cancelado"));
            }
        }

        let chunk_result = tokio::time::timeout(CHUNK_TIMEOUT, stream.next()).await;

        match chunk_result {
            Ok(Some(Ok(chunk))) => {
                file.write_all(&chunk).await?;
                downloaded += chunk.len() as u64;

                if let Some(total) = total_size {
                    if total > 0 {
                        let percent = (downloaded as f64 / total as f64) * 100.0;
                        let _ = progress_tx.send(percent).await;
                    }
                }
            }
            Ok(Some(Err(e))) => {
                drop(file);
                let _ = tokio::fs::remove_file(output).await;
                return Err(anyhow!("Erro no stream de download: {}", e));
            }
            Ok(None) => break, // stream finished
            Err(_) => {
                drop(file);
                let _ = tokio::fs::remove_file(output).await;
                return Err(anyhow!("Download timeout — nenhum dado recebido por 30 segundos"));
            }
        }
    }

    file.flush().await?;
    let _ = progress_tx.send(100.0).await;

    Ok(downloaded)
}
