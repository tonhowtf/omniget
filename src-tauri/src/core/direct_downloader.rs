use std::path::Path;
use std::time::Duration;

use anyhow::anyhow;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const CHUNK_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RETRIES: u32 = 3;

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
    let mut last_err = None;

    for attempt in 0..MAX_RETRIES {
        if let Some(token) = cancel {
            if token.is_cancelled() {
                return Err(anyhow!("Download cancelado"));
            }
        }

        if attempt > 0 {
            let base = 1000 * (attempt as u64);
            let jitter = rand::random::<u64>() % (base / 2 + 1);
            tokio::time::sleep(Duration::from_millis(base + jitter)).await;
        }

        match download_attempt(client, url, output, &progress_tx, headers.clone(), cancel).await {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                if is_fatal_error(&e) {
                    let part_path = part_path_for(output);
                    let _ = tokio::fs::remove_file(&part_path).await;
                    return Err(e);
                }
                tracing::warn!(
                    "[direct] attempt {}/{} failed: {}",
                    attempt + 1,
                    MAX_RETRIES,
                    e
                );
                last_err = Some(e);
            }
        }
    }

    let part_path = part_path_for(output);
    let _ = tokio::fs::remove_file(&part_path).await;

    Err(last_err.unwrap_or_else(|| anyhow!("Download falhou após {} tentativas", MAX_RETRIES)))
}

fn part_path_for(output: &Path) -> std::path::PathBuf {
    let mut part = output.as_os_str().to_owned();
    part.push(".part");
    std::path::PathBuf::from(part)
}

fn is_fatal_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    for code in &["HTTP 400", "HTTP 401", "HTTP 403", "HTTP 404", "HTTP 405",
                   "HTTP 410", "HTTP 451"] {
        if msg.contains(code) {
            return true;
        }
    }
    if msg.contains("HTML em vez de mídia") {
        return true;
    }
    if msg.contains("cancelado") {
        return true;
    }
    false
}

async fn download_attempt(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: &mpsc::Sender<f64>,
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

    let part_path = part_path_for(output);

    let mut file = tokio::io::BufWriter::with_capacity(
        256 * 1024,
        tokio::fs::File::create(&part_path).await?,
    );
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    loop {
        if let Some(token) = cancel {
            if token.is_cancelled() {
                drop(file);
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(anyhow!("Download cancelado"));
            }
        }

        let chunk_result = tokio::time::timeout(CHUNK_TIMEOUT, stream.next()).await;

        match chunk_result {
            Ok(Some(Ok(chunk))) => {
                file.write_all(&chunk).await.map_err(|e| {
                    anyhow!("Erro de escrita (disco cheio?): {}", e)
                })?;
                downloaded += chunk.len() as u64;

                if let Some(total) = total_size {
                    if total > 0 {
                        let percent = (downloaded as f64 / total as f64) * 100.0;
                        let _ = progress_tx.send(percent).await;
                    }
                } else {
                    let percent = (downloaded as f64 / (downloaded as f64 + 500_000.0)) * 100.0;
                    let _ = progress_tx.send(percent.min(95.0)).await;
                }
            }
            Ok(Some(Err(e))) => {
                drop(file);
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(anyhow!("Erro no stream de download: {}", e));
            }
            Ok(None) => break,
            Err(_) => {
                drop(file);
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(anyhow!("Download timeout — nenhum dado recebido por 30 segundos"));
            }
        }
    }

    file.flush().await?;
    drop(file);

    if let Some(expected) = total_size {
        if expected > 0 && downloaded != expected {
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(anyhow!(
                "Tamanho incorreto: esperado {} bytes, recebido {}",
                expected,
                downloaded
            ));
        }
    }

    tokio::fs::rename(&part_path, output).await?;

    let _ = progress_tx.send(100.0).await;

    Ok(downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn part_path_appends_suffix() {
        let output = Path::new("video.mp4");
        let part = part_path_for(output);
        assert_eq!(part, PathBuf::from("video.mp4.part"));
    }

    #[test]
    fn part_path_no_extension() {
        let output = Path::new("video");
        let part = part_path_for(output);
        assert_eq!(part, PathBuf::from("video.part"));
    }

    #[test]
    fn part_path_nested() {
        let output = Path::new("downloads/curso/aula.mp4");
        let part = part_path_for(output);
        assert_eq!(part, PathBuf::from("downloads/curso/aula.mp4.part"));
    }

    #[test]
    fn is_fatal_http_400() {
        assert!(is_fatal_error(&anyhow!("HTTP 400 ao baixar url")));
    }

    #[test]
    fn is_fatal_http_401() {
        assert!(is_fatal_error(&anyhow!("HTTP 401 ao baixar url")));
    }

    #[test]
    fn is_fatal_http_403() {
        assert!(is_fatal_error(&anyhow!("HTTP 403 ao baixar url")));
    }

    #[test]
    fn is_fatal_http_404() {
        assert!(is_fatal_error(&anyhow!("HTTP 404 ao baixar url")));
    }

    #[test]
    fn is_fatal_html_response() {
        assert!(is_fatal_error(&anyhow!(
            "Servidor retornou HTML em vez de mídia — URL pode ter expirado"
        )));
    }

    #[test]
    fn is_fatal_cancelled() {
        assert!(is_fatal_error(&anyhow!("Download cancelado")));
    }

    #[test]
    fn is_not_fatal_500() {
        assert!(!is_fatal_error(&anyhow!("HTTP 500 Internal Server Error")));
    }

    #[test]
    fn is_not_fatal_502() {
        assert!(!is_fatal_error(&anyhow!("HTTP 502 Bad Gateway")));
    }

    #[test]
    fn is_not_fatal_timeout() {
        assert!(!is_fatal_error(&anyhow!("connection timed out")));
    }

    #[test]
    fn is_not_fatal_network() {
        assert!(!is_fatal_error(&anyhow!("network error")));
    }
}
