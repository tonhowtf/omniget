//! Real-ESRGAN via `realesrgan-ncnn-vulkan` (estudo 37): binário oficial do
//! xinntao, que já vem com os modelos `realesrgan-x4plus` e
//! `realesrgan-x4plus-anime`. Roda em qualquer GPU com Vulkan.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::github;
use crate::core::dependencies::bin_name;

const REPO: &str = "xinntao/Real-ESRGAN-ncnn-vulkan";
const BIN: &str = "realesrgan-ncnn-vulkan";

fn managed_dir() -> Option<PathBuf> {
    crate::core::dependencies::managed_bin_dir().map(|d| d.join("realesrgan"))
}

#[derive(Debug, Clone, Serialize)]
pub struct UpscaleStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub models: Vec<String>,
}

fn locate() -> Option<PathBuf> {
    if let Some(custom) = crate::core::binary_overrides::get(BIN) {
        return Some(custom);
    }
    managed_dir().and_then(|d| github::find_file(&d, &bin_name(BIN)))
}

fn models_of(bin: &Path) -> Vec<String> {
    let dir = bin.parent().map(|p| p.join("models")).unwrap_or_default();
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    if p.extension().map(|x| x == "param").unwrap_or(false) {
                        p.file_stem().map(|s| s.to_string_lossy().to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

pub fn status() -> UpscaleStatus {
    match locate() {
        Some(p) => UpscaleStatus {
            installed: true,
            models: models_of(&p),
            path: Some(p.to_string_lossy().to_string()),
        },
        None => UpscaleStatus {
            installed: false,
            path: None,
            models: vec![],
        },
    }
}

fn asset_pick(name: &str) -> bool {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "ubuntu"
    };
    name.starts_with("realesrgan-ncnn-vulkan") && name.contains(os) && name.ends_with(".zip")
}

pub async fn install(progress: super::ProgressFn) -> anyhow::Result<String> {
    let dir = managed_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    let client = github::client()?;
    let asset = github::asset(&client, REPO, None, asset_pick).await?;
    // Release de 2022: a API não tem digest para esses assets.
    let data = github::download(&client, &asset, true, &progress, BIN).await?;
    let staging = dir.with_extension("new");
    let _ = std::fs::remove_dir_all(&staging);
    let (s2, n2) = (staging.clone(), asset.name.clone());
    tokio::task::spawn_blocking(move || github::unpack(&data, &n2, &s2))
        .await
        .map_err(|e| anyhow!("Spawn blocking failed: {}", e))??;
    let Some(exe) = github::find_file(&staging, &bin_name(BIN)) else {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(anyhow!("o pacote nao contem o {}", BIN));
    };
    github::make_executable(&exe);
    github::swap_dir(&staging, &dir)?;
    github::strip_quarantine(&dir).await;
    locate()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| anyhow!("binario sumiu apos instalar"))
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpscaleOptions {
    pub inputs: Vec<String>,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_scale")]
    pub scale: u32,
    /// "png" | "jpg" | "webp"
    #[serde(default = "default_fmt")]
    pub format: String,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub tile_size: u32,
}

fn default_model() -> String {
    "realesrgan-x4plus".into()
}
fn default_scale() -> u32 {
    4
}
fn default_fmt() -> String {
    "png".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct UpscaleResult {
    pub outputs: Vec<String>,
    pub failed: Vec<String>,
}

pub async fn run(
    opts: UpscaleOptions,
    progress: super::ProgressFn,
) -> anyhow::Result<UpscaleResult> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let bin = locate().ok_or_else(|| anyhow!("Real-ESRGAN nao esta instalado"))?;
    let models_dir = bin.parent().map(|p| p.join("models")).unwrap_or_default();
    let mut outputs = Vec::new();
    let mut failed = Vec::new();
    let total = opts.inputs.len() as u64;
    for (i, input) in opts.inputs.iter().enumerate() {
        let inp = Path::new(input);
        let out_dir = if opts.output_dir.trim().is_empty() {
            inp.parent().map(|p| p.to_path_buf()).unwrap_or_default()
        } else {
            PathBuf::from(opts.output_dir.trim())
        };
        std::fs::create_dir_all(&out_dir)?;
        let stem = inp
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "imagem".into());
        let out = out_dir.join(format!("{}_{}x.{}", stem, opts.scale, opts.format));
        let id = format!("upscale:{}", input);
        super::report(&progress, &id, "started", i as u64, Some(total), None);
        let mut cmd = crate::core::process::command(&bin);
        cmd.arg("-i")
            .arg(inp)
            .arg("-o")
            .arg(&out)
            .arg("-m")
            .arg(&models_dir);
        cmd.args([
            "-n",
            &opts.model,
            "-s",
            &opts.scale.to_string(),
            "-f",
            &opts.format,
        ]);
        if opts.tile_size > 0 {
            cmd.args(["-t", &opts.tile_size.to_string()]);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn()?;
        let stderr = child.stderr.take();
        let p2 = progress.clone();
        let id2 = id.clone();
        let task = tokio::spawn(async move {
            let mut tail = String::new();
            if let Some(e) = stderr {
                let mut lines = BufReader::new(e).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let l = line.trim().to_string();
                    if let Some(pct) = l
                        .strip_suffix('%')
                        .and_then(|s| s.trim().parse::<f64>().ok())
                    {
                        super::report(&p2, &id2, "progress", pct as u64, Some(100), None);
                    } else if !l.is_empty() {
                        tail = l;
                    }
                }
            }
            tail
        });
        let st = child.wait().await?;
        let tail = task.await.unwrap_or_default();
        if st.success() && out.exists() {
            outputs.push(out.to_string_lossy().to_string());
        } else {
            tracing::warn!("[upscale] {} falhou: {}", input, tail);
            failed.push(input.clone());
        }
    }
    Ok(UpscaleResult { outputs, failed })
}
