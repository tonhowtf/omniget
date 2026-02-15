const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub struct MediaProcessor;

impl MediaProcessor {
    pub async fn download_hls_ffmpeg(
        m3u8_url: &str,
        output: &str,
        referer: &str,
    ) -> anyhow::Result<()> {
        if let Some(parent) = std::path::Path::new(output).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let headers = format!("Referer: {}\r\nUser-Agent: {}\r\n", referer, USER_AGENT);

        let status = tokio::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-headers",
                &headers,
                "-i",
                m3u8_url,
                "-c",
                "copy",
                "-bsf:a",
                "aac_adtstoasc",
                output,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("FFmpeg HLS download falhou com status {}", status);
        }

        tracing::info!("HLS download completo: {}", output);
        Ok(())
    }

    pub async fn remux(input: &str, output: &str) -> anyhow::Result<()> {
        let status = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-i", input, "-c", "copy", output])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("FFmpeg remux falhou com status {}", status);
        }

        tracing::info!("Remux completo: {} -> {}", input, output);
        Ok(())
    }

    pub async fn merge_audio_video(
        video: &str,
        audio: &str,
        output: &str,
    ) -> anyhow::Result<()> {
        let status = tokio::process::Command::new("ffmpeg")
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

        tracing::info!("Merge completo: {} + {} -> {}", video, audio, output);
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

        let status = tokio::process::Command::new("ffmpeg")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("FFmpeg download_direct falhou com status {}", status);
        }

        tracing::info!("Download direto completo: {}", output);
        Ok(())
    }
}

pub fn check_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
