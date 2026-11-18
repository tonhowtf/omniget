use std::io::Read;
use std::path::PathBuf;

use anyhow::{anyhow, Context};
use serde::Serialize;

use super::ProgressFn;

#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub family: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
    pub license: &'static str,
    pub source: &'static str,
    pub recommended: bool,
}

const CATALOG: &[ModelSpec] = &[
    ModelSpec {
        id: "u2netp",
        name: "U²-Net light",
        family: "rembg",
        url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx",
        sha256: "309c8469258dda742793dce0ebea8e6dd393174f89934733ecc8b14c76f4ddd8",
        bytes: 4_574_861,
        license: "Apache-2.0",
        source: "https://github.com/xuebinqin/U-2-Net",
        recommended: true,
    },
    ModelSpec {
        id: "silueta",
        name: "Silueta",
        family: "rembg",
        url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/silueta.onnx",
        sha256: "75da6c8d2f8096ec743d071951be73b4a8bc7b3e51d9a6625d63644f90ffeedb",
        bytes: 44_173_029,
        license: "Apache-2.0",
        source: "https://github.com/xuebinqin/U-2-Net/issues/295",
        recommended: false,
    },
    ModelSpec {
        id: "u2net",
        name: "U²-Net",
        family: "rembg",
        url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2net.onnx",
        sha256: "8d10d2f3bb75ae3b6d527c77944fc5e7dcd94b29809d47a739a7a728a912b491",
        bytes: 175_997_641,
        license: "Apache-2.0",
        source: "https://github.com/xuebinqin/U-2-Net",
        recommended: false,
    },
    ModelSpec {
        id: "isnet-general-use",
        name: "IS-Net",
        family: "rembg",
        url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx",
        sha256: "60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a",
        bytes: 178_648_008,
        license: "Apache-2.0",
        source: "https://github.com/xuebinqin/DIS",
        recommended: false,
    },
];

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    CATALOG.iter().find(|m| m.id == id)
}

pub fn models_dir() -> Option<PathBuf> {
    super::tools_dir().map(|d| d.join("models").join("onnx"))
}

pub fn model_path(id: &str) -> Option<PathBuf> {
    models_dir().map(|d| d.join(format!("{id}.onnx")))
}

pub fn is_downloaded(id: &str) -> bool {
    match (find(id), model_path(id)) {
        (Some(spec), Some(p)) => std::fs::metadata(&p)
            .map(|m| m.is_file() && m.len() == spec.bytes)
            .unwrap_or(false),
        _ => false,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub id: String,
    pub name: String,
    pub family: String,
    pub bytes: u64,
    pub license: String,
    pub source: String,
    pub recommended: bool,
    pub downloaded: bool,
    pub path: Option<String>,
}

fn sha256_of(path: &std::path::Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("opening {} to check its sha256", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub async fn ensure_model(id: &str, progress: &ProgressFn) -> anyhow::Result<PathBuf> {
    let spec = find(id).ok_or_else(|| anyhow!("unknown model: {id}"))?;
    let dest = model_path(id).ok_or_else(|| anyhow!("could not find the app data folder"))?;
    if is_downloaded(id) {
        return Ok(dest);
    }
    let dir = dest
        .parent()
        .ok_or_else(|| anyhow!("model path has no folder"))?
        .to_path_buf();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let tmp = dir.join(format!(".{id}.onnx.download"));
    let client = super::client()?;
    let pid = format!("onnx-model:{id}");
    super::download_to(&client, spec.url, &tmp, progress, &pid).await?;

    let expected = spec.sha256.to_string();
    let tmp2 = tmp.clone();
    let dest2 = dest.clone();
    let p = progress.clone();
    let pid2 = pid.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        super::report(&p, &pid2, "verify", 0, None, Some("checking sha256".into()));
        let got = sha256_of(&tmp2)?;
        if got != expected {
            let _ = std::fs::remove_file(&tmp2);
            return Err(anyhow!(
                "the downloaded model does not match: expected sha256 {expected}, got {got}"
            ));
        }
        if dest2.exists() {
            let _ = std::fs::remove_file(&dest2);
        }
        std::fs::rename(&tmp2, &dest2).with_context(|| format!("moving to {}", dest2.display()))?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("verification task failed: {e}"))??;

    super::report(progress, &pid, "done", 1, Some(1), None);
    Ok(dest)
}

fn intra_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| (n.get().saturating_sub(1)).max(1))
        .unwrap_or(1)
}

pub fn session_for(id: &str) -> anyhow::Result<ort::session::Session> {
    let path = model_path(id).ok_or_else(|| anyhow!("could not find the app data folder"))?;
    if !path.is_file() {
        return Err(anyhow!(
            "model {id} has not been downloaded yet (expected at {})",
            path.display()
        ));
    }
    crate::core::onnxrt::init()?;
    session_from_file(&path)
}

pub fn session_from_file(path: &std::path::Path) -> anyhow::Result<ort::session::Session> {
    use ort::session::builder::GraphOptimizationLevel;
    let mut builder = ort::session::Session::builder()
        .map_err(|e| anyhow!("could not create the ONNX session builder: {e}"))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .unwrap_or_else(|e| e.recover())
        .with_intra_threads(intra_threads())
        .unwrap_or_else(|e| e.recover());
    builder
        .commit_from_file(path)
        .map_err(|e| anyhow!("could not load model {}: {}", path.display(), e))
}
