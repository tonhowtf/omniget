//! gallery-dl (estudo 52) como processo: lista antes (`-j`) e baixa depois,
//! contando arquivos pela saída.

use anyhow::anyhow;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct GalleryStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

pub async fn status() -> GalleryStatus {
    let path = crate::core::dependencies::find_tool("gallery-dl").await;
    let version = match &path {
        Some(p) => crate::core::dependencies::check_version_at_path(p, "gallery-dl").await,
        None => None,
    };
    GalleryStatus { installed: path.is_some(), path: path.map(|p| p.to_string_lossy().to_string()), version }
}

pub async fn install() -> anyhow::Result<String> {
    crate::core::dependencies::ensure_gallerydl()
        .await
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| anyhow!("nao foi possivel baixar o gallery-dl para este sistema"))
}

#[derive(Debug, Clone, Serialize)]
pub struct GalleryResult {
    pub files: Vec<String>,
    pub dest: String,
    pub log_tail: String,
}

pub async fn download(url: &str, dest: &str, cookies_file: Option<&str>, progress: super::ProgressFn) -> anyhow::Result<GalleryResult> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let bin = crate::core::dependencies::ensure_gallerydl()
        .await
        .ok_or_else(|| anyhow!("gallery-dl nao esta instalado"))?;
    std::fs::create_dir_all(dest)?;
    let mut cmd = crate::core::process::command(&bin);
    cmd.args(["-d", dest, "--write-metadata"]);
    if let Some(c) = cookies_file.filter(|c| !c.trim().is_empty()) {
        cmd.args(["--cookies", c]);
    }
    cmd.arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| anyhow!("nao foi possivel iniciar o gallery-dl: {}", e))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let id = format!("gallery:{}", url);
    let p2 = progress.clone();
    let id2 = id.clone();
    let out_task = tokio::spawn(async move {
        let mut files = Vec::new();
        if let Some(o) = stdout {
            let mut lines = BufReader::new(o).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let l = line.trim().trim_start_matches("# ").to_string();
                if !l.is_empty() {
                    files.push(l.clone());
                    super::report(&p2, &id2, "download", files.len() as u64, None, Some(l));
                }
            }
        }
        files
    });
    let err_task = tokio::spawn(async move {
        let mut tail = String::new();
        if let Some(e) = stderr {
            let mut lines = BufReader::new(e).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tail = line;
            }
        }
        tail
    });
    let status = child.wait().await?;
    let files = out_task.await.unwrap_or_default();
    let tail = err_task.await.unwrap_or_default();
    if !status.success() && files.is_empty() {
        return Err(anyhow!("gallery-dl falhou: {}", tail));
    }
    super::report(&progress, &id, "done", files.len() as u64, Some(files.len() as u64), None);
    Ok(GalleryResult { files, dest: dest.to_string(), log_tail: tail })
}
