use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use std::time::Duration;

use anyhow::anyhow;
use futures::StreamExt;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

const CHUNK_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_RETRIES: u32 = 3;
const CHUNK_SIZE: u64 = 10 * 1024 * 1024;
const CHUNK_THRESHOLD: u64 = 10 * 1024 * 1024;
const MAX_PARALLEL: usize = 12;
const MAX_PER_HOST: usize = 16;

fn host_semaphores() -> &'static tokio::sync::Mutex<HashMap<String, Arc<Semaphore>>> {
    static MAP: OnceLock<tokio::sync::Mutex<HashMap<String, Arc<Semaphore>>>> = OnceLock::new();
    MAP.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

pub async fn get_host_semaphore(url: &str) -> Arc<Semaphore> {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default();
    let mut map = host_semaphores().lock().await;
    map.entry(host)
        .or_insert_with(|| Arc::new(Semaphore::new(MAX_PER_HOST)))
        .clone()
}

struct ProbeResult {
    content_length: Option<u64>,
    accept_ranges: bool,
}

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
                    let _ = tokio::fs::remove_file(&part_path_for(output)).await;
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

    let _ = tokio::fs::remove_file(&part_path_for(output)).await;
    Err(last_err.unwrap_or_else(|| anyhow!("Download falhou após {} tentativas", MAX_RETRIES)))
}

fn part_path_for(output: &Path) -> PathBuf {
    let mut part = output.as_os_str().to_owned();
    part.push(".part");
    PathBuf::from(part)
}

fn is_fatal_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    for code in &[
        "HTTP 400", "HTTP 401", "HTTP 403", "HTTP 404", "HTTP 405",
        "HTTP 410", "HTTP 451",
    ] {
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

async fn probe_url(
    client: &reqwest::Client,
    url: &str,
    headers: Option<&reqwest::header::HeaderMap>,
) -> ProbeResult {
    let mut request = client.head(url);
    if let Some(h) = headers {
        request = request.headers(h.clone());
    }
    match tokio::time::timeout(Duration::from_secs(15), request.send()).await {
        Ok(Ok(resp)) if resp.status().is_success() => {
            let content_length = resp.content_length();
            let accept_ranges = resp
                .headers()
                .get("accept-ranges")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.contains("bytes"))
                .unwrap_or(false);
            ProbeResult {
                content_length,
                accept_ranges,
            }
        }
        _ => ProbeResult {
            content_length: None,
            accept_ranges: false,
        },
    }
}

async fn download_attempt(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: &mpsc::Sender<f64>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<u64> {
    let part_path = part_path_for(output);
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let probe = probe_url(client, url, headers.as_ref()).await;

    let use_chunked = probe.accept_ranges
        && probe.content_length.is_some_and(|s| s > CHUNK_THRESHOLD);

    if use_chunked {
        let total = probe.content_length.unwrap();
        let _ = tokio::fs::remove_file(&part_path).await;
        if let Err(chunked_err) = download_chunked(
            client, url, &part_path, total, progress_tx, headers.clone(), cancel,
        )
        .await
        {
            if is_fatal_error(&chunked_err) {
                return Err(chunked_err);
            }
            let _ = tokio::fs::remove_file(&part_path).await;
            tracing::warn!("[direct] chunked failed, falling back: {}", chunked_err);
            download_single_stream(
                client, url, &part_path, 0, probe.content_length, progress_tx, headers, cancel,
            )
            .await?;
        }
    } else {
        let existing = match tokio::fs::metadata(&part_path).await {
            Ok(m) if m.len() > 0 && probe.accept_ranges => m.len(),
            _ => 0,
        };
        download_single_stream(
            client, url, &part_path, existing, probe.content_length, progress_tx, headers, cancel,
        )
        .await?;
    }

    if let Some(expected) = probe.content_length {
        let actual = tokio::fs::metadata(&part_path).await?.len();
        if expected > 0 && actual != expected {
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(anyhow!(
                "Tamanho incorreto: esperado {} bytes, recebido {}",
                expected,
                actual
            ));
        }
    }

    tokio::fs::rename(&part_path, output).await?;
    let _ = progress_tx.send(100.0).await;

    let size = tokio::fs::metadata(output).await?.len();
    Ok(size)
}

async fn download_chunked(
    client: &reqwest::Client,
    url: &str,
    part_path: &Path,
    total_size: u64,
    progress_tx: &mpsc::Sender<f64>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<()> {
    let file = tokio::fs::File::create(part_path).await?;
    file.set_len(total_size).await?;
    drop(file);

    let num_chunks = total_size.div_ceil(CHUNK_SIZE);
    let downloaded = Arc::new(AtomicU64::new(0));
    let semaphore = Arc::new(Semaphore::new(MAX_PARALLEL));
    let cancel_token = match cancel {
        Some(ct) => ct.clone(),
        None => CancellationToken::new(),
    };

    let mut join_set = tokio::task::JoinSet::new();
    for i in 0..num_chunks {
        let start = i * CHUNK_SIZE;
        let end = ((i + 1) * CHUNK_SIZE).min(total_size) - 1;
        let client = client.clone();
        let url = url.to_string();
        let path = part_path.to_owned();
        let sem = semaphore.clone();
        let dl = downloaded.clone();
        let ptx = progress_tx.clone();
        let ct = cancel_token.clone();
        let hdrs = headers.clone();

        join_set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if ct.is_cancelled() {
                return Err(anyhow!("Download cancelado"));
            }
            download_chunk(&client, &url, &path, start, end, &dl, total_size, &ptx, hdrs, &ct)
                .await
        });
    }

    while let Some(result) = join_set.join_next().await {
        if cancel_token.is_cancelled() {
            join_set.abort_all();
            return Err(anyhow!("Download cancelado"));
        }
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                join_set.abort_all();
                return Err(e);
            }
            Err(e) => {
                join_set.abort_all();
                return Err(anyhow!("Chunk task falhou: {:?}", e));
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_chunk(
    client: &reqwest::Client,
    url: &str,
    part_path: &Path,
    start: u64,
    end: u64,
    downloaded: &AtomicU64,
    total_size: u64,
    progress_tx: &mpsc::Sender<f64>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let mut request = client.get(url);
    if let Some(h) = headers {
        request = request.headers(h);
    }
    request = request.header("Range", format!("bytes={}-{}", start, end));

    let response = tokio::time::timeout(Duration::from_secs(30), request.send())
        .await
        .map_err(|_| anyhow!("Timeout ao conectar para chunk"))??;

    let status = response.status();
    if status != reqwest::StatusCode::PARTIAL_CONTENT {
        if status.is_success() {
            return Err(anyhow!("Servidor não suportou Range request (HTTP 200)"));
        }
        return Err(anyhow!("HTTP {} ao baixar chunk", status));
    }

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(part_path)
        .await?;
    file.seek(std::io::SeekFrom::Start(start)).await?;

    let expected_size = end - start + 1;
    let mut chunk_written: u64 = 0;
    let mut stream = response.bytes_stream();

    loop {
        if cancel.is_cancelled() {
            return Err(anyhow!("Download cancelado"));
        }

        let chunk_result = tokio::time::timeout(CHUNK_TIMEOUT, stream.next()).await;
        match chunk_result {
            Ok(Some(Ok(data))) => {
                file.write_all(&data).await.map_err(|e| {
                    anyhow!("Erro de escrita (disco cheio?): {}", e)
                })?;
                chunk_written += data.len() as u64;
                let total_dl =
                    downloaded.fetch_add(data.len() as u64, Ordering::Relaxed) + data.len() as u64;
                let percent = (total_dl as f64 / total_size as f64) * 100.0;
                let _ = progress_tx.send(percent.min(99.9)).await;
            }
            Ok(Some(Err(e))) => return Err(anyhow!("Erro no stream de chunk: {}", e)),
            Ok(None) => break,
            Err(_) => {
                return Err(anyhow!(
                    "Chunk timeout — nenhum dado recebido por 30 segundos"
                ))
            }
        }
    }

    if chunk_written != expected_size {
        return Err(anyhow!(
            "Chunk incompleto: esperado {} bytes, recebido {}",
            expected_size,
            chunk_written
        ));
    }

    file.flush().await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_single_stream(
    client: &reqwest::Client,
    url: &str,
    part_path: &Path,
    existing_bytes: u64,
    total_size: Option<u64>,
    progress_tx: &mpsc::Sender<f64>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<()> {
    let mut request = client.get(url);
    if let Some(h) = headers {
        request = request.headers(h);
    }

    if existing_bytes > 0 {
        if let Some(total) = total_size {
            if existing_bytes >= total {
                return Ok(());
            }
        }
        request = request.header("Range", format!("bytes={}-", existing_bytes));
    }

    let response = request.send().await?;

    let mut offset = 0u64;
    if existing_bytes > 0 {
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            offset = existing_bytes;
        } else if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            let _ = tokio::fs::remove_file(part_path).await;
            return Err(anyhow!("Range não satisfatível, reiniciando"));
        } else if !response.status().is_success() {
            return Err(anyhow!("HTTP {} ao baixar {}", response.status(), url));
        }
    } else if !response.status().is_success() {
        return Err(anyhow!("HTTP {} ao baixar {}", response.status(), url));
    }

    if let Some(ct) = response.headers().get("content-type") {
        if let Ok(ct_str) = ct.to_str() {
            if ct_str.contains("text/html") {
                return Err(anyhow!(
                    "Servidor retornou HTML em vez de mídia — URL pode ter expirado"
                ));
            }
        }
    }

    let file = if offset > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(part_path)
            .await?
    } else {
        tokio::fs::File::create(part_path).await?
    };

    let mut file = tokio::io::BufWriter::with_capacity(256 * 1024, file);
    let mut downloaded = offset;
    let mut stream = response.bytes_stream();

    loop {
        if let Some(token) = cancel {
            if token.is_cancelled() {
                file.flush().await?;
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
                    let percent =
                        (downloaded as f64 / (downloaded as f64 + 500_000.0)) * 100.0;
                    let _ = progress_tx.send(percent.min(95.0)).await;
                }
            }
            Ok(Some(Err(e))) => {
                file.flush().await?;
                return Err(anyhow!("Erro no stream de download: {}", e));
            }
            Ok(None) => break,
            Err(_) => {
                file.flush().await?;
                return Err(anyhow!(
                    "Download timeout — nenhum dado recebido por 30 segundos"
                ));
            }
        }
    }

    file.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn chunk_count_for_12mb() {
        let total: u64 = 12 * 1024 * 1024;
        assert_eq!(total.div_ceil(CHUNK_SIZE), 2);
    }

    #[test]
    fn chunk_count_exact_boundary() {
        assert_eq!(CHUNK_SIZE.div_ceil(CHUNK_SIZE), 1);
    }

    #[test]
    fn chunk_count_single_byte_over() {
        assert_eq!((CHUNK_SIZE + 1).div_ceil(CHUNK_SIZE), 2);
    }

    #[test]
    fn threshold_gte_chunk_size() {
        assert!(CHUNK_THRESHOLD >= CHUNK_SIZE);
    }
}
