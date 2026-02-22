use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::stream::{self, StreamExt};
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

impl Default for HlsDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl HlsDownloader {
    pub fn new() -> Self {
        Self::with_client(
            Client::builder()
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

    #[allow(clippy::too_many_arguments)]
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
        self.download_with_quality(m3u8_url, output_path, referer, bytes_tx, cancel_token, max_concurrent, max_retries, None).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn download_with_quality(
        &self,
        m3u8_url: &str,
        output_path: &str,
        referer: &str,
        bytes_tx: Option<UnboundedSender<u64>>,
        cancel_token: CancellationToken,
        max_concurrent: u32,
        max_retries: u32,
        max_height: Option<u32>,
    ) -> anyhow::Result<HlsDownloadResult> {
        if cancel_token.is_cancelled() {
            anyhow::bail!("Download cancelado pelo usuário");
        }

        let m3u8_text = self.fetch_m3u8_with_retry(m3u8_url, referer, 3).await?;

        let m3u8_bytes = m3u8_text.as_bytes();

        if let Ok((_, master)) = parse_master_playlist(m3u8_bytes) {
            if let Some(variant) = select_best_variant(&master, max_height.unwrap_or(720)) {
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

    #[allow(clippy::too_many_arguments)]
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
        let part_path = {
            let mut p = output.as_os_str().to_owned();
            p.push(".part");
            PathBuf::from(p)
        };
        if let Some(parent) = output.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let (seg_tx, seg_rx) = mpsc::channel::<(usize, Vec<u8>)>(max_concurrent as usize);

        let writer_output = part_path.clone();
        let media_sequence = playlist.media_sequence;
        let writer = tokio::spawn(async move {
            write_segments_ordered(seg_rx, &writer_output, &encryption, media_sequence, total_segments).await
        });

        let semaphore = Arc::new(Semaphore::new(max_concurrent as usize));
        let completed = Arc::new(AtomicUsize::new(0));
        let fail_token = cancel_token.child_token();
        let errors: Arc<tokio::sync::Mutex<HashMap<String, u32>>> = Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        let segment_urls: Vec<(usize, String)> = playlist
            .segments
            .iter()
            .enumerate()
            .map(|(i, seg)| (i, resolve_url(m3u8_url, &seg.uri)))
            .collect();

        let client = &self.client;
        let errors_ref = &errors;
        let completed_ref = &completed;
        let fail_ref = &fail_token;
        let sem_ref = &semaphore;

        stream::iter(segment_urls)
            .map(|(i, url)| {
                let bytes_tx = bytes_tx.clone();
                let seg_tx = seg_tx.clone();
                let referer = referer.to_string();
                async move {
                    let _permit = sem_ref.acquire().await.unwrap();
                    if fail_ref.is_cancelled() {
                        return;
                    }
                    match download_segment_with_retry(client, &url, &referer, max_retries, fail_ref).await {
                        Ok(data) => {
                            if let Some(ref btx) = bytes_tx {
                                let _ = btx.send(data.len() as u64);
                            }
                            completed_ref.fetch_add(1, Ordering::Relaxed);
                            let _ = seg_tx.send((i, data)).await;
                        }
                        Err(e) => {
                            let key = e.to_string();
                            let mut errs = errors_ref.lock().await;
                            *errs.entry(key).or_insert(0) += 1;
                            drop(errs);
                            fail_ref.cancel();
                        }
                    }
                }
            })
            .buffer_unordered(max_concurrent as usize)
            .collect::<()>()
            .await;

        drop(seg_tx);

        let writer_result = writer.await
            .map_err(|e| anyhow::anyhow!("Writer task panicked: {:?}", e))?;

        if cancel_token.is_cancelled() {
            let _ = tokio::fs::remove_file(&part_path).await;
            anyhow::bail!("Download cancelado pelo usuário");
        }

        let errs = errors.lock().await;
        if !errs.is_empty() {
            let _ = tokio::fs::remove_file(&part_path).await;
            let summary: Vec<String> = errs
                .iter()
                .map(|(msg, count)| {
                    if *count > 1 {
                        format!("{} (x{})", msg, count)
                    } else {
                        msg.clone()
                    }
                })
                .collect();
            anyhow::bail!("Falha no download de segmentos: {}", summary.join("; "));
        }
        drop(errs);

        writer_result?;

        tokio::fs::rename(&part_path, &output).await?;

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
                        let key_bytes = self.fetch_key_with_retry(&key_url, referer, 3).await?;

                        let iv = key.iv.as_ref().map(|iv_str| parse_hex_iv(iv_str));

                        return Ok(Some(EncryptionInfo { key_bytes, iv }));
                    }
                }
            }
        }
        Ok(None)
    }

    async fn fetch_key_with_retry(
        &self,
        url: &str,
        referer: &str,
        max_retries: u32,
    ) -> anyhow::Result<Vec<u8>> {
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
                Ok(resp) => {
                    if !resp.status().is_success() {
                        last_err = Some(anyhow::anyhow!("HTTP {} ao buscar chave AES", resp.status()));
                    } else {
                        match resp.bytes().await {
                            Ok(bytes) => return Ok(bytes.to_vec()),
                            Err(e) => last_err = Some(anyhow::anyhow!(e)),
                        }
                    }
                }
                Err(e) => last_err = Some(anyhow::anyhow!(e)),
            }
            if attempt < max_retries - 1 {
                let base = 500 * (attempt as u64 + 1);
                let jitter = rand::random::<u64>() % (base / 2 + 1);
                tokio::time::sleep(Duration::from_millis(base + jitter)).await;
            }
        }
        Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!("Falha ao buscar chave AES após {} tentativas", max_retries)
        }))
    }
}

struct EncryptionInfo {
    key_bytes: Vec<u8>,
    iv: Option<[u8; 16]>,
}

fn select_best_variant(master: &MasterPlaylist, max_height: u32) -> Option<&VariantStream> {
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

    let max_h = max_height as u64;
    let mut best: Option<&VariantStream> = None;
    for v in &sorted {
        if v.resolution
            .as_ref()
            .map(|r| r.height <= max_h)
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
    let mut file = tokio::io::BufWriter::with_capacity(
        256 * 1024,
        tokio::fs::File::create(output_path).await?,
    );
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
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<u8>> {
    let mut last_err = None;
    for attempt in 0..max_retries {
        if cancel.is_cancelled() {
            anyhow::bail!("Download cancelado");
        }

        let result = tokio::time::timeout(SEGMENT_TIMEOUT, async {
            let resp = client
                .get(url)
                .header("Referer", referer)
                .header("User-Agent", USER_AGENT)
                .send()
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let code = status.as_u16();
                if (400..500).contains(&code) && code != 429 && code != 408 {
                    return Err(anyhow::anyhow!("HTTP {} (fatal) ao baixar segmento", code));
                }
                return Err(anyhow::anyhow!("HTTP {} ao baixar segmento", code));
            }

            resp.bytes().await.map(|b| b.to_vec()).map_err(|e| anyhow::anyhow!(e))
        })
        .await;

        match result {
            Ok(Ok(data)) => return Ok(data),
            Ok(Err(e)) => {
                if e.to_string().contains("(fatal)") {
                    return Err(e);
                }
                last_err = Some(e);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use m3u8_rs::{MasterPlaylist, Resolution, VariantStream};

    #[test]
    fn resolve_url_absolute_passthrough() {
        assert_eq!(
            resolve_url(
                "https://cdn.example.com/path/master.m3u8",
                "https://other.com/video.ts"
            ),
            "https://other.com/video.ts"
        );
    }

    #[test]
    fn resolve_url_relative() {
        assert_eq!(
            resolve_url("https://cdn.example.com/path/master.m3u8", "segment0.ts"),
            "https://cdn.example.com/path/segment0.ts"
        );
    }

    #[test]
    fn resolve_url_propagates_query() {
        assert_eq!(
            resolve_url(
                "https://cdn.example.com/path/master.m3u8?token=abc",
                "segment0.ts"
            ),
            "https://cdn.example.com/path/segment0.ts?token=abc"
        );
    }

    #[test]
    fn resolve_url_relative_with_own_query_skips_base_query() {
        assert_eq!(
            resolve_url(
                "https://cdn.example.com/path/master.m3u8?token=abc",
                "segment0.ts?key=123"
            ),
            "https://cdn.example.com/path/segment0.ts?key=123"
        );
    }

    #[test]
    fn resolve_url_no_slash_in_base() {
        assert_eq!(resolve_url("master.m3u8", "segment0.ts"), "segment0.ts");
    }

    #[test]
    fn select_best_variant_picks_720() {
        let master = MasterPlaylist {
            variants: vec![
                VariantStream {
                    uri: "360.m3u8".into(),
                    bandwidth: 800_000,
                    resolution: Some(Resolution {
                        width: 640,
                        height: 360,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "720.m3u8".into(),
                    bandwidth: 2_500_000,
                    resolution: Some(Resolution {
                        width: 1280,
                        height: 720,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "1080.m3u8".into(),
                    bandwidth: 5_000_000,
                    resolution: Some(Resolution {
                        width: 1920,
                        height: 1080,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let best = select_best_variant(&master, 720).unwrap();
        assert_eq!(best.uri, "720.m3u8");
    }

    #[test]
    fn select_best_variant_picks_1080() {
        let master = MasterPlaylist {
            variants: vec![
                VariantStream {
                    uri: "720.m3u8".into(),
                    bandwidth: 2_500_000,
                    resolution: Some(Resolution {
                        width: 1280,
                        height: 720,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "1080.m3u8".into(),
                    bandwidth: 5_000_000,
                    resolution: Some(Resolution {
                        width: 1920,
                        height: 1080,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let best = select_best_variant(&master, 1080).unwrap();
        assert_eq!(best.uri, "1080.m3u8");
    }

    #[test]
    fn select_best_variant_empty_returns_none() {
        let master = MasterPlaylist {
            variants: vec![],
            ..Default::default()
        };
        assert!(select_best_variant(&master, 720).is_none());
    }

    #[test]
    fn select_best_variant_skips_iframe() {
        let master = MasterPlaylist {
            variants: vec![
                VariantStream {
                    uri: "iframe.m3u8".into(),
                    bandwidth: 100_000,
                    is_i_frame: true,
                    resolution: Some(Resolution {
                        width: 320,
                        height: 180,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "720.m3u8".into(),
                    bandwidth: 2_500_000,
                    resolution: Some(Resolution {
                        width: 1280,
                        height: 720,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let best = select_best_variant(&master, 720).unwrap();
        assert_eq!(best.uri, "720.m3u8");
    }

    #[test]
    fn select_best_variant_fallback_to_lowest_when_all_exceed() {
        let master = MasterPlaylist {
            variants: vec![
                VariantStream {
                    uri: "1080.m3u8".into(),
                    bandwidth: 5_000_000,
                    resolution: Some(Resolution {
                        width: 1920,
                        height: 1080,
                    }),
                    ..Default::default()
                },
                VariantStream {
                    uri: "4k.m3u8".into(),
                    bandwidth: 15_000_000,
                    resolution: Some(Resolution {
                        width: 3840,
                        height: 2160,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let best = select_best_variant(&master, 360).unwrap();
        assert_eq!(best.uri, "1080.m3u8");
    }

    #[test]
    fn select_best_variant_no_resolution_treated_as_eligible() {
        let master = MasterPlaylist {
            variants: vec![VariantStream {
                uri: "audio.m3u8".into(),
                bandwidth: 128_000,
                resolution: None,
                ..Default::default()
            }],
            ..Default::default()
        };
        let best = select_best_variant(&master, 720).unwrap();
        assert_eq!(best.uri, "audio.m3u8");
    }

    #[test]
    fn parse_hex_iv_full_32_chars() {
        let iv = parse_hex_iv("0x00000000000000000000000000000001");
        let mut expected = [0u8; 16];
        expected[15] = 1;
        assert_eq!(iv, expected);
    }

    #[test]
    fn parse_hex_iv_short_padded() {
        let iv = parse_hex_iv("0xFF");
        let mut expected = [0u8; 16];
        expected[15] = 0xFF;
        assert_eq!(iv, expected);
    }

    #[test]
    fn parse_hex_iv_uppercase_prefix() {
        let iv = parse_hex_iv("0X0A0B0C0D0E0F10111213141516171819");
        assert_eq!(iv[0], 0x0A);
        assert_eq!(iv[7], 0x11);
        assert_eq!(iv[15], 0x19);
    }

    #[test]
    fn parse_hex_iv_no_prefix() {
        let iv = parse_hex_iv("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");
        assert_eq!(iv, [0xFF; 16]);
    }

    #[test]
    fn compute_iv_returns_explicit_when_present() {
        let explicit_iv = [0xAB; 16];
        let enc = EncryptionInfo {
            key_bytes: vec![0u8; 16],
            iv: Some(explicit_iv),
        };
        assert_eq!(compute_iv(&enc, 5, 100), explicit_iv);
    }

    #[test]
    fn compute_iv_derives_from_sequence() {
        let enc = EncryptionInfo {
            key_bytes: vec![0u8; 16],
            iv: None,
        };
        let result = compute_iv(&enc, 3, 100);
        let mut expected = [0u8; 16];
        expected[8..16].copy_from_slice(&103u64.to_be_bytes());
        assert_eq!(result, expected);
    }

    #[test]
    fn compute_iv_sequence_zero() {
        let enc = EncryptionInfo {
            key_bytes: vec![0u8; 16],
            iv: None,
        };
        let result = compute_iv(&enc, 0, 0);
        assert_eq!(result, [0u8; 16]);
    }
}
