use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use futures::future::join_all;
use m3u8_rs::{parse_master_playlist, parse_media_playlist, MasterPlaylist, VariantStream};
use reqwest::Client;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub struct HlsDownloadResult {
    pub path: PathBuf,
    pub file_size: u64,
    pub segments: usize,
}

pub struct HlsDownloader {
    client: Client,
}

impl HlsDownloader {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .danger_accept_invalid_certs(true)
                .build()
                .unwrap(),
        }
    }

    pub async fn download(
        &self,
        m3u8_url: &str,
        output_path: &str,
        referer: &str,
        bytes_tx: Option<UnboundedSender<u64>>,
        cancel_token: CancellationToken,
    ) -> anyhow::Result<HlsDownloadResult> {
        if cancel_token.is_cancelled() {
            anyhow::bail!("Download cancelado pelo usuário");
        }

        let m3u8_text = self
            .client
            .get(m3u8_url)
            .header("Referer", referer)
            .header("User-Agent", USER_AGENT)
            .send()
            .await?
            .text()
            .await?;

        let m3u8_bytes = m3u8_text.as_bytes();

        if let Ok((_, master)) = parse_master_playlist(m3u8_bytes) {
            if let Some(variant) = select_best_variant(&master) {
                let variant_url = resolve_url(m3u8_url, &variant.uri);
                tracing::info!(
                    "[hls] Variante selecionada: {}x{} @ {} bps",
                    variant.resolution.as_ref().map(|r| r.width).unwrap_or(0),
                    variant.resolution.as_ref().map(|r| r.height).unwrap_or(0),
                    variant.bandwidth
                );
                return self
                    .download_media_playlist(&variant_url, output_path, referer, bytes_tx, cancel_token)
                    .await;
            }
        }

        if parse_media_playlist(m3u8_bytes).is_ok() {
            return self
                .download_media_playlist(m3u8_url, output_path, referer, bytes_tx, cancel_token)
                .await;
        }

        anyhow::bail!("Falha ao parsear m3u8: nem master nem media playlist")
    }

    async fn download_media_playlist(
        &self,
        m3u8_url: &str,
        output_path: &str,
        referer: &str,
        bytes_tx: Option<UnboundedSender<u64>>,
        cancel_token: CancellationToken,
    ) -> anyhow::Result<HlsDownloadResult> {
        let resp = self
            .client
            .get(m3u8_url)
            .header("Referer", referer)
            .header("User-Agent", USER_AGENT)
            .send()
            .await?;

        let text = resp.text().await?;

        let (_, playlist) = parse_media_playlist(text.as_bytes())
            .map_err(|e| anyhow::anyhow!("Parse media playlist: {:?}", e))?;

        let total_segments = playlist.segments.len();
        tracing::info!("[download] {} segmentos para baixar", total_segments);

        let encryption = self
            .fetch_encryption_info(&playlist, m3u8_url, referer)
            .await?;
        if encryption.is_some() {
            tracing::info!("[download] Segmentos encriptados com AES-128");
        }

        let semaphore = Arc::new(Semaphore::new(20));
        let completed = Arc::new(AtomicUsize::new(0));

        let tasks: Vec<_> = playlist
            .segments
            .iter()
            .enumerate()
            .map(|(i, segment)| {
                let sem = semaphore.clone();
                let client = self.client.clone();
                let url = resolve_url(m3u8_url, &segment.uri);
                let referer = referer.to_string();
                let completed = completed.clone();
                let total = total_segments;
                let bytes_tx = bytes_tx.clone();
                let ct = cancel_token.clone();

                tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    if ct.is_cancelled() {
                        anyhow::bail!("Download cancelado pelo usuário");
                    }
                    let data: Vec<u8> =
                        download_segment_with_retry(&client, &url, &referer).await?;
                    if let Some(ref tx) = bytes_tx {
                        let _ = tx.send(data.len() as u64);
                    }
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    tracing::info!(
                        "[download] Segmento {}/{} baixado ({} KB)",
                        done,
                        total,
                        data.len() / 1024
                    );
                    Ok::<(usize, Vec<u8>), anyhow::Error>((i, data))
                })
            })
            .collect();

        let results = join_all(tasks).await;

        if cancel_token.is_cancelled() {
            anyhow::bail!("Download cancelado pelo usuário");
        }

        let mut segments_data: Vec<(usize, Vec<u8>)> = Vec::with_capacity(total_segments);
        for result in results {
            let (idx, data) = result??;
            segments_data.push((idx, data));
        }
        segments_data.sort_by_key(|(idx, _)| *idx);

        let output = PathBuf::from(output_path);
        if let Some(parent) = output.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::fs::File::create(&output).await?;

        if let Some(enc) = &encryption {
            use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
            type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

            let media_sequence = playlist.media_sequence;

            for (i, (_, data)) in segments_data.iter().enumerate() {
                let iv = compute_iv(enc, i, media_sequence);
                let mut buf = data.clone();
                let decryptor = Aes128CbcDec::new_from_slices(&enc.key_bytes, &iv)
                    .map_err(|e| anyhow::anyhow!("AES init: {:?}", e))?;
                let decrypted = decryptor
                    .decrypt_padded_mut::<Pkcs7>(&mut buf)
                    .map_err(|e| anyhow::anyhow!("AES decrypt: {:?}", e))?;
                file.write_all(decrypted).await?;
            }
        } else {
            for (_, data) in &segments_data {
                file.write_all(data).await?;
            }
        }

        file.flush().await?;

        let file_size = tokio::fs::metadata(&output).await?.len();
        tracing::info!(
            "[download] HLS download completo: {} ({:.1} MB, {} segmentos)",
            output.display(),
            file_size as f64 / (1024.0 * 1024.0),
            total_segments
        );

        Ok(HlsDownloadResult {
            path: output,
            file_size,
            segments: total_segments,
        })
    }

    async fn fetch_encryption_info(
        &self,
        playlist: &m3u8_rs::MediaPlaylist,
        m3u8_url: &str,
        referer: &str,
    ) -> anyhow::Result<Option<EncryptionInfo>> {
        for segment in &playlist.segments {
            if let Some(key) = &segment.key {
                if matches!(key.method, m3u8_rs::KeyMethod::AES128) {
                    if let Some(uri) = &key.uri {
                        let key_url = resolve_url(m3u8_url, uri);
                        tracing::info!("[download] Baixando chave AES-128: {}", key_url);
                        let key_bytes = self
                            .client
                            .get(&key_url)
                            .header("Referer", referer)
                            .header("User-Agent", USER_AGENT)
                            .send()
                            .await?
                            .bytes()
                            .await?
                            .to_vec();

                        let iv = key.iv.as_ref().map(|iv_str| parse_hex_iv(iv_str));

                        return Ok(Some(EncryptionInfo { key_bytes, iv }));
                    }
                }
            }
        }
        Ok(None)
    }
}

struct EncryptionInfo {
    key_bytes: Vec<u8>,
    iv: Option<[u8; 16]>,
}

fn select_best_variant(master: &MasterPlaylist) -> Option<&VariantStream> {
    let real: Vec<&VariantStream> = master
        .variants
        .iter()
        .filter(|v| !v.is_i_frame)
        .collect();

    if real.is_empty() {
        return None;
    }

    let mut sorted = real;
    sorted.sort_by_key(|v| v.resolution.as_ref().map(|r| r.height).unwrap_or(0));

    let mut best: Option<&VariantStream> = None;
    for v in &sorted {
        if v.resolution
            .as_ref()
            .map(|r| r.height <= 720)
            .unwrap_or(true)
        {
            best = Some(*v);
        }
    }

    best.or_else(|| sorted.first().copied())
}

fn resolve_url(base: &str, relative: &str) -> String {
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.to_string();
    }

    let (base_path, query) = match base.find('?') {
        Some(pos) => (&base[..pos], Some(&base[pos..])),
        None => (base, None),
    };

    let resolved = if let Some(pos) = base_path.rfind('/') {
        format!("{}/{}", &base_path[..pos], relative)
    } else {
        relative.to_string()
    };

    match query {
        Some(q) if !relative.contains('?') => format!("{}{}", resolved, q),
        _ => resolved,
    }
}

async fn download_segment_with_retry(
    client: &Client,
    url: &str,
    referer: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut last_err = None;
    for attempt in 0..3u32 {
        match client
            .get(url)
            .header("Referer", referer)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
        {
            Ok(resp) => match resp.bytes().await {
                Ok(bytes) => return Ok(bytes.to_vec()),
                Err(e) => last_err = Some(anyhow::anyhow!(e)),
            },
            Err(e) => last_err = Some(anyhow::anyhow!(e)),
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1)))
                .await;
        }
    }
    Err(last_err
        .unwrap_or_else(|| anyhow::anyhow!("Download do segmento falhou após 3 tentativas")))
}

fn compute_iv(encryption: &EncryptionInfo, segment_index: usize, media_sequence: u64) -> [u8; 16] {
    if let Some(iv) = &encryption.iv {
        return *iv;
    }
    let seq = media_sequence + segment_index as u64;
    let mut iv = [0u8; 16];
    iv[8..16].copy_from_slice(&seq.to_be_bytes());
    iv
}

fn parse_hex_iv(iv_str: &str) -> [u8; 16] {
    let hex = iv_str
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    let mut result = [0u8; 16];
    let padded = format!("{:0>32}", hex);
    for i in 0..16 {
        result[i] = u8::from_str_radix(&padded[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    result
}
