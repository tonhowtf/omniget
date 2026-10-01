use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use serde::Serialize;

use crate::core::tools::ProgressFn;

pub const DEFAULT_MODEL: &str = "whisper-base";

pub const DIR_ENV: &str = "OMNIGET_ASR_MODEL_DIR";

const HOSTS: &[&str] = &["https://huggingface.co", "https://hf-mirror.com"];

#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    pub local: &'static str,
    pub remote: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub repo: &'static str,
    pub rev: &'static str,
    pub heads: usize,
    pub head_dim: usize,
    pub files: [ModelFile; 4],
}

pub const ENCODER: &str = "encoder.onnx";
pub const DECODER: &str = "decoder_merged.onnx";
pub const VOCAB: &str = "vocab.json";
pub const GEN_CONFIG: &str = "generation_config.json";

const VOCAB_FILE: ModelFile = ModelFile {
    local: VOCAB,
    remote: "vocab.json",
    bytes: 1_036_584,
    sha256: "50d6a919f0a0601d56a04eb583c780d18553aa388254ba3158eb6a00f13e2c1a",
};

pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "whisper-tiny",
        label: "Whisper tiny (fast, ~45 MB)",
        repo: "onnx-community/whisper-tiny",
        rev: "ff4177021cc41f7db950912b73ea4fdf7d01d8e7",
        heads: 6,
        head_dim: 64,
        files: [
            ModelFile {
                local: ENCODER,
                remote: "onnx/encoder_model_quantized.onnx",
                bytes: 10_124_990,
                sha256: "2af4a414ca47aa30f61246017e5fe82b0a8d229281d1255ba666a2a7f6b84d19",
            },
            ModelFile {
                local: DECODER,
                remote: "onnx/decoder_model_merged_quantized.onnx",
                bytes: 30_719_241,
                sha256: "25e807a962b6349356d0ea5d0dfe530b7e5bf0e2a484aeca0359d03143faddd3",
            },
            VOCAB_FILE,
            ModelFile {
                local: GEN_CONFIG,
                remote: "generation_config.json",
                bytes: 3_772,
                sha256: "f5c67e5a4f7102f8cb4d058bc95da276bbc19eeec997267c3bb0f25ef68facd1",
            },
        ],
    },
    ModelSpec {
        id: "whisper-base",
        label: "Whisper base (balanced, ~80 MB)",
        repo: "onnx-community/whisper-base",
        rev: "1846881b6b3a3024392c1eea3ad983695bc23925",
        heads: 8,
        head_dim: 64,
        files: [
            ModelFile {
                local: ENCODER,
                remote: "onnx/encoder_model_quantized.onnx",
                bytes: 23_201_314,
                sha256: "5862993336bf33acd23736071aae2b32261d3b1b2f37780194460d4ef974dd46",
            },
            ModelFile {
                local: DECODER,
                remote: "onnx/decoder_model_merged_quantized.onnx",
                bytes: 53_693_315,
                sha256: "fa3ef9902734ce5ae6f9ef2bdb2ba9a6c4b5785b09f4f420ce036573dc9d090b",
            },
            VOCAB_FILE,
            ModelFile {
                local: GEN_CONFIG,
                remote: "generation_config.json",
                bytes: 3_832,
                sha256: "61070cf8de25b1e9256e8e102ded49d8d24a8369ed36ef84fdf21549e68125a0",
            },
        ],
    },
    ModelSpec {
        id: "whisper-small",
        label: "Whisper small (accurate, ~250 MB)",
        repo: "onnx-community/whisper-small",
        rev: "36050c46d777d46dc4b5f43f6d90574fc38f8732",
        heads: 12,
        head_dim: 64,
        files: [
            ModelFile {
                local: ENCODER,
                remote: "onnx/encoder_model_quantized.onnx",
                bytes: 92_326_160,
                sha256: "a43a83f3c5361cd591cfa7c36f14b43cf7cb22f47a415cc14a8d557be800fa92",
            },
            ModelFile {
                local: DECODER,
                remote: "onnx/decoder_model_merged_quantized.onnx",
                bytes: 156_750_845,
                sha256: "ec07c3cbb64172c39791e26ee870a65ac22b458c36722bfe2776b3dbf741e0c9",
            },
            VOCAB_FILE,
            ModelFile {
                local: GEN_CONFIG,
                remote: "generation_config.json",
                bytes: 3_893,
                sha256: "f538b28220c6a6d6f1af1458d4141cacb4ef4963df3de98a19490440c412ddf0",
            },
        ],
    },
];

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    let id = id.trim();
    MODELS
        .iter()
        .find(|m| m.id == id || m.id.trim_start_matches("whisper-") == id)
}

pub fn pick(id: Option<&str>) -> anyhow::Result<&'static ModelSpec> {
    match id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => find(id).ok_or_else(|| {
            anyhow!(
                "unknown Whisper model `{id}` (available: {})",
                MODELS.iter().map(|m| m.id).collect::<Vec<_>>().join(", ")
            )
        }),
        None => find(DEFAULT_MODEL).ok_or_else(|| anyhow!("default model missing")),
    }
}

pub fn root_dir() -> Option<PathBuf> {
    match std::env::var(DIR_ENV).ok().filter(|s| !s.trim().is_empty()) {
        Some(s) => Some(PathBuf::from(s.trim())),
        None => crate::core::tools::onnx::models_dir(),
    }
}

pub fn model_dir(m: &ModelSpec) -> Option<PathBuf> {
    root_dir().map(|d| d.join(m.id))
}

pub fn file_path(m: &ModelSpec, local: &str) -> Option<PathBuf> {
    model_dir(m).map(|d| d.join(local))
}

fn has_file(m: &ModelSpec, f: &ModelFile) -> bool {
    file_path(m, f.local)
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|md| md.is_file() && md.len() == f.bytes)
        .unwrap_or(false)
}

pub fn is_downloaded(m: &ModelSpec) -> bool {
    m.files.iter().all(|f| has_file(m, f))
}

pub fn total_bytes(m: &ModelSpec) -> u64 {
    m.files.iter().map(|f| f.bytes).sum()
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub downloaded: bool,
    pub default: bool,
}

pub fn list() -> Vec<ModelStatus> {
    MODELS
        .iter()
        .map(|m| ModelStatus {
            id: m.id.to_string(),
            label: m.label.to_string(),
            bytes: total_bytes(m),
            downloaded: is_downloaded(m),
            default: m.id == DEFAULT_MODEL,
        })
        .collect()
}

fn sha256_of(path: &Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
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

pub fn urls(m: &ModelSpec, f: &ModelFile) -> Vec<String> {
    HOSTS
        .iter()
        .map(|h| format!("{}/{}/resolve/{}/{}", h, m.repo, m.rev, f.remote))
        .collect()
}

async fn fetch(
    client: &reqwest::Client,
    m: &ModelSpec,
    f: &ModelFile,
    progress: &ProgressFn,
) -> anyhow::Result<()> {
    if has_file(m, f) {
        return Ok(());
    }
    let dest = file_path(m, f.local).ok_or_else(|| anyhow!("no app data directory"))?;
    let dir = dest
        .parent()
        .ok_or_else(|| anyhow!("model path without a parent"))?
        .to_path_buf();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = dir.join(format!(".{}.download", f.local));
    let id = format!("{}:{}", m.id, f.local);

    let mut last_err = None;
    for url in urls(m, f) {
        match crate::core::tools::download_to(client, &url, &tmp, progress, &id).await {
            Ok(_) => {
                let expected = f.sha256;
                let tmp2 = tmp.clone();
                let got = tokio::task::spawn_blocking(move || sha256_of(&tmp2))
                    .await
                    .map_err(|e| anyhow!("hash task died: {e}"))??;
                if got == expected {
                    if dest.exists() {
                        let _ = std::fs::remove_file(&dest);
                    }
                    std::fs::rename(&tmp, &dest)
                        .with_context(|| format!("moving to {}", dest.display()))?;
                    return Ok(());
                }
                let _ = std::fs::remove_file(&tmp);
                last_err = Some(anyhow!(
                    "{url}: checksum mismatch (expected {expected}, got {got})"
                ));
            }
            Err(e) => {
                tracing::warn!("[asr] download of {} failed: {}", url, e);
                last_err = Some(anyhow!("{url}: {e}"));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("no URL for {}", f.local)))
}

pub async fn install(m: &ModelSpec, progress: &ProgressFn) -> anyhow::Result<PathBuf> {
    if !crate::core::onnxrt::is_installed() {
        crate::core::onnxrt::install_runtime(None, progress).await?;
    }
    let client = crate::core::tools::client()?;
    for f in &m.files {
        fetch(&client, m, f, progress).await?;
    }
    model_dir(m).ok_or_else(|| anyhow!("no app data directory"))
}
