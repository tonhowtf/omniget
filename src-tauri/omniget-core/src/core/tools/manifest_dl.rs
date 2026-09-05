//! Baixar um manifesto HLS (.m3u8) ou DASH (.mpd) direto pelo FFmpeg
//! (estudos 34 e 54): `-c copy`, cabeçalhos e cookies do usuário, sem DRM.

use std::path::PathBuf;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestOptions {
    pub url: String,
    pub dest_dir: String,
    pub file_name: String,
    #[serde(default)]
    pub referer: String,
    #[serde(default)]
    pub user_agent: String,
    #[serde(default)]
    pub cookie: String,
    #[serde(default)]
    pub extra_headers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestResult {
    pub path: String,
    pub seconds: f64,
}

pub fn headers_arg(opts: &ManifestOptions) -> Option<String> {
    let mut h = Vec::new();
    if !opts.referer.trim().is_empty() {
        h.push(format!("Referer: {}", opts.referer.trim()));
        h.push(format!(
            "Origin: {}",
            opts.referer.trim().trim_end_matches('/')
        ));
    }
    if !opts.cookie.trim().is_empty() {
        h.push(format!("Cookie: {}", opts.cookie.trim()));
    }
    for e in &opts.extra_headers {
        if e.contains(':') {
            h.push(e.trim().to_string());
        }
    }
    if h.is_empty() {
        None
    } else {
        Some(h.join("\r\n") + "\r\n")
    }
}

pub async fn download(
    opts: ManifestOptions,
    progress: super::ProgressFn,
) -> anyhow::Result<ManifestResult> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let url = opts.url.trim().to_string();
    if !(url.contains(".m3u8") || url.contains(".mpd") || url.starts_with("http")) {
        return Err(anyhow!("cole a URL de um .m3u8 ou .mpd"));
    }
    std::fs::create_dir_all(&opts.dest_dir)?;
    let mut name = super::sanitize_name(opts.file_name.trim());
    if !name.to_lowercase().ends_with(".mp4") && !name.to_lowercase().ends_with(".mkv") {
        name.push_str(".mp4");
    }
    let out = PathBuf::from(&opts.dest_dir).join(name);
    let mut cmd = crate::core::process::command(&ffmpeg);
    cmd.args([
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostats",
        "-progress",
        "pipe:1",
    ]);
    if !opts.user_agent.trim().is_empty() {
        cmd.args(["-user_agent", opts.user_agent.trim()]);
    }
    if let Some(h) = headers_arg(&opts) {
        cmd.args(["-headers", &h]);
    }
    cmd.args([
        "-protocol_whitelist",
        "file,http,https,tcp,tls,crypto,data",
        "-allowed_extensions",
        "ALL",
        "-i",
        &url,
    ]);
    cmd.args([
        "-c",
        "copy",
        "-bsf:a",
        "aac_adtstoasc",
        "-movflags",
        "+faststart",
    ])
    .arg(&out);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let id = format!("manifest:{}", url);
    let p2 = progress.clone();
    let id2 = id.clone();
    let t1 = tokio::spawn(async move {
        let mut secs = 0.0f64;
        if let Some(o) = stdout {
            let mut lines = BufReader::new(o).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(us) = line
                    .strip_prefix("out_time_us=")
                    .and_then(|v| v.trim().parse::<u64>().ok())
                {
                    secs = us as f64 / 1_000_000.0;
                    super::report(
                        &p2,
                        &id2,
                        "progress",
                        us / 1_000_000,
                        None,
                        Some(format!("{:.0}s", secs)),
                    );
                }
            }
        }
        secs
    });
    let t2 = tokio::spawn(async move {
        let mut tail = String::new();
        if let Some(e) = stderr {
            let mut lines = BufReader::new(e).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    tail = line;
                }
            }
        }
        tail
    });
    let st = child.wait().await?;
    let secs = t1.await.unwrap_or(0.0);
    let tail = t2.await.unwrap_or_default();
    if !st.success() {
        let hint = if tail.contains("403") || tail.contains("401") {
            " (o servidor recusou: falta Referer/cookie ou o link expirou)"
        } else if tail.to_lowercase().contains("drm") || tail.contains("cenc") {
            " (conteudo com DRM nao e suportado)"
        } else {
            ""
        };
        return Err(anyhow!("ffmpeg falhou: {}{}", tail, hint));
    }
    super::report(&progress, &id, "done", secs as u64, Some(secs as u64), None);
    Ok(ManifestResult {
        path: out.to_string_lossy().to_string(),
        seconds: secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_headers() {
        let o = ManifestOptions {
            url: "".into(),
            dest_dir: "".into(),
            file_name: "".into(),
            referer: "https://x.com/".into(),
            user_agent: "".into(),
            cookie: "a=1".into(),
            extra_headers: vec!["X-Y: z".into()],
        };
        assert_eq!(
            headers_arg(&o).unwrap(),
            "Referer: https://x.com/\r\nOrigin: https://x.com\r\nCookie: a=1\r\nX-Y: z\r\n"
        );
    }
}
