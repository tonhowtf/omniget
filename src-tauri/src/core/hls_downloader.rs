use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use m3u8_rs::{parse_master_playlist, parse_media_playlist, MasterPlaylist, VariantStream};
use reqwest::Client;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::{self, UnboundedSender};
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
        Self::with_client(
            Client::builder()
                .danger_accept_invalid_certs(true)
                .connect_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(300))
                .pool_max_idle_per_host(50)
                .pool_idle_timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
        )
    }

    pub fn with_client(client: Client) -> Self {
        Self { client }
    }

    pub async fn download(
        &self,
        m3u8_url: &str,
        output_path: &str,
        referer: &str,
        bytes_tx: Option<UnboundedSender<u64>>,
        cancel_token: CancellationToken,
        max_concurrent: u32,
        max_retries: u32,
    ) -> anyhow::Result<HlsDownloadResult> {
        if cancel_token.is_cancelled() {
            anyhow::bail!("Download cancelado pelo usuário");
        }

        let m3u8_text = self.fetch_m3u8_with_retry(m3u8_url, referer, 3).await?;

        let m3u8_bytes = m3u8_text.as_bytes();

        if let Ok((_, master)) = parse_master_playlist(m3u8_bytes) {
            if let Some(variant) = select_best_variant(&master) {
                let variant_url = resolve_url(m3u8_url, &variant.uri);
                return self
                    .download_media_playlist(&variant_url, output_path, referer, bytes_tx, cancel_token, max_concurrent, max_retries)
                    .await;
            }
        }

        if parse_media_playlist(m3u8_bytes).is_ok() {
            return self
                .download_media_playlist(m3u8_url, output_path, referer, bytes_tx, cancel_token, max_concurrent, max_retries)
                .await;
        }

        anyhow::bail!("Falha ao parsear m3u8: nem master nem media playlist")
    }

    async fn fetch_m3u8_with_retry(
        &self,
        url: &str,
        referer: &str,
        max_retries: u32,
    ) -> anyhow::Result<String> {
        let mut last_err = None;
        for attempt in 0..max_retries {
            match self
                .client
                .get(url)
                .header("Referer", referer)
                .header("User-Agent", USER_AGENT)
                .send()
                .await
            {
                Ok(resp) => match resp.text().await {
                    Ok(text) => return Ok(text),
                    Err(e) => last_err = Some(anyhow::anyhow!(e)),
                },
                Err(e) => last_err = Some(anyhow::anyhow!(e)),
            }
            if attempt < max_retries - 1 {
                let base = 500 * (attempt as u64 + 1);
                let jitter = rand::random::<u64>() % (base / 2 + 1);
                tokio::time::sleep(Duration::from_millis(base + jitter)).await;
            }
        }
        Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!("Falha ao buscar m3u8 após {} tentativas", max_retries)
        }))
    }

    async fn download_media_playlist(
        &self,
        m3u8_url: &str,
        output_path: &str,
        referer: &str,
        bytes_tx: Option<UnboundedSender<u64>>,
        cancel_token: CancellationToken,
        max_concurrent: u32,
        max_retries: u32,
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

        let encryption = self
            .fetch_encryption_info(&playlist, m3u8_url, referer)
            .await?;

        let output = PathBuf::from(output_path);
        if let Some(parent) = output.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Channel for segments: bounded to max_concurrent to limit memory
        let (seg_tx, seg_rx) = mpsc::channel::<(usize, Vec<u8>)>(max_concurrent as usize * 2);

        // Spawn ordered writer task
        let writer_output = output.clone();
        let media_sequence = playlist.media_sequence;
        let writer = tokio::spawn(async move {
            write_segments_ordered(seg_rx, &writer_output, &encryption, media_sequence, total_segments).await
        });

        let semaphore = Arc::new(Semaphore::new(max_concurrent as usize));
        let completed = Arc::new(AtomicUsize::new(0));
        let download_error = Arc::new(tokio::sync::Mutex::new(None::<String>));

        for (i, segment) in playlist.segments.iter().enumerate() {
            let sem = semaphore.clone();
            let client = self.client.clone();
            let url = resolve_url(m3u8_url, &segment.uri);
            let referer = referer.to_string();
            let completed = completed.clone();
            let bytes_tx = bytes_tx.clone();
            let ct = cancel_token.clone();
            let retries = max_retries;
            let tx = seg_tx.clone();
            let err_holder = download_error.clone();

            tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                if ct.is_cancelled() {
                    return;
                }
                match download_segment_with_retry(&client, &url, &referer, retries).await {
                    Ok(data) => {
                        if let Some(ref btx) = bytes_tx {
                            let _ = btx.send(data.len() as u64);
                        }
                        completed.fetch_add(1, Ordering::Relaxed);
                        let _ = tx.send((i, data)).await;
                    }
                    Err(e) => {
                        let mut guard = err_holder.lock().await;
                        if guard.is_none() {
                            *guard = Some(e.to_string());
                        }
                    }
                }
            });
        }
        // Drop our sender so writer finishes when all tasks complete
        drop(seg_tx);

        // Wait for writer to finish processing all segments
        let writer_result = writer.await
            .map_err(|e| anyhow::anyhow!("Writer task panicked: {:?}", e))?;

        if cancel_token.is_cancelled() {
            let _ = tokio::fs::remove_file(&output).await;
            anyhow::bail!("Download cancelado pelo usuário");
        }

        // Check for download errors
        if let Some(err_msg) = download_error.lock().await.take() {
            let _ = tokio::fs::remove_file(&output).await;
            anyhow::bail!("Falha no download de segmento: {}", err_msg);
        }

        writer_result?;

        let file_size = tokio::fs::metadata(&output).await?.len();

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

async fn write_segments_ordered(
    mut rx: mpsc::Receiver<(usize, Vec<u8>)>,
    output_path: &PathBuf,
    encryption: &Option<EncryptionInfo>,
    media_sequence: u64,
    total_segments: usize,
) -> anyhow::Result<()> {
    let mut file = tokio::fs::File::create(output_path).await?;
    let mut next_expected: usize = 0;
    let mut pending: BTreeMap<usize, Vec<u8>> = BTreeMap::new();

    while let Some((idx, data)) = rx.recv().await {
        pending.insert(idx, data);

        while let Some(segment_data) = pending.remove(&next_expected) {
            if let Some(enc) = encryption {
                use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
                type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

                let iv = compute_iv(enc, next_expected, media_sequence);
                let mut buf = segment_data;
                let decryptor = Aes128CbcDec::new_from_slices(&enc.key_bytes, &iv)
                    .map_err(|e| anyhow::anyhow!("AES init: {:?}", e))?;
                let decrypted = decryptor
                    .decrypt_padded_mut::<Pkcs7>(&mut buf)
                    .map_err(|e| anyhow::anyhow!("AES decrypt: {:?}", e))?;
                file.write_all(decrypted).await?;
            } else {
                file.write_all(&segment_data).await?;
            }
            next_expected += 1;
        }
    }

    file.flush().await?;

    if next_expected < total_segments {
        anyhow::bail!(
            "Apenas {} de {} segmentos foram escritos",
            next_expected,
            total_segments
        );
    }

    Ok(())
}

const SEGMENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

async fn download_segment_with_retry(
    client: &Client,
    url: &str,
    referer: &str,
    max_retries: u32,
) -> anyhow::Result<Vec<u8>> {
    let mut last_err = None;
    for attempt in 0..max_retries {
        let result = tokio::time::timeout(SEGMENT_TIMEOUT, async {
            let resp = client
                .get(url)
                .header("Referer", referer)
                .header("User-Agent", USER_AGENT)
                .send()
                .await?;
            resp.bytes().await.map(|b| b.to_vec())
        })
        .await;

        match result {
            Ok(Ok(data)) => return Ok(data),
            Ok(Err(e)) => last_err = Some(anyhow::anyhow!(e)),
            Err(_) => last_err = Some(anyhow::anyhow!("Timeout ao baixar segmento")),
        }
        if attempt < max_retries - 1 {
            let base = 500 * (attempt as u64 + 1);
            let jitter = rand::random::<u64>() % (base / 2 + 1);
            tokio::time::sleep(std::time::Duration::from_millis(base + jitter))
                .await;
        }
    }
    Err(last_err
        .unwrap_or_else(|| anyhow::anyhow!("Download do segmento falhou após {} tentativas", max_retries)))
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
