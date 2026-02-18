use crate::core::hls_downloader::HlsDownloadResult;
use tokio_util::sync::CancellationToken;

pub struct MediaProcessor;

impl MediaProcessor {
    pub async fn download_hls(
        m3u8_url: &str,
        output: &str,
        referer: &str,
        bytes_tx: Option<tokio::sync::mpsc::UnboundedSender<u64>>,
        cancel_token: CancellationToken,
        max_concurrent: u32,
        max_retries: u32,
        client: Option<reqwest::Client>,
    ) -> anyhow::Result<HlsDownloadResult> {
        let downloader = match client {
            Some(c) => crate::core::hls_downloader::HlsDownloader::with_client(c),
            None => crate::core::hls_downloader::HlsDownloader::new(),
        };
        downloader.download(m3u8_url, output, referer, bytes_tx, cancel_token, max_concurrent, max_retries).await
    }

    pub async fn remux(input: &str, output: &str) -> anyhow::Result<()> {
        let status = crate::core::process::command("ffmpeg")
            .args(["-y", "-i", input, "-c", "copy", output])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("FFmpeg remux falhou com status {}", status);
        }

        Ok(())
    }

    pub async fn merge_audio_video(
        video: &str,
        audio: &str,
        output: &str,
    ) -> anyhow::Result<()> {
        let status = crate::core::process::command("ffmpeg")
            .args([
                "-y", "-i", video, "-i", audio, "-map", "0:v", "-map", "1:a", "-c", "copy",
                output,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("FFmpeg merge falhou com status {}", status);
        }

        Ok(())
    }

    pub async fn download_direct(
        url: &str,
        output: &str,
        headers: &[(&str, &str)],
    ) -> anyhow::Result<()> {
        let mut args = vec!["-y".to_string()];

        if !headers.is_empty() {
            let header_str: String = headers
                .iter()
                .map(|(k, v)| format!("{}: {}\r\n", k, v))
                .collect();
            args.extend(["-headers".to_string(), header_str]);
        }

        args.extend([
            "-i".to_string(),
            url.to_string(),
            "-c".to_string(),
            "copy".to_string(),
            output.to_string(),
        ]);

        let status = crate::core::process::command("ffmpeg")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("FFmpeg download_direct falhou com status {}", status);
        }

        Ok(())
    }
}

pub fn check_ffmpeg() -> bool {
    crate::core::process::std_command("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn check_ytdlp() -> bool {
    crate::core::process::std_command("yt-dlp")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn check_dependencies() -> Vec<String> {
    let mut missing = Vec::new();
    if !check_ytdlp() {
        missing.push("yt-dlp".into());
    }
    if !check_ffmpeg() {
        missing.push("ffmpeg".into());
    }
    missing
}
