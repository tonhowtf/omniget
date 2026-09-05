//! Download acelerado com aria2c (estudo 53): 16 conexões, resume, checksum.

use std::path::PathBuf;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct Aria2Options {
    pub url: String,
    pub dest_dir: String,
    #[serde(default)]
    pub file_name: String,
    #[serde(default = "sixteen")]
    pub connections: u32,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub headers: Vec<String>,
}

fn sixteen() -> u32 {
    16
}

#[derive(Debug, Clone, Serialize)]
pub struct Aria2Result {
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Aria2Status {
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

pub async fn status() -> Aria2Status {
    let p = crate::core::dependencies::ensure_aria2c().await;
    let version = match &p {
        Some(p) => crate::core::dependencies::check_version_at_path(p, "aria2c").await,
        None => None,
    };
    Aria2Status { installed: p.is_some(), path: p.map(|p| p.to_string_lossy().to_string()), version }
}

/// `[#a1b2c3 12MiB/100MiB(12%) CN:16 DL:5.0MiB ETA:10s]`
pub fn parse_progress(line: &str) -> Option<(u64, String)> {
    let re = regex::Regex::new(r"\((\d+)%\).*?DL:([^\s\]]+)").ok()?;
    let c = re.captures(line)?;
    Some((c[1].parse().ok()?, c[2].to_string()))
}

pub async fn download(opts: Aria2Options, progress: super::ProgressFn) -> anyhow::Result<Aria2Result> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let bin = crate::core::dependencies::ensure_aria2c().await.ok_or_else(|| anyhow!("aria2c nao esta instalado (Linux/macOS: instale pelo gerenciador de pacotes)"))?;
    std::fs::create_dir_all(&opts.dest_dir)?;
    let conns = opts.connections.clamp(1, 16).to_string();
    let mut cmd = crate::core::process::command(&bin);
    cmd.args([
        "--dir", &opts.dest_dir, "-x", &conns, "-s", &conns, "-k", "1M", "--continue=true", "--auto-file-renaming=false",
        "--allow-overwrite=true", "--summary-interval=1", "--console-log-level=warn", "--download-result=hide", "--file-allocation=none",
    ]);
    if !opts.file_name.trim().is_empty() {
        cmd.args(["--out", opts.file_name.trim()]);
    }
    if !opts.sha256.trim().is_empty() {
        cmd.arg(format!("--checksum=sha-256={}", opts.sha256.trim()));
    }
    for h in &opts.headers {
        if !h.trim().is_empty() {
            cmd.arg(format!("--header={}", h.trim()));
        }
    }
    cmd.arg(&opts.url).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| anyhow!("nao foi possivel iniciar o aria2c: {}", e))?;
    let stdout = child.stdout.take();
    let id = format!("aria2:{}", opts.url);
    let p2 = progress.clone();
    let id2 = id.clone();
    let task = tokio::spawn(async move {
        let mut last = String::new();
        if let Some(o) = stdout {
            let mut lines = BufReader::new(o).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some((pct, speed)) = parse_progress(&line) {
                    super::report(&p2, &id2, "progress", pct, Some(100), Some(speed));
                } else if !line.trim().is_empty() {
                    last = line;
                }
            }
        }
        last
    });
    let st = child.wait().await?;
    let last = task.await.unwrap_or_default();
    if !st.success() {
        return Err(anyhow!("aria2c falhou: {}", last));
    }
    // aria2 decide o nome pelo Content-Disposition/URL; pega o arquivo mais novo da pasta
    let path = if !opts.file_name.trim().is_empty() {
        PathBuf::from(&opts.dest_dir).join(opts.file_name.trim())
    } else {
        std::fs::read_dir(&opts.dest_dir)?
            .flatten()
            .filter(|e| e.path().is_file() && !e.path().to_string_lossy().ends_with(".aria2"))
            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
            .map(|e| e.path())
            .ok_or_else(|| anyhow!("download terminou mas nao achei o arquivo"))?
    };
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    super::report(&progress, &id, "done", 100, Some(100), None);
    Ok(Aria2Result { path: path.to_string_lossy().to_string(), bytes })
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_line() {
        let (p, s) = super::parse_progress("[#2089b0 12MiB/100MiB(12%) CN:16 DL:5.0MiB ETA:10s]").unwrap();
        assert_eq!(p, 12);
        assert_eq!(s, "5.0MiB");
    }
}
