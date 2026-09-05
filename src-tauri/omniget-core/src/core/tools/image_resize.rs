//! Redimensionar imagens em lote com o FFmpeg já gerido (estudo 29,
//! Image Resizer do PowerToys): largura fixa, porcentagem ou "caber em".

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct ResizeOptions {
    pub inputs: Vec<String>,
    /// "width" | "height" | "fit" | "percent"
    pub mode: String,
    pub value: u32,
    #[serde(default)]
    pub value2: u32,
    /// "" mantém a extensão; "jpg" | "png" | "webp"
    #[serde(default)]
    pub format: String,
    #[serde(default = "default_quality")]
    pub quality: u32,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub suffix: String,
}

fn default_quality() -> u32 {
    90
}

#[derive(Debug, Clone, Serialize)]
pub struct ResizeResult {
    pub outputs: Vec<String>,
    pub failed: Vec<String>,
}

pub fn scale_filter(mode: &str, v: u32, v2: u32) -> String {
    match mode {
        "height" => format!("scale=-2:{}", v.max(1)),
        "fit" => format!("scale='min({w},iw)':'min({h},ih)':force_original_aspect_ratio=decrease", w = v.max(1), h = v2.max(1)),
        "percent" => format!("scale=iw*{p}/100:ih*{p}/100", p = v.max(1)),
        _ => format!("scale={}:-2", v.max(1)),
    }
}

pub async fn run(opts: ResizeOptions, progress: super::ProgressFn) -> anyhow::Result<ResizeResult> {
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let filter = scale_filter(&opts.mode, opts.value, opts.value2);
    let mut outputs = Vec::new();
    let mut failed = Vec::new();
    let total = opts.inputs.len() as u64;
    // ffmpeg: -q:v 2 (melhor) … 31 (pior) para JPEG; mapeia 100..0 → 2..31
    let qv = (2.0 + (100.0 - opts.quality.min(100) as f64) * 0.29).round() as u32;
    for (i, input) in opts.inputs.iter().enumerate() {
        super::report(&progress, "resize", "progress", i as u64, Some(total), Some(input.clone()));
        let inp = Path::new(input);
        let ext = if opts.format.trim().is_empty() {
            inp.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "jpg".into())
        } else {
            opts.format.trim().to_lowercase()
        };
        let out_dir = if opts.output_dir.trim().is_empty() {
            inp.parent().map(|p| p.to_path_buf()).unwrap_or_default()
        } else {
            PathBuf::from(opts.output_dir.trim())
        };
        std::fs::create_dir_all(&out_dir)?;
        let stem = inp.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "imagem".into());
        let suffix = if opts.suffix.is_empty() { "-resized" } else { opts.suffix.as_str() };
        let out = out_dir.join(format!("{}{}.{}", stem, suffix, ext));
        let mut cmd = crate::core::process::command(&ffmpeg);
        cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-i"]).arg(inp).args(["-vf", &filter]);
        if ext == "jpg" || ext == "jpeg" {
            cmd.args(["-q:v", &qv.to_string()]);
        } else if ext == "webp" {
            cmd.args(["-quality", &opts.quality.to_string()]);
        }
        cmd.arg(&out);
        match cmd.output().await {
            Ok(o) if o.status.success() => outputs.push(out.to_string_lossy().to_string()),
            Ok(o) => {
                tracing::warn!("[resize] {}: {}", input, String::from_utf8_lossy(&o.stderr).trim());
                failed.push(input.clone());
            }
            Err(e) => return Err(anyhow!("ffmpeg nao iniciou: {}", e)),
        }
    }
    super::report(&progress, "resize", "done", total, Some(total), None);
    Ok(ResizeResult { outputs, failed })
}

#[cfg(test)]
mod tests {
    use super::scale_filter;

    #[test]
    fn filters() {
        assert_eq!(scale_filter("width", 800, 0), "scale=800:-2");
        assert_eq!(scale_filter("percent", 50, 0), "scale=iw*50/100:ih*50/100");
        assert!(scale_filter("fit", 1920, 1080).contains("force_original_aspect_ratio"));
    }
}
