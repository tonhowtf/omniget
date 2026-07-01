

use crate::models::progress::ProgressUpdate;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality};
use crate::platforms::traits::PlatformDownloader;

const CHUNK_SIZE: usize = 64 * 1024;

fn relay_addr() -> String {
    std::env::var("OMNIGET_RELAY").unwrap_or_else(|_| "relay.tonho.wtf:9009".to_string())
}

async fn connect_relay() -> anyhow::Result<TcpStream> {
    let addr = relay_addr();
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        TcpStream::connect(&addr),
    )
    .await
    .map_err(|_| anyhow!("Connection to relay timed out (10s)"))?
    .map_err(|e| anyhow!("Failed to connect to relay {}: {}", addr, e))?;
    Ok(stream)
}

async fn read_line(
    reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>,
) -> anyhow::Result<String> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        anyhow::bail!("Relay closed connection unexpectedly");
    }
    Ok(line.trim_end().to_string())
}

fn check_relay_error(line: &str) -> anyhow::Result<()> {
    if let Some(err) = line.strip_prefix("ERROR ") {
        anyhow::bail!("Relay error: {}", err);
    }
    Ok(())
}

pub struct P2pDownloader;

impl P2pDownloader {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl PlatformDownloader for P2pDownloader {
    fn name(&self) -> &str {
        "p2p"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Some(code) = url.strip_prefix("p2p:") {
            return super::p2p_words::is_valid_code(code);
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let code = url
            .strip_prefix("p2p:")
            .ok_or_else(|| anyhow!("Invalid P2P URL: {}", url))?;

        if !super::p2p_words::is_valid_code(code) {
            anyhow::bail!("Invalid share code: {}", code);
        }

        let title = format!("P2P Transfer ({})", &code[..code.len().min(30)]);

        Ok(MediaInfo {
            title,
            author: "P2P Transfer".to_string(),
            platform: "p2p".to_string(),
            duration_seconds: None,
            thumbnail_url: None,
            available_qualities: vec![VideoQuality {
                label: "Original".to_string(),
                width: 0,
                height: 0,
                url: url.to_string(),
                format: "p2p".to_string(),
            }],
            media_type: MediaType::Video,
            file_size_bytes: None,
        })
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        let url = match info.available_qualities.first() {
            Some(q) => &q.url,
            None => anyhow::bail!("No URL found in MediaInfo"),
        };

        let code = url
            .strip_prefix("p2p:")
            .ok_or_else(|| anyhow!("Invalid P2P URL"))?;

        let _ = progress.send(ProgressUpdate::percent(-2.0)).await;

        tracing::info!("[p2p] connecting to relay for code: {}", code);

        let stream = connect_relay().await?;
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);

        write_half
            .write_all(format!("RECV {}\n", code).as_bytes())
            .await?;
        write_half.flush().await?;

        let response = read_line(&mut reader).await?;
        check_relay_error(&response)?;
        if response != "READY" {
            anyhow::bail!("Unexpected relay response: {}", response);
        }

        tracing::info!("[p2p] connected to sender via relay");

        let file_name = read_line(&mut reader).await?;
        let file_size_str = read_line(&mut reader).await?;
        let file_size: u64 = file_size_str
            .parse()
            .map_err(|_| anyhow!("Invalid file size from sender: {}", file_size_str))?;

        tracing::info!("[p2p] receiving: {} ({} bytes)", file_name, file_size);

        write_half.write_all(b"OK\n").await?;
        write_half.flush().await?;

        let _ = progress.send(ProgressUpdate::percent(0.0)).await;

        let sanitized = sanitize_filename::sanitize(&file_name);
        let output_path = opts.output_dir.join(&sanitized);
        if let Some(parent) = output_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = File::create(&output_path).await?;
        let mut received: u64 = 0;
        let mut buf = vec![0u8; CHUNK_SIZE];

        while received < file_size {
            if opts.cancel_token.is_cancelled() {
                let _ = tokio::fs::remove_file(&output_path).await;
                anyhow::bail!("Download cancelled");
            }

            let to_read = ((file_size - received) as usize).min(CHUNK_SIZE);
            let n = reader.read(&mut buf[..to_read]).await?;
            if n == 0 {
                break;
            }

            file.write_all(&buf[..n]).await?;
            received += n as u64;

            if file_size > 0 {
                let pct = (received as f64 / file_size as f64) * 100.0;
                let _ = progress.send(ProgressUpdate::percent(pct)).await;
            }
        }

        file.flush().await?;
        drop(file);

        let _ = progress.send(ProgressUpdate::percent(100.0)).await;

        tracing::info!("[p2p] download complete: {}", output_path.display());

        Ok(DownloadResult {
            file_path: output_path,
            file_size_bytes: received,
            duration_seconds: 0.0,
            torrent_id: None,
        })
    }
}

pub struct P2pSendSession {
    pub code: String,
    pub file_path: PathBuf,
    pub file_name: String,
    pub file_size: u64,
    pub cancel_token: CancellationToken,
    pub progress: Arc<tokio::sync::Mutex<f64>>,
    pub status: Arc<tokio::sync::Mutex<String>>,
    pub sent_bytes: Arc<tokio::sync::Mutex<u64>>,
    pub paused: Arc<std::sync::atomic::AtomicBool>,
}

pub async fn start_send(
    file_path: PathBuf,
    cancel_token: CancellationToken,
) -> anyhow::Result<P2pSendSession> {
    let metadata = tokio::fs::metadata(&file_path)
        .await
        .map_err(|e| anyhow!("File not found: {}", e))?;

    if !metadata.is_file() {
        anyhow::bail!("Path is not a file: {}", file_path.display());
    }

    let file_size = metadata.len();
    let file_name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    let code = super::p2p_words::generate_code();

    tracing::info!("[p2p] share code generated: {}", code);

    Ok(P2pSendSession {
        code,
        file_path,
        file_name,
        file_size,
        cancel_token,
        progress: Arc::new(tokio::sync::Mutex::new(0.0)),
        status: Arc::new(tokio::sync::Mutex::new("waiting".to_string())),
        sent_bytes: Arc::new(tokio::sync::Mutex::new(0)),
        paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    })
}

pub async fn run_sender(session: &P2pSendSession) -> anyhow::Result<()> {
    let cancel = session.cancel_token.clone();

    *session.status.lock().await = "connecting".to_string();

    tracing::info!("[p2p] connecting to relay...");

    let stream = connect_relay().await?;
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);

    write_half
        .write_all(format!("SEND {}\n", session.code).as_bytes())
        .await?;
    write_half.flush().await?;

    let response = read_line(&mut reader).await?;
    check_relay_error(&response)?;
    if response != "WAIT" {
        anyhow::bail!("Unexpected relay response: {}", response);
    }

    *session.status.lock().await = "waiting_for_receiver".to_string();
    tracing::info!("[p2p] waiting for receiver... code: {}", session.code);

    let ready = tokio::select! {
        line = read_line(&mut reader) => line?,
        _ = cancel.cancelled() => {
            anyhow::bail!("Send cancelled while waiting for receiver");
        }
    };

    check_relay_error(&ready)?;
    if ready != "READY" {
        anyhow::bail!("Unexpected relay response: {}", ready);
    }

    *session.status.lock().await = "connected".to_string();
    tracing::info!("[p2p] receiver connected");

    let header = format!("{}\n{}\n", session.file_name, session.file_size);
    write_half.write_all(header.as_bytes()).await?;
    write_half.flush().await?;

    let ok_response = tokio::select! {
        line = read_line(&mut reader) => line?,
        _ = cancel.cancelled() => {
            anyhow::bail!("Send cancelled while waiting for OK");
        }
    };

    if ok_response != "OK" {
        anyhow::bail!("Receiver rejected transfer: {}", ok_response);
    }

    *session.status.lock().await = "transferring".to_string();
    tracing::info!(
        "[p2p] transferring: {} ({} bytes)",
        session.file_name,
        session.file_size
    );

    let mut file = File::open(&session.file_path).await?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut sent: u64 = 0;

    loop {
        if cancel.is_cancelled() {
            anyhow::bail!("Send cancelled during transfer");
        }

        while session.paused.load(std::sync::atomic::Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if cancel.is_cancelled() {
                anyhow::bail!("Send cancelled while paused");
            }
        }

        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }

        write_half.write_all(&buf[..n]).await?;
        sent += n as u64;

        *session.sent_bytes.lock().await = sent;
        if session.file_size > 0 {
            *session.progress.lock().await = (sent as f64 / session.file_size as f64) * 100.0;
        }
    }

    write_half.flush().await?;
    drop(write_half);

    *session.progress.lock().await = 100.0;
    *session.status.lock().await = "complete".to_string();

    tracing::info!("[p2p] transfer complete: {} bytes sent", sent);

    Ok(())
}
