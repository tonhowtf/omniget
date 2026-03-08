use std::path::Path;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAGIC: &[u8; 4] = b"omni";
const MAX_FRAME_SIZE: u32 = 4 * 1024 * 1024;
pub const CHUNK_SIZE: usize = 64 * 1024;
pub const DISCOVERY_PORT: u16 = 53317;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    Hello { code_hash: String },
    FileInfo {
        name: String,
        size: u64,
        file_count: u32,
    },
    Accept,
    Reject,
    Done,
    Error { message: String },
}

pub fn hash_code(code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"omniget-p2p-v1:");
    hasher.update(code.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}

pub async fn write_message(stream: &mut TcpStream, msg: &Message) -> Result<()> {
    let payload = serde_json::to_vec(msg)?;
    let len = payload.len() as u32;
    if len > MAX_FRAME_SIZE {
        return Err(anyhow!("Message too large: {} bytes", len));
    }

    stream.write_all(MAGIC).await?;
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&payload).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn read_message(stream: &mut TcpStream) -> Result<Message> {
    let mut magic = [0u8; 4];
    stream
        .read_exact(&mut magic)
        .await
        .context("Failed to read magic bytes")?;
    if &magic != MAGIC {
        return Err(anyhow!(
            "Invalid magic bytes: expected {:?}, got {:?}",
            MAGIC,
            magic
        ));
    }

    let mut len_bytes = [0u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .await
        .context("Failed to read length")?;
    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_FRAME_SIZE {
        return Err(anyhow!("Frame too large: {} bytes", len));
    }

    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .context("Failed to read payload")?;

    serde_json::from_slice(&payload).context("Failed to parse message")
}

pub async fn write_data_frame(stream: &mut TcpStream, data: &[u8]) -> Result<()> {
    let len = data.len() as u32;
    stream.write_all(MAGIC).await?;
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(data).await?;
    Ok(())
}

pub async fn read_data_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut magic = [0u8; 4];
    stream
        .read_exact(&mut magic)
        .await
        .context("Connection closed during transfer")?;
    if &magic != MAGIC {
        return Err(anyhow!("Invalid magic bytes in data frame"));
    }

    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_FRAME_SIZE {
        return Err(anyhow!("Data frame too large: {} bytes", len));
    }

    if len == 0 {
        return Ok(Vec::new());
    }

    let mut data = vec![0u8; len as usize];
    stream.read_exact(&mut data).await?;
    Ok(data)
}

pub fn encode_discovery_packet(code: &str, tcp_port: u16, _filename: &str, _file_size: u64) -> Vec<u8> {
    let ch = hash_code(code);
    format!("OMNIGET\n{}\n{}\n-\n0", ch, tcp_port).into_bytes()
}

pub struct DiscoveryPacket {
    pub code_hash: String,
    pub tcp_port: u16,
    pub filename: String,
    pub file_size: u64,
}

pub fn parse_discovery_packet(data: &[u8]) -> Option<DiscoveryPacket> {
    let s = std::str::from_utf8(data).ok()?;
    let mut lines = s.lines();

    if lines.next()? != "OMNIGET" {
        return None;
    }

    let code_hash = lines.next()?.to_string();
    let tcp_port: u16 = lines.next()?.parse().ok()?;
    let filename = lines.next()?.to_string();
    let file_size: u64 = lines.next()?.parse().ok()?;

    Some(DiscoveryPacket {
        code_hash,
        tcp_port,
        filename,
        file_size,
    })
}

pub fn safe_filename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string())
}
