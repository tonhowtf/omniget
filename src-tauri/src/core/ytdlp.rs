use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::anyhow;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::models::media::DownloadResult;

pub async fn find_ytdlp() -> Option<PathBuf> {
    let bin_name = if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };

    if let Ok(output) = tokio::process::Command::new(bin_name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        if output.success() {
            return Some(PathBuf::from(bin_name));
        }
    }

    let managed = managed_ytdlp_path()?;
    if managed.exists() {
        return Some(managed);
    }

    None
}

fn managed_ytdlp_path() -> Option<PathBuf> {
    let data = dirs::data_dir()?;
    let bin_name = if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    Some(data.join("omniget").join("bin").join(bin_name))
}

pub async fn ensure_ytdlp() -> anyhow::Result<PathBuf> {
    if let Some(path) = find_ytdlp().await {
        return Ok(path);
    }

    download_ytdlp_binary().await
}

async fn download_ytdlp_binary() -> anyhow::Result<PathBuf> {
    let target = managed_ytdlp_path()
        .ok_or_else(|| anyhow!("Não foi possível determinar diretório de dados"))?;

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let download_url = if cfg!(target_os = "windows") {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos"
    } else {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp"
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let response = client.get(download_url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Falha ao baixar yt-dlp: HTTP {}",
            response.status()
        ));
    }

    let bytes = response.bytes().await?;
    tokio::fs::write(&target, &bytes).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        tokio::fs::set_permissions(&target, perms).await?;
    }

    Ok(target)
}

pub async fn get_video_info(ytdlp: &Path, url: &str) -> anyhow::Result<serde_json::Value> {
    let output = tokio::process::Command::new(ytdlp)
        .args(["--dump-json", "--no-warnings", "--no-playlist", url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow!("Falha ao executar yt-dlp: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("yt-dlp falhou: {}", stderr.trim()));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| anyhow!("yt-dlp retornou JSON inválido: {}", e))?;

    Ok(json)
}

pub async fn get_playlist_info(
    ytdlp: &Path,
    url: &str,
) -> anyhow::Result<(String, Vec<PlaylistEntry>)> {
    let output = tokio::process::Command::new(ytdlp)
        .args([
            "--flat-playlist",
            "--dump-json",
            "--no-warnings",
            url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow!("Falha ao executar yt-dlp: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("yt-dlp playlist falhou: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    let mut playlist_title = String::new();

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if playlist_title.is_empty() {
                playlist_title = json
                    .get("playlist_title")
                    .or_else(|| json.get("playlist"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("playlist")
                    .to_string();
            }

            let id = json
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = json
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let url = json
                .get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={}", id));
            let duration = json.get("duration").and_then(|v| v.as_f64());

            if !id.is_empty() {
                entries.push(PlaylistEntry {
                    id,
                    title,
                    url,
                    duration,
                });
            }
        }
    }

    Ok((playlist_title, entries))
}

pub struct PlaylistEntry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub duration: Option<f64>,
}

pub async fn download_video(
    ytdlp: &Path,
    url: &str,
    output_dir: &Path,
    quality_height: Option<u32>,
    progress: mpsc::Sender<f64>,
) -> anyhow::Result<DownloadResult> {
    let format_selector = match quality_height {
        Some(h) if h > 0 => format!(
            "bv*[height<={}]+ba/b[height<={}]/bv*+ba/b",
            h, h
        ),
        _ => "bv*+ba/b".to_string(),
    };

    let output_template = output_dir
        .join("%(title).200s [%(id)s].%(ext)s")
        .to_string_lossy()
        .to_string();

    tokio::fs::create_dir_all(output_dir).await?;

    let mut child = tokio::process::Command::new(ytdlp)
        .args([
            "-f",
            &format_selector,
            "--merge-output-format",
            "mp4",
            "--no-playlist",
            "--newline",
            "--progress-template",
            "download:%(progress._percent_str)s",
            "-o",
            &output_template,
            url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("Falha ao iniciar yt-dlp: {}", e))?;

    let stdout = child.stdout.take().ok_or_else(|| anyhow!("Sem stdout"))?;
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    let progress_tx = progress.clone();
    let line_reader = tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(pct) = parse_progress_line(&line) {
                let _ = progress_tx.send(pct).await;
            }
        }
    });

    let status = child
        .wait()
        .await
        .map_err(|e| anyhow!("yt-dlp processo falhou: {}", e))?;

    let _ = line_reader.await;

    if !status.success() {
        return Err(anyhow!("yt-dlp saiu com código {}", status));
    }

    let _ = progress.send(100.0).await;

    let file_path = find_downloaded_file(output_dir, url).await?;
    let meta = tokio::fs::metadata(&file_path).await?;

    Ok(DownloadResult {
        file_path,
        file_size_bytes: meta.len(),
        duration_seconds: 0.0,
    })
}

fn parse_progress_line(line: &str) -> Option<f64> {
    let line = line.trim();
    let pct_str = if let Some(rest) = line.strip_prefix("download:") {
        rest.trim().trim_end_matches('%')
    } else if line.ends_with('%') {
        line.trim_end_matches('%').split_whitespace().last()?
    } else {
        return None;
    };

    pct_str.trim().parse::<f64>().ok()
}

async fn find_downloaded_file(output_dir: &Path, url: &str) -> anyhow::Result<PathBuf> {
    let video_id = extract_id_from_url(url).unwrap_or_default();

    let mut entries = tokio::fs::read_dir(output_dir).await?;
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".part") || name.ends_with(".ytdl") || name.starts_with('.') {
            continue;
        }

        if !video_id.is_empty() && name.contains(&video_id) {
            if let Ok(meta) = entry.metadata().await {
                if let Ok(modified) = meta.modified() {
                    match &best {
                        Some((_, best_time)) if modified > *best_time => {
                            best = Some((path, modified));
                        }
                        None => {
                            best = Some((path, modified));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if best.is_none() {
        let mut entries = tokio::fs::read_dir(output_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".part") || name.ends_with(".ytdl") || name.starts_with('.') {
                continue;
            }
            if let Ok(meta) = entry.metadata().await {
                if let Ok(modified) = meta.modified() {
                    match &best {
                        Some((_, best_time)) if modified > *best_time => {
                            best = Some((path, modified));
                        }
                        None => {
                            best = Some((path, modified));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    best.map(|(p, _)| p)
        .ok_or_else(|| anyhow!("Arquivo baixado não encontrado em {:?}", output_dir))
}

fn extract_id_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_lowercase();

    if host.contains("youtu.be") {
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        return segments.first().map(|s| s.to_string());
    }

    if host.contains("youtube.com") {
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        if segments.first() == Some(&"shorts") {
            return segments.get(1).map(|s| s.to_string());
        }
        return parsed
            .query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.to_string());
    }

    None
}
