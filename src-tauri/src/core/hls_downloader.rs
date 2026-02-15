use m3u8_rs::{parse_master_playlist, parse_media_playlist, MasterPlaylist, VariantStream};
use reqwest::Client;
use std::path::PathBuf;
use tokio::sync::mpsc;

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

    pub async fn download_hls(
        &self,
        m3u8_url: &str,
        output_path: &str,
        headers: &[(&str, &str)],
        quality: &str,
        progress_tx: Option<mpsc::Sender<f64>>,
    ) -> anyhow::Result<PathBuf> {
        let mut req = self.client.get(m3u8_url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let m3u8_text = req.send().await?.text().await?;
        let m3u8_bytes = m3u8_text.as_bytes();

        if let Ok((_, master)) = parse_master_playlist(m3u8_bytes) {
            let variant = self.select_variant(&master, quality);
            let variant_url = resolve_url(m3u8_url, &variant.uri);
            tracing::info!(
                "Variante selecionada: {}x{} @ {} bps",
                variant
                    .resolution
                    .as_ref()
                    .map(|r| r.width)
                    .unwrap_or(0),
                variant
                    .resolution
                    .as_ref()
                    .map(|r| r.height)
                    .unwrap_or(0),
                variant.bandwidth
            );
            return self
                .download_media_playlist(&variant_url, output_path, headers, progress_tx)
                .await;
        }

        if parse_media_playlist(m3u8_bytes).is_ok() {
            return self
                .download_media_playlist(m3u8_url, output_path, headers, progress_tx)
                .await;
        }

        anyhow::bail!("Falha ao parsear m3u8: nem master nem media playlist")
    }

    async fn download_media_playlist(
        &self,
        m3u8_url: &str,
        output_path: &str,
        headers: &[(&str, &str)],
        progress_tx: Option<mpsc::Sender<f64>>,
    ) -> anyhow::Result<PathBuf> {
        let mut req = self.client.get(m3u8_url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let text = req.send().await?.text().await?;
        let (_, playlist) = parse_media_playlist(text.as_bytes())
            .map_err(|e| anyhow::anyhow!("Parse media playlist: {:?}", e))?;

        let temp_dir = tempfile::tempdir()?;
        let total_segments = playlist.segments.len();
        tracing::info!(
            "Baixando {} segmentos de {}",
            total_segments,
            m3u8_url
        );
        let mut segment_paths: Vec<PathBuf> = Vec::new();

        for (i, segment) in playlist.segments.iter().enumerate() {
            let segment_url = resolve_url(m3u8_url, &segment.uri);
            let seg_path = temp_dir.path().join(format!("seg_{:05}.ts", i));

            let mut req = self.client.get(&segment_url);
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            let bytes = req.send().await?.bytes().await?;
            tokio::fs::write(&seg_path, &bytes).await?;
            segment_paths.push(seg_path);

            if let Some(tx) = &progress_tx {
                let pct = (i + 1) as f64 / total_segments as f64 * 100.0;
                let _ = tx.send(pct).await;
            }
        }

        let concat_file = temp_dir.path().join("concat.txt");
        let concat_content: String = segment_paths
            .iter()
            .map(|p| format!("file '{}'", p.display()))
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(&concat_file, &concat_content).await?;

        let output = PathBuf::from(output_path);
        if let Some(parent) = output.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let status = tokio::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                concat_file.to_str().unwrap(),
                "-c",
                "copy",
                output.to_str().unwrap(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("FFmpeg concat falhou com status {}", status);
        }

        tracing::info!("HLS download completo: {}", output.display());
        Ok(output)
    }

    fn select_variant<'a>(
        &self,
        master: &'a MasterPlaylist,
        quality: &str,
    ) -> &'a VariantStream {
        let mut variants: Vec<_> = master.variants.iter().collect();
        variants.sort_by_key(|v| v.bandwidth);

        match quality {
            "max" => variants.last().unwrap(),
            "min" => variants.first().unwrap(),
            q => {
                let target: u64 = q.parse().unwrap_or(720);
                variants
                    .iter()
                    .min_by_key(|v| {
                        v.resolution
                            .as_ref()
                            .map(|r| r.height.abs_diff(target))
                            .unwrap_or(u64::MAX)
                    })
                    .unwrap()
            }
        }
    }
}

fn resolve_url(base: &str, relative: &str) -> String {
    if relative.starts_with("http") {
        return relative.to_string();
    }
    if let Some(base_prefix) = base.rsplit_once('/') {
        format!("{}/{}", base_prefix.0, relative)
    } else {
        relative.to_string()
    }
}
